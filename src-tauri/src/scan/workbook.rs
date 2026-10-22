use super::{
    format::{is_markup_workbook, is_xlsx_path, workbook_format, WorkbookFormat},
    header::{
        combine_header_rows_with_width, recommend_header, source_table_from_headers,
        AUTO_HEADER_ROWS,
    },
    markup::{scan_markup_auto_headers, scan_markup_sheet},
};
use crate::file_cache::{Cache, FileVersion};
use crate::model::SourceTable;
use anyhow::{bail, Context, Result};
use calamine::{open_workbook, open_workbook_auto, Data, Ods, Reader, Sheets, Xls, Xlsb, Xlsx};
use std::path::{Path, PathBuf};
use std::{
    fs::File,
    io::BufReader,
    sync::{Mutex, OnceLock},
    thread,
    time::Duration,
};
pub(crate) struct XlsxWorkbook {
    inner: Xlsx<BufReader<File>>,
    path: PathBuf,
    version: Option<FileVersion>,
}
impl std::ops::Deref for XlsxWorkbook {
    type Target = Xlsx<BufReader<File>>;
    fn deref(&self) -> &Self::Target {
        &self.inner
    }
}
impl std::ops::DerefMut for XlsxWorkbook {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.inner
    }
}
impl XlsxWorkbook {
    fn unchanged(&self) -> Result<()> {
        if self.version != FileVersion::read(&self.path) {
            bail!("读取期间源文件已修改，请重新读取");
        }
        Ok(())
    }
}
pub(crate) fn open_xlsx_workbook(path: &Path) -> Result<XlsxWorkbook> {
    let mut last_error = None;
    for attempt in 0..3 {
        let version = FileVersion::read(path);
        match open_workbook::<Xlsx<_>, _>(path) {
            Ok(inner) => {
                let workbook = XlsxWorkbook {
                    inner,
                    path: path.to_path_buf(),
                    version,
                };
                workbook.unchanged()?;
                return Ok(workbook);
            }
            Err(error) => {
                let incomplete = error
                    .to_string()
                    .to_ascii_lowercase()
                    .contains("unexpected end");
                last_error = Some(error);
                if !incomplete || attempt == 2 {
                    break;
                }
                thread::sleep(Duration::from_millis(250 * (attempt + 1)));
            }
        }
    }
    let _ = last_error.context("无法打开工作簿")?;
    bail!("XLSX 文件不完整、已损坏或格式与扩展名不符；请等待文件下载或复制完成后重试，仍然失败时请用 Excel 另存为新的 XLSX 文件")
}

pub(crate) fn open_detected_workbook(path: &Path) -> Result<Sheets<BufReader<File>>> {
    match workbook_format(path)? {
        WorkbookFormat::Xlsx => Ok(Sheets::Xlsx(open_xlsx_workbook(path)?.inner)),
        WorkbookFormat::Xls => open_workbook::<Xls<_>, _>(path)
            .map(Sheets::Xls)
            .context("无法打开工作簿"),
        WorkbookFormat::Xlsb => open_workbook::<Xlsb<_>, _>(path)
            .map(Sheets::Xlsb)
            .context("无法打开工作簿"),
        WorkbookFormat::Ods => open_workbook::<Ods<_>, _>(path)
            .map(Sheets::Ods)
            .context("无法打开工作簿"),
        WorkbookFormat::Html | WorkbookFormat::SpreadsheetXml => {
            bail!("标记表格需要使用兼容读取器")
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct XlsxStreamBounds {
    pub(crate) actual_start: Option<(u32, u32)>,
    pub(crate) actual_end: Option<(u32, u32)>,
    pub(crate) actual_columns: Option<(u32, u32)>,
}

type OriginKey = (PathBuf, String, FileVersion);
static XLSX_COLUMN_ORIGINS: OnceLock<Mutex<Cache<OriginKey, u32>>> = OnceLock::new();
pub(super) fn clear_origin(path: &Path) {
    if let Some(cache) = XLSX_COLUMN_ORIGINS.get() {
        if let Ok(mut cache) = cache.lock() {
            cache.retain(|(file, _, _)| file != path);
        }
    }
}
fn origin_key(workbook: &XlsxWorkbook, sheet: &str) -> Option<OriginKey> {
    Some((
        workbook.path.clone(),
        sheet.to_owned(),
        workbook.version.clone()?,
    ))
}
fn xlsx_origin(workbook: &XlsxWorkbook, sheet: &str) -> Option<u32> {
    let key = origin_key(workbook, sheet)?;
    XLSX_COLUMN_ORIGINS.get()?.lock().ok()?.get(&key)
}
pub(crate) fn for_each_xlsx_source_row<F>(
    workbook: &mut XlsxWorkbook,
    sheet: &str,
    offset: usize,
    limit: Option<usize>,
    callback: F,
) -> Result<XlsxStreamBounds>
where
    F: FnMut(Vec<Data>) -> Result<()>,
{
    workbook.unchanged()?;
    let origin = xlsx_origin(workbook, sheet);
    let bounds = read_xlsx_rows(
        &mut workbook.inner,
        sheet,
        offset,
        limit,
        origin,
        true,
        callback,
    )?;
    workbook.unchanged()?;
    if let (Some(key), Some((column, _))) = (origin_key(workbook, sheet), bounds.actual_columns) {
        if let Ok(mut cache) = XLSX_COLUMN_ORIGINS
            .get_or_init(|| Mutex::new(Cache::new(1024, 1024 * 1024)))
            .lock()
        {
            cache.insert(key, column, 512);
        }
    }
    Ok(bounds)
}

/// Read an XLSX worksheet one row at a time without materializing the full Range.
/// `row_offset` is relative to the first used row, matching `Range::rows().skip(...)`.
#[cfg(test)]
pub(crate) fn for_each_xlsx_row<F>(
    workbook: &mut Xlsx<BufReader<File>>,
    sheet_name: &str,
    row_offset: usize,
    max_rows: Option<usize>,
    callback: F,
) -> Result<XlsxStreamBounds>
where
    F: FnMut(Vec<Data>) -> Result<()>,
{
    read_xlsx_rows(
        workbook, sheet_name, row_offset, max_rows, None, true, callback,
    )
}

/// Preview only: reuse the scanned column origin and stop after the requested rows.
pub(crate) fn for_each_xlsx_preview_row<F>(
    workbook: &mut XlsxWorkbook,
    _path: &Path,
    sheet_name: &str,
    row_offset: usize,
    max_rows: usize,
    callback: F,
) -> Result<()>
where
    F: FnMut(Vec<Data>) -> Result<()>,
{
    workbook.unchanged()?;
    let origin = xlsx_origin(workbook, sheet_name);
    read_xlsx_rows(
        &mut workbook.inner,
        sheet_name,
        row_offset,
        Some(max_rows),
        origin,
        false,
        callback,
    )?;
    workbook.unchanged()
}

fn read_xlsx_rows<F>(
    workbook: &mut Xlsx<BufReader<File>>,
    sheet_name: &str,
    row_offset: usize,
    max_rows: Option<usize>,
    mut base_column: Option<u32>,
    full_bounds: bool,
    mut callback: F,
) -> Result<XlsxStreamBounds>
where
    F: FnMut(Vec<Data>) -> Result<()>,
{
    // The first non-empty cell can be a centered title. Find the leftmost actual
    // column before emitting rows so later header cells cannot overwrite it.
    if base_column.is_none() {
        let mut bounds_reader = workbook
            .worksheet_cells_reader(sheet_name)
            .with_context(|| format!("无法读取工作表 {sheet_name}"))?;
        while let Some(cell) = bounds_reader.next_cell()? {
            let value = Data::from(cell.get_value().clone());
            if !is_effectively_empty(&value) {
                let column = cell.get_position().1;
                base_column = Some(base_column.map_or(column, |base| base.min(column)));
                if base_column == Some(0) {
                    break;
                }
            }
        }
    }
    let mut reader = workbook
        .worksheet_cells_reader(sheet_name)
        .with_context(|| format!("无法读取工作表 {sheet_name}"))?;
    let max_rows = max_rows.unwrap_or(usize::MAX);
    if max_rows == 0 {
        return Ok(XlsxStreamBounds {
            actual_start: None,
            actual_end: None,
            actual_columns: None,
        });
    }

    let mut actual_start = None;
    let mut actual_end = None;
    let mut actual_columns: Option<(u32, u32)> = None;
    let mut target_row = None;
    let mut next_row = 0_u32;
    let mut current = Vec::<Data>::new();
    let mut emitted = 0_usize;
    let mut collecting = true;

    'cells: while let Some(cell) = reader.next_cell()? {
        let (row, column) = cell.get_position();
        let value = Data::from(cell.get_value().clone());
        // Excel 经常会把“设置过格式但没有内容”的单元格写进 XML，
        // 甚至把它们放到 XFD 或很靠后的行。它们不能参与真实范围、表头
        // 或数据行的计算，否则会产生上万列和数百万空行。
        if is_effectively_empty(&value) {
            continue;
        }
        if actual_start.is_none() {
            actual_start = Some((row, column));
            target_row = Some(row.saturating_add(row_offset as u32));
            next_row = target_row.unwrap_or(row);
        }
        actual_end = Some((row, column));
        actual_columns = Some(match actual_columns {
            Some((first, last)) => (first.min(column), last.max(column)),
            None => (column, column),
        });
        let target_row = target_row.expect("XLSX stream target row must be initialized");
        if row < target_row || !collecting {
            continue;
        }

        while next_row < row {
            callback(std::mem::take(&mut current))?;
            emitted += 1;
            if emitted >= max_rows {
                collecting = false;
                current.clear();
                if !full_bounds {
                    break 'cells;
                }
                // 导入扫描仍需读到最后，计算真实范围；预览到达上限即结束。
                break;
            }
            next_row = next_row.saturating_add(1);
        }
        if !collecting {
            continue;
        }

        let relative_column = column.saturating_sub(base_column.unwrap_or(column)) as usize;
        if current.len() <= relative_column {
            current.resize(relative_column + 1, Data::Empty);
        }
        current[relative_column] = value;
    }

    if collecting && actual_start.is_some() && !current.is_empty() {
        callback(current)?;
    }

    Ok(XlsxStreamBounds {
        actual_start,
        actual_end,
        actual_columns,
    })
}

pub(super) fn is_effectively_empty(value: &Data) -> bool {
    match value {
        Data::Empty => true,
        Data::String(text) | Data::DateTimeIso(text) | Data::DurationIso(text) => {
            text.trim().is_empty()
        }
        _ => false,
    }
}

pub(super) fn scan_workbook_auto_headers(path: &Path) -> Result<Vec<SourceTable>> {
    match open_workbook_auto(path) {
        Ok(workbook) => scan_range_workbook_auto_headers(path, workbook),
        Err(primary_error) => match workbook_format(path)? {
            WorkbookFormat::Html | WorkbookFormat::SpreadsheetXml => scan_markup_auto_headers(path),
            _ => scan_range_workbook_auto_headers(path, open_detected_workbook(path)?)
                .with_context(|| format!("常规读取失败：{primary_error}")),
        },
    }
}

pub(super) fn scan_range_workbook_auto_headers(
    path: &Path,
    mut workbook: Sheets<BufReader<File>>,
) -> Result<Vec<SourceTable>> {
    let mut result = Vec::new();
    for sheet_name in workbook.sheet_names().to_vec() {
        let range = workbook
            .worksheet_range(&sheet_name)
            .with_context(|| format!("无法读取工作表 {sheet_name}"))?;
        if range.is_empty() {
            continue;
        }
        let suggested = recommend_header(
            range
                .rows()
                .take(AUTO_HEADER_ROWS)
                .map(|row| row.iter().map(Data::to_string).collect::<Vec<_>>()),
        ) + 1;
        result.push(table_from_range(
            path, sheet_name, &range, suggested, 1, suggested,
        ));
    }
    Ok(result)
}

pub(super) fn scan_xlsx_auto_headers(path: &Path) -> Result<Vec<SourceTable>> {
    match open_xlsx_workbook(path) {
        Ok(workbook) => scan_xlsx_reader_auto_headers(path, workbook),
        Err(primary_error) => match workbook_format(path)? {
            WorkbookFormat::Html | WorkbookFormat::SpreadsheetXml => scan_markup_auto_headers(path),
            WorkbookFormat::Xlsx => Err(primary_error),
            _ => scan_range_workbook_auto_headers(path, open_detected_workbook(path)?)
                .with_context(|| format!("常规 XLSX 读取失败：{primary_error:#}")),
        },
    }
}

pub(super) fn scan_xlsx_reader_auto_headers(
    path: &Path,
    mut workbook: XlsxWorkbook,
) -> Result<Vec<SourceTable>> {
    let mut result = Vec::new();
    for sheet_name in workbook.sheet_names().to_vec() {
        let mut sample_rows = Vec::new();
        let bounds = for_each_xlsx_source_row(
            &mut workbook,
            &sheet_name,
            0,
            Some(AUTO_HEADER_ROWS),
            |row| {
                sample_rows.push(row.iter().map(Data::to_string).collect::<Vec<_>>());
                Ok(())
            },
        )?;
        if sample_rows.is_empty() {
            continue;
        }
        let suggested = recommend_header(sample_rows.clone()) + 1;
        let header_parts = sample_rows
            .iter()
            .skip(suggested - 1)
            .take(1)
            .cloned()
            .collect::<Vec<_>>();
        let headers = combine_header_rows_with_width(&header_parts, xlsx_actual_width(&bounds));
        let estimated_rows = xlsx_estimated_rows(&bounds, suggested);
        result.push(source_table_from_headers(
            path,
            sheet_name,
            suggested,
            1,
            suggested,
            headers,
            estimated_rows,
        ));
    }
    Ok(result)
}

pub(super) fn scan_workbook_sheet(
    path: &Path,
    sheet_name: &str,
    header_row: usize,
    header_rows: usize,
    suggested_header_row: usize,
) -> Result<SourceTable> {
    if is_xlsx_path(path) {
        return match open_xlsx_workbook(path) {
            Ok(mut workbook) => scan_xlsx_sheet(
                &mut workbook,
                path,
                sheet_name,
                header_row,
                header_rows,
                suggested_header_row,
            ),
            Err(primary_error) => scan_workbook_sheet_fallback(
                path,
                sheet_name,
                header_row,
                header_rows,
                suggested_header_row,
            )
            .with_context(|| format!("常规 XLSX 读取失败：{primary_error:#}")),
        };
    }
    match open_workbook_auto(path) {
        Ok(workbook) => scan_range_workbook_sheet(
            path,
            sheet_name,
            header_row,
            header_rows,
            suggested_header_row,
            workbook,
        ),
        Err(primary_error) => scan_workbook_sheet_fallback(
            path,
            sheet_name,
            header_row,
            header_rows,
            suggested_header_row,
        )
        .with_context(|| format!("常规读取失败：{primary_error}")),
    }
}

pub(super) fn scan_range_workbook_sheet(
    path: &Path,
    sheet_name: &str,
    header_row: usize,
    header_rows: usize,
    suggested_header_row: usize,
    mut workbook: Sheets<BufReader<File>>,
) -> Result<SourceTable> {
    let range = workbook
        .worksheet_range(sheet_name)
        .with_context(|| format!("无法读取工作表 {sheet_name}"))?;
    let required = header_row.max(1) + header_rows.clamp(1, 3) - 1;
    if range.height() < required {
        anyhow::bail!("工作表不足 {required} 行，无法设为表头");
    }
    Ok(table_from_range(
        path,
        sheet_name.to_owned(),
        &range,
        header_row,
        header_rows,
        suggested_header_row,
    ))
}

pub(super) fn scan_workbook_sheet_fallback(
    path: &Path,
    sheet_name: &str,
    header_row: usize,
    header_rows: usize,
    suggested_header_row: usize,
) -> Result<SourceTable> {
    if is_markup_workbook(path) {
        return scan_markup_sheet(
            path,
            sheet_name,
            header_row,
            header_rows,
            suggested_header_row,
        );
    }
    scan_range_workbook_sheet(
        path,
        sheet_name,
        header_row,
        header_rows,
        suggested_header_row,
        open_detected_workbook(path)?,
    )
}

pub(super) fn scan_xlsx_sheet(
    workbook: &mut XlsxWorkbook,
    path: &Path,
    sheet_name: &str,
    header_row: usize,
    header_rows: usize,
    suggested_header_row: usize,
) -> Result<SourceTable> {
    let header_row = header_row.max(1);
    let header_rows = header_rows.clamp(1, 3);
    let mut parts = Vec::new();
    let bounds = for_each_xlsx_source_row(
        workbook,
        sheet_name,
        header_row - 1,
        Some(header_rows),
        |row| {
            parts.push(row.iter().map(Data::to_string).collect::<Vec<_>>());
            Ok(())
        },
    )?;
    if parts.len() < header_rows {
        anyhow::bail!(
            "工作表不足 {} 行，无法读取多行表头",
            header_row + header_rows - 1
        );
    }
    let headers = combine_header_rows_with_width(&parts, xlsx_actual_width(&bounds));
    let estimated_rows = xlsx_estimated_rows(&bounds, header_row + header_rows - 1);
    Ok(source_table_from_headers(
        path,
        sheet_name.to_owned(),
        header_row,
        header_rows,
        suggested_header_row,
        headers,
        estimated_rows,
    ))
}

pub(super) fn table_from_range(
    path: &Path,
    sheet_name: String,
    range: &calamine::Range<Data>,
    header_row: usize,
    header_rows: usize,
    suggested_header_row: usize,
) -> SourceTable {
    let header_row = header_row.max(1);
    let header_rows = header_rows.clamp(1, 3);
    let (actual_height, actual_width) = range_content_bounds(range);
    let parts = range
        .rows()
        .skip(header_row - 1)
        .take(header_rows)
        .map(|row| {
            row.iter()
                .take(actual_width)
                .map(Data::to_string)
                .collect::<Vec<_>>()
        })
        .collect::<Vec<_>>();
    let headers = combine_header_rows_with_width(&parts, actual_width);
    source_table_from_headers(
        path,
        sheet_name,
        header_row,
        header_rows,
        suggested_header_row,
        headers,
        actual_height.saturating_sub(header_row + header_rows - 1) as u64,
    )
}

pub(super) fn xlsx_estimated_rows(bounds: &XlsxStreamBounds, rows_before_data: usize) -> u64 {
    let Some((start_row, _)) = bounds.actual_start else {
        return 0;
    };
    let Some((end_row, _)) = bounds.actual_end else {
        return 0;
    };
    let height = end_row.saturating_sub(start_row).saturating_add(1) as usize;
    height.saturating_sub(rows_before_data) as u64
}

pub(super) fn xlsx_actual_width(bounds: &XlsxStreamBounds) -> usize {
    bounds
        .actual_columns
        .map(|(first, last)| last.saturating_sub(first).saturating_add(1) as usize)
        .unwrap_or(0)
}

pub(super) fn range_content_bounds(range: &calamine::Range<Data>) -> (usize, usize) {
    let mut last_row = None;
    let mut last_column = None;
    for (row_index, row) in range.rows().enumerate() {
        for (column_index, value) in row.iter().enumerate() {
            if !is_effectively_empty(value) {
                last_row = Some(row_index);
                last_column =
                    Some(last_column.map_or(column_index, |last: usize| last.max(column_index)));
            }
        }
    }
    (
        last_row.map_or(0, |row| row + 1),
        last_column.map_or(0, |column| column + 1),
    )
}

/// One active workbook bounds shared-string memory while reloading adjacent Sheets.
#[derive(Default)]
pub(super) struct SheetReader {
    workbook: Option<(PathBuf, XlsxWorkbook)>,
}
impl SheetReader {
    pub(super) fn scan(
        &mut self,
        source: &SourceTable,
        header_row: usize,
        header_rows: usize,
    ) -> Result<SourceTable> {
        if is_xlsx_path(&source.path) {
            if !self
                .workbook
                .as_ref()
                .is_some_and(|(path, _)| path == &source.path)
            {
                self.workbook = None;
                if let Ok(workbook) = open_xlsx_workbook(&source.path) {
                    self.workbook = Some((source.path.clone(), workbook));
                }
            }
            if let Some((_, workbook)) = &mut self.workbook {
                return scan_xlsx_sheet(
                    workbook,
                    &source.path,
                    &source.sheet_name,
                    header_row,
                    header_rows,
                    source.suggested_header_row,
                );
            }
        }
        scan_workbook_sheet(
            &source.path,
            &source.sheet_name,
            header_row,
            header_rows,
            source.suggested_header_row,
        )
    }
}
