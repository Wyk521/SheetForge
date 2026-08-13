use crate::model::{make_default_mappings, normalize_headers, SourceKind, SourceTable};
use anyhow::{Context, Result};
use calamine::{open_workbook_auto, Data, Reader};
use csv::{ByteRecord, ReaderBuilder};
use encoding_rs::GBK;
use std::collections::{HashSet, VecDeque};
use std::fs::File;
use std::io::{BufReader, Read};
use std::path::{Path, PathBuf};
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex};
use walkdir::WalkDir;

const SAMPLE_BYTES: usize = 256 * 1024;
const AUTO_HEADER_ROWS: usize = 20;
const EXACT_COUNT_LIMIT: u64 = 64 * 1024 * 1024;

#[derive(Debug)]
pub enum ScanEvent {
    Progress {
        done: usize,
        total: usize,
        name: String,
    },
    Finished {
        tables: Vec<SourceTable>,
        warnings: Vec<String>,
    },
    TableReloaded {
        index: usize,
        table: SourceTable,
    },
    TablesReloaded {
        tables: Vec<(usize, SourceTable)>,
    },
    TableReloadFailed {
        index: usize,
        message: String,
    },
    Failed(String),
}

pub fn supported_file(path: &Path) -> bool {
    if path
        .file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.starts_with("~$"))
    {
        return false;
    }
    matches!(
        path.extension()
            .and_then(|ext| ext.to_str())
            .map(|ext| ext.to_ascii_lowercase())
            .as_deref(),
        Some("csv" | "tsv" | "xlsx" | "xlsm" | "xls" | "xlsb" | "ods")
    )
}

pub fn collect_folder(folder: &Path) -> Vec<PathBuf> {
    let mut paths: Vec<PathBuf> = WalkDir::new(folder)
        .follow_links(false)
        .into_iter()
        .filter_map(Result::ok)
        .filter(|entry| entry.file_type().is_file() && supported_file(entry.path()))
        .map(|entry| entry.into_path())
        .collect();
    paths.sort_by_key(|path| path.to_string_lossy().to_lowercase());
    paths
}

pub fn spawn_scan(mut paths: Vec<PathBuf>, tx: Sender<ScanEvent>) {
    paths.sort();
    paths.dedup();
    std::thread::spawn(move || {
        let total = paths.len();
        let workers = std::thread::available_parallelism()
            .map(|value| value.get())
            .unwrap_or(2)
            .clamp(1, 4)
            .min(total.max(1));
        let queue = Arc::new(Mutex::new(VecDeque::from(paths)));
        let (result_tx, result_rx) = std::sync::mpsc::channel();
        for _ in 0..workers {
            let queue = Arc::clone(&queue);
            let result_tx = result_tx.clone();
            std::thread::spawn(move || loop {
                let path = queue.lock().ok().and_then(|mut queue| queue.pop_front());
                let Some(path) = path else { break };
                if result_tx.send((path.clone(), scan_file(&path))).is_err() {
                    break;
                }
            });
        }
        drop(result_tx);
        let mut tables = Vec::new();
        let mut warnings = Vec::new();
        for (index, (path, result)) in result_rx.into_iter().enumerate() {
            let name = path
                .file_name()
                .map(|value| value.to_string_lossy().into_owned())
                .unwrap_or_else(|| path.display().to_string());
            let _ = tx.send(ScanEvent::Progress {
                done: index + 1,
                total,
                name,
            });
            match result {
                Ok((mut found, file_warnings)) => {
                    tables.append(&mut found);
                    warnings.extend(file_warnings);
                }
                Err(error) => warnings.push(format!("{}：{error:#}", path.display())),
            }
        }
        tables.sort_by_key(|table| {
            (
                table.path.to_string_lossy().to_lowercase(),
                table.sheet_name.to_lowercase(),
            )
        });
        warnings.sort();
        if tables.is_empty() && !warnings.is_empty() {
            let _ = tx.send(ScanEvent::Failed(warnings.join("\n")));
        } else {
            let _ = tx.send(ScanEvent::Finished { tables, warnings });
        }
    });
}

/// 扫描单个文件，返回数据表和该文件的警告（如可疑编码）。
pub fn scan_file(path: &Path) -> Result<(Vec<SourceTable>, Vec<String>)> {
    match path
        .extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| ext.to_ascii_lowercase())
        .as_deref()
    {
        Some("csv" | "tsv") => {
            let table = scan_csv_auto(path)?;
            let delimiter = match table.kind {
                SourceKind::Csv { delimiter } => delimiter,
                SourceKind::Workbook => return Ok((vec![table], Vec::new())),
            };
            let mut warnings = Vec::new();
            if csv_encoding_suspicious(path, delimiter) {
                warnings.push(format!(
                    "{}：文件编码可能不是 UTF-8 或 GBK，部分字符无法识别",
                    path.display()
                ));
            }
            Ok((vec![table], warnings))
        }
        Some("xlsx" | "xlsm" | "xls" | "xlsb" | "ods") => {
            Ok((scan_workbook_auto_headers(path)?, Vec::new()))
        }
        _ => Ok((Vec::new(), Vec::new())),
    }
}

/// 抽样检查 CSV 前若干条记录的解码质量，出现替换字符（U+FFFD）视为可疑编码。
fn csv_encoding_suspicious(path: &Path, delimiter: u8) -> bool {
    let Ok(mut reader) = ReaderBuilder::new()
        .has_headers(false)
        .flexible(true)
        .delimiter(delimiter)
        .from_path(path)
    else {
        return false;
    };
    let mut record = ByteRecord::new();
    let mut checked = 0;
    while checked < 50 && reader.read_byte_record(&mut record).unwrap_or(false) {
        if record
            .iter()
            .any(|field| decode_csv_field_with_quality(field).1)
        {
            return true;
        }
        checked += 1;
    }
    false
}

pub fn spawn_table_reload(
    index: usize,
    source: SourceTable,
    header_row: usize,
    header_rows: usize,
    tx: Sender<ScanEvent>,
) {
    std::thread::spawn(move || {
        let result = match source.kind {
            SourceKind::Csv { delimiter } => scan_csv(
                &source.path,
                delimiter,
                header_row,
                header_rows,
                source.suggested_header_row,
            ),
            SourceKind::Workbook => scan_workbook_sheet(
                &source.path,
                &source.sheet_name,
                header_row,
                header_rows,
                source.suggested_header_row,
            ),
        };
        match result {
            Ok(mut table) => {
                table.enabled = source.enabled;
                preserve_mappings(&source, &mut table);
                let _ = tx.send(ScanEvent::TableReloaded { index, table });
            }
            Err(error) => {
                let _ = tx.send(ScanEvent::TableReloadFailed {
                    index,
                    message: format!("{}：{error:#}", source.display_name()),
                });
            }
        }
    });
}

pub fn spawn_group_reload(
    sources: Vec<(usize, SourceTable)>,
    header_row: usize,
    header_rows: usize,
    tx: Sender<ScanEvent>,
) {
    std::thread::spawn(move || {
        let mut tables = Vec::with_capacity(sources.len());
        for (index, source) in sources {
            let result = match source.kind {
                SourceKind::Csv { delimiter } => scan_csv(
                    &source.path,
                    delimiter,
                    header_row,
                    header_rows,
                    source.suggested_header_row,
                ),
                SourceKind::Workbook => scan_workbook_sheet(
                    &source.path,
                    &source.sheet_name,
                    header_row,
                    header_rows,
                    source.suggested_header_row,
                ),
            };
            match result {
                Ok(mut table) => {
                    table.enabled = source.enabled;
                    preserve_mappings(&source, &mut table);
                    tables.push((index, table));
                }
                Err(error) => {
                    let _ = tx.send(ScanEvent::TableReloadFailed {
                        index,
                        message: format!("{}：{error:#}", source.display_name()),
                    });
                    return;
                }
            }
        }
        let _ = tx.send(ScanEvent::TablesReloaded { tables });
    });
}

fn preserve_mappings(old: &SourceTable, new: &mut SourceTable) {
    for mapping in &mut new.mappings {
        if let Some(previous) = old
            .mappings
            .iter()
            .find(|previous| previous.source_name == mapping.source_name)
        {
            *mapping = previous.clone();
            mapping.source_index = new
                .headers
                .iter()
                .position(|header| header == &mapping.source_name)
                .unwrap_or(mapping.source_index);
        }
    }
}

fn scan_csv_auto(path: &Path) -> Result<SourceTable> {
    let delimiter = detect_delimiter(path)?;
    let sample = sample_csv_records(path, delimiter, AUTO_HEADER_ROWS + 4)?;
    let suggested = recommend_header(
        sample
            .iter()
            .map(|row| row.iter().map(decode_csv_field).collect::<Vec<_>>()),
    ) + 1;
    scan_csv(path, delimiter, suggested, 1, suggested)
}

fn scan_csv(
    path: &Path,
    delimiter: u8,
    header_row: usize,
    header_rows: usize,
    suggested_header_row: usize,
) -> Result<SourceTable> {
    let header_row = header_row.max(1);
    let header_rows = header_rows.clamp(1, 3);
    let mut reader = ReaderBuilder::new()
        .has_headers(false)
        .flexible(true)
        .delimiter(delimiter)
        .from_path(path)
        .with_context(|| "无法打开 CSV")?;
    let mut record = ByteRecord::new();
    for row_index in 1..header_row {
        if !reader.read_byte_record(&mut record)? {
            anyhow::bail!("CSV 不足 {row_index} 行，无法设为表头");
        }
    }
    let mut header_parts = Vec::new();
    for offset in 0..header_rows {
        if !reader.read_byte_record(&mut record)? {
            anyhow::bail!("CSV 不足 {} 行，无法读取多行表头", header_row + offset);
        }
        header_parts.push(record.iter().map(decode_csv_field).collect::<Vec<_>>());
    }
    let headers = combine_header_rows(&header_parts);
    let estimated_rows = count_remaining_csv_rows(path, delimiter, header_row + header_rows - 1)?;
    Ok(SourceTable {
        path: path.to_owned(),
        sheet_name: "CSV".to_owned(),
        kind: SourceKind::Csv { delimiter },
        header_row,
        header_rows,
        suggested_header_row: suggested_header_row.max(1),
        mappings: make_default_mappings(&headers),
        headers,
        estimated_rows,
        enabled: true,
    })
}

fn scan_workbook_auto_headers(path: &Path) -> Result<Vec<SourceTable>> {
    let mut workbook = open_workbook_auto(path).with_context(|| "无法打开工作簿")?;
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

fn scan_workbook_sheet(
    path: &Path,
    sheet_name: &str,
    header_row: usize,
    header_rows: usize,
    suggested_header_row: usize,
) -> Result<SourceTable> {
    let mut workbook = open_workbook_auto(path).with_context(|| "无法打开工作簿")?;
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

fn table_from_range(
    path: &Path,
    sheet_name: String,
    range: &calamine::Range<Data>,
    header_row: usize,
    header_rows: usize,
    suggested_header_row: usize,
) -> SourceTable {
    let header_row = header_row.max(1);
    let header_rows = header_rows.clamp(1, 3);
    let parts = range
        .rows()
        .skip(header_row - 1)
        .take(header_rows)
        .map(|row| row.iter().map(Data::to_string).collect::<Vec<_>>())
        .collect::<Vec<_>>();
    let headers = combine_header_rows(&parts);
    SourceTable {
        path: path.to_owned(),
        sheet_name,
        kind: SourceKind::Workbook,
        header_row,
        header_rows,
        suggested_header_row: suggested_header_row.max(1),
        mappings: make_default_mappings(&headers),
        headers,
        estimated_rows: range.height().saturating_sub(header_row + header_rows - 1) as u64,
        enabled: true,
    }
}

fn combine_header_rows(rows: &[Vec<String>]) -> Vec<String> {
    let width = rows.iter().map(Vec::len).max().unwrap_or(0);
    normalize_headers((0..width).map(|column| {
        let mut seen = Vec::new();
        for row in rows {
            let value = row
                .get(column)
                .map(|value| value.trim())
                .unwrap_or_default();
            if !value.is_empty() && !seen.iter().any(|existing: &String| existing == value) {
                seen.push(value.to_owned());
            }
        }
        seen.join(" / ")
    }))
}

pub fn recommend_header<I>(rows: I) -> usize
where
    I: IntoIterator<Item = Vec<String>>,
{
    let rows = rows.into_iter().take(AUTO_HEADER_ROWS).collect::<Vec<_>>();
    if rows.is_empty() {
        return 0;
    }
    let max_width = rows.iter().map(Vec::len).max().unwrap_or(1).max(1);
    rows.iter()
        .enumerate()
        .map(|(index, row)| {
            let non_empty = row.iter().filter(|cell| !cell.trim().is_empty()).count();
            let unique = row
                .iter()
                .filter(|cell| !cell.trim().is_empty())
                .map(|cell| cell.trim().to_lowercase())
                .collect::<HashSet<_>>()
                .len();
            let text = row
                .iter()
                .filter(|cell| cell.chars().any(char::is_alphabetic))
                .count();
            let next_same = rows
                .get(index + 1)
                .is_some_and(|next| next.len() == row.len() && row.len() > 1);
            let score = non_empty as i64 * 20 / max_width as i64
                + unique as i64 * 4
                + text as i64 * 3
                + i64::from(next_same) * 8
                - index as i64;
            (score, std::cmp::Reverse(index))
        })
        .max()
        .map(|(_, std::cmp::Reverse(index))| index)
        .unwrap_or(0)
}

fn sample_csv_records(path: &Path, delimiter: u8, limit: usize) -> Result<Vec<ByteRecord>> {
    ReaderBuilder::new()
        .has_headers(false)
        .flexible(true)
        .delimiter(delimiter)
        .from_path(path)?
        .byte_records()
        .take(limit)
        .collect::<csv::Result<Vec<_>>>()
        .map_err(Into::into)
}

fn count_remaining_csv_rows(path: &Path, delimiter: u8, skipped_rows: usize) -> Result<u64> {
    let file_size = std::fs::metadata(path)?.len();
    let mut reader = ReaderBuilder::new()
        .has_headers(false)
        .flexible(true)
        .delimiter(delimiter)
        .from_path(path)?;
    let mut record = ByteRecord::new();
    for _ in 0..skipped_rows {
        if !reader.read_byte_record(&mut record)? {
            return Ok(0);
        }
    }
    if file_size <= EXACT_COUNT_LIMIT {
        let mut rows = 0_u64;
        while reader.read_byte_record(&mut record)? {
            rows += 1;
        }
        return Ok(rows);
    }
    let start = reader.position().byte();
    let mut sampled = 0_u64;
    while sampled < 10_000 && reader.read_byte_record(&mut record)? {
        sampled += 1;
    }
    let consumed = reader.position().byte().saturating_sub(start).max(1);
    Ok(sampled.saturating_mul(file_size.saturating_sub(start)) / consumed)
}

pub fn detect_delimiter(path: &Path) -> Result<u8> {
    if path
        .extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| ext.eq_ignore_ascii_case("tsv"))
    {
        return Ok(b'\t');
    }
    let mut file = BufReader::new(File::open(path)?);
    let mut sample = vec![0_u8; SAMPLE_BYTES];
    let read = file.read(&mut sample)?;
    sample.truncate(read);
    let mut best = (i64::MIN, b',');
    for &delimiter in b",\t;|" {
        let widths = ReaderBuilder::new()
            .has_headers(false)
            .flexible(true)
            .delimiter(delimiter)
            .from_reader(sample.as_slice())
            .byte_records()
            .take(30)
            .filter_map(Result::ok)
            .map(|record| record.len())
            .filter(|width| *width > 0)
            .collect::<Vec<_>>();
        if widths.is_empty() {
            continue;
        }
        let mode_width = widths
            .iter()
            .copied()
            .max_by_key(|width| {
                widths
                    .iter()
                    .filter(|candidate| *candidate == width)
                    .count()
            })
            .unwrap_or(1);
        let consistent = widths.iter().filter(|width| **width == mode_width).count();
        // A wrong delimiter can split a quoted multiline field into several
        // physical one-column records.  Consistency alone would then make that
        // delimiter look better than the real one.  Prefer a stable table with
        // multiple columns; genuinely single-column files still fall back to
        // the first candidate (comma).
        let single_column_penalty = if mode_width == 1 { 10_000 } else { 0 };
        let score = consistent as i64 * 100 + mode_width as i64 * 50
            - widths.len() as i64
            - single_column_penalty;
        if score > best.0 {
            best = (score, delimiter);
        }
    }
    Ok(best.1)
}

pub fn decode_csv_field(bytes: &[u8]) -> String {
    decode_csv_field_with_quality(bytes).0
}

/// 解码 CSV 字段，同时返回解码质量：为 true 表示出现了替换字符（U+FFFD），
/// 通常是文件既非 UTF-8 也非 GBK 编码的信号。
pub fn decode_csv_field_with_quality(bytes: &[u8]) -> (String, bool) {
    let bytes = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(bytes);
    if let Ok(value) = std::str::from_utf8(bytes) {
        return (value.to_owned(), false);
    }
    let (value, _, had_errors) = GBK.decode(bytes);
    (value.into_owned(), had_errors)
}

pub fn for_each_csv_row<F>(
    path: &Path,
    delimiter: u8,
    header_row: usize,
    header_rows: usize,
    mut callback: F,
) -> Result<()>
where
    F: FnMut(Vec<String>) -> Result<()>,
{
    let mut reader = ReaderBuilder::new()
        .has_headers(false)
        .flexible(true)
        .delimiter(delimiter)
        .from_path(path)?;
    let mut record = ByteRecord::new();
    for _ in 0..header_row.max(1) + header_rows.clamp(1, 3) - 1 {
        if !reader.read_byte_record(&mut record)? {
            return Ok(());
        }
    }
    while reader.read_byte_record(&mut record)? {
        callback(record.iter().map(decode_csv_field).collect())?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn header_recommendation_skips_report_title() {
        let rows = vec![
            vec!["2026 年销售报表".into()],
            vec!["姓名".into(), "金额".into(), "城市".into()],
            vec!["张三".into(), "12".into(), "北京".into()],
        ];
        assert_eq!(recommend_header(rows), 1);
    }

    #[test]
    fn multi_row_headers_are_combined_and_deduplicated() {
        let headers = combine_header_rows(&[
            vec!["客户".into(), "金额".into()],
            vec!["姓名".into(), "金额".into()],
        ]);
        assert_eq!(headers, vec!["客户 / 姓名", "金额"]);
    }

    #[test]
    fn delimiter_detection_respects_quoted_commas() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("带 空格.csv");
        let mut file = File::create(&path).unwrap();
        writeln!(file, "姓名;说明;金额").unwrap();
        writeln!(file, "张三;\"北京,朝阳\";12").unwrap();
        writeln!(file, "李四;\"上海,浦东\";15").unwrap();
        assert_eq!(detect_delimiter(&path).unwrap(), b';');
        let (tables, warnings) = scan_file(&path).unwrap();
        assert_eq!(tables[0].headers, vec!["姓名", "说明", "金额"]);
        assert!(warnings.is_empty());
    }

    #[test]
    fn csv_reader_handles_multiline_quoted_fields() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("quoted.csv");
        std::fs::write(&path, "id,note\n1,\"line one\nline two\"\n").unwrap();
        assert_eq!(detect_delimiter(&path).unwrap(), b',');
        let (tables, _) = scan_file(&path).unwrap();
        let table = tables.into_iter().next().unwrap();
        let mut values = Vec::new();
        if let SourceKind::Csv { delimiter } = table.kind {
            for_each_csv_row(
                &path,
                delimiter,
                table.header_row,
                table.header_rows,
                |row| {
                    values.push(row);
                    Ok(())
                },
            )
            .unwrap();
        }
        assert_eq!(values.len(), 1);
        assert_eq!(values[0][1], "line one\nline two");
    }

    #[test]
    fn non_utf8_non_gbk_csv_gets_encoding_warning() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("latin1.csv");
        // "café" 的 é 以 Latin-1 字节 0xE9 写入，既非 UTF-8 也无法按 GBK 完整解码。
        std::fs::write(&path, b"name,value\ncaf\xE9\n").unwrap();
        let (_, warnings) = scan_file(&path).unwrap();
        assert!(
            warnings
                .iter()
                .any(|warning| warning.contains("编码可能不是 UTF-8 或 GBK")),
            "warnings: {warnings:?}"
        );
    }
}
