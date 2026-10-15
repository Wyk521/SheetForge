use super::{
    format::{decode_markup_document, workbook_format, WorkbookFormat},
    header::{
        combine_header_rows_with_width, recommend_header, source_table_from_headers,
        AUTO_HEADER_ROWS,
    },
};
use crate::model::SourceTable;
use anyhow::{bail, Context, Result};
use dom_query::Document;

use quick_xml::{
    events::{BytesStart, Event},
    Reader as XmlReader,
};
use std::fs;
use std::path::Path;
#[derive(Clone, Debug)]
pub(crate) struct MarkupSheet {
    pub(crate) name: String,
    pub(crate) rows: Vec<Vec<String>>,
}

pub(crate) fn read_markup_workbook(path: &Path) -> Result<Vec<MarkupSheet>> {
    let bytes = fs::read(path).with_context(|| format!("无法读取 {}", path.display()))?;
    let content = decode_markup_document(&bytes);
    match workbook_format(path)? {
        WorkbookFormat::Html => read_html_tables(&content),
        WorkbookFormat::SpreadsheetXml => read_spreadsheet_xml(&content),
        _ => bail!("文件不是兼容表格"),
    }
}

pub(super) fn read_html_tables(html: &str) -> Result<Vec<MarkupSheet>> {
    let document = Document::from(html);
    let mut sheets = Vec::new();
    for (table_index, table) in document.select("table").iter().enumerate() {
        let mut rows = Vec::new();
        let mut occupied_until = Vec::new();
        for (row_index, row) in table.select("tr").iter().enumerate() {
            let cells = row.children().filter("td, th");
            if cells.is_empty() {
                continue;
            }
            let mut values = Vec::new();
            for cell in cells.iter() {
                while occupied_until
                    .get(values.len())
                    .is_some_and(|end| *end > row_index)
                {
                    values.push(String::new());
                }
                let column = values.len();
                let value = cell
                    .formatted_text()
                    .replace('\u{a0}', " ")
                    .trim()
                    .to_owned();
                values.push(value);
                let colspan = cell
                    .attr("colspan")
                    .and_then(|value| value.parse::<usize>().ok())
                    .unwrap_or(1)
                    .clamp(1, 16_384);
                values.extend(std::iter::repeat_n(String::new(), colspan - 1));
                let rowspan = cell
                    .attr("rowspan")
                    .map(|value| value.parse::<usize>())
                    .transpose()
                    .context("HTML 单元格 rowspan 必须是有效的行数")?
                    .unwrap_or(1)
                    .max(1);
                occupied_until.resize(occupied_until.len().max(values.len()), 0);
                for end in &mut occupied_until[column..values.len()] {
                    *end = row_index.saturating_add(rowspan);
                }
            }
            rows.push(values);
        }
        if rows
            .iter()
            .any(|row| row.iter().any(|cell| !cell.trim().is_empty()))
        {
            let name = table
                .attr("id")
                .filter(|value| !value.trim().is_empty())
                .map(|value| value.to_string())
                .unwrap_or_else(|| format!("Sheet{}", table_index + 1));
            sheets.push(MarkupSheet { name, rows });
        }
    }
    if sheets.is_empty() {
        bail!("网页表格中没有找到可读取的数据表");
    }
    Ok(sheets)
}

pub(super) fn read_spreadsheet_xml(xml: &str) -> Result<Vec<MarkupSheet>> {
    let mut reader = XmlReader::from_str(xml);
    reader.config_mut().trim_text(false);
    let mut sheets = Vec::new();
    let mut current_sheet: Option<MarkupSheet> = None;
    let mut current_row: Option<Vec<String>> = None;
    let mut current_cell: Option<String> = None;
    let mut merge_across = 0u16;
    let mut in_data = false;

    loop {
        match reader.read_event().context("XML 表格格式不正确")? {
            Event::Start(event) => match event.local_name().as_ref() {
                name if name.eq_ignore_ascii_case(b"Worksheet") => {
                    let name = xml_attribute(&event, b"Name", reader.decoder())
                        .unwrap_or_else(|| format!("Sheet{}", sheets.len() + 1));
                    current_sheet = Some(MarkupSheet {
                        name,
                        rows: Vec::new(),
                    });
                }
                name if name.eq_ignore_ascii_case(b"Row") && current_sheet.is_some() => {
                    current_row = Some(Vec::new());
                }
                name if name.eq_ignore_ascii_case(b"Cell") && current_row.is_some() => {
                    if let Some(index) = xml_attribute(&event, b"Index", reader.decoder())
                        .and_then(|value| value.parse::<usize>().ok())
                    {
                        let row = current_row.as_mut().expect("row checked above");
                        while row.len() + 1 < index {
                            row.push(String::new());
                        }
                    }
                    merge_across = xml_attribute(&event, b"MergeAcross", reader.decoder())
                        .map(|value| value.parse::<u16>())
                        .transpose()
                        .context("XML 单元格 MergeAcross 必须是有效的列数")?
                        .unwrap_or(0);
                    current_cell = Some(String::new());
                }
                name if name.eq_ignore_ascii_case(b"Data") && current_cell.is_some() => {
                    in_data = true;
                }
                _ => {}
            },
            Event::Empty(event)
                if event.local_name().as_ref().eq_ignore_ascii_case(b"Cell")
                    && current_row.is_some() =>
            {
                if let Some(index) = xml_attribute(&event, b"Index", reader.decoder())
                    .and_then(|value| value.parse::<usize>().ok())
                {
                    let row = current_row.as_mut().expect("row checked above");
                    while row.len() + 1 < index {
                        row.push(String::new());
                    }
                }
                let merge_across = xml_attribute(&event, b"MergeAcross", reader.decoder())
                    .map(|value| value.parse::<u16>())
                    .transpose()
                    .context("XML 单元格 MergeAcross 必须是有效的列数")?
                    .unwrap_or(0);
                let row = current_row.as_mut().expect("row checked above");
                row.extend(std::iter::repeat_n(
                    String::new(),
                    usize::from(merge_across) + 1,
                ));
            }
            Event::Text(event) if in_data => {
                let decoded = event.decode().context("无法解码 XML 单元格")?;
                if let Some(cell) = current_cell.as_mut() {
                    cell.push_str(&decoded);
                }
            }
            Event::GeneralRef(event) if in_data => {
                let value = if let Some(value) = event.resolve_char_ref()? {
                    value.to_string()
                } else {
                    let name = event.decode().context("无法解码 XML 实体")?;
                    quick_xml::escape::resolve_predefined_entity(&name)
                        .map(str::to_owned)
                        .unwrap_or_else(|| format!("&{name};"))
                };
                if let Some(cell) = current_cell.as_mut() {
                    cell.push_str(&value);
                }
            }
            Event::CData(event) if in_data => {
                if let Some(cell) = current_cell.as_mut() {
                    cell.push_str(&event.decode().context("无法解码 XML 单元格")?);
                }
            }
            Event::End(event) => match event.local_name().as_ref() {
                name if name.eq_ignore_ascii_case(b"Data") => in_data = false,
                name if name.eq_ignore_ascii_case(b"Cell") => {
                    if let (Some(row), Some(cell)) = (current_row.as_mut(), current_cell.take()) {
                        row.push(cell.trim().to_owned());
                        row.extend(std::iter::repeat_n(
                            String::new(),
                            usize::from(merge_across),
                        ));
                    }
                }
                name if name.eq_ignore_ascii_case(b"Row") => {
                    if let (Some(sheet), Some(row)) = (current_sheet.as_mut(), current_row.take()) {
                        sheet.rows.push(row);
                    }
                }
                name if name.eq_ignore_ascii_case(b"Worksheet") => {
                    if let Some(sheet) = current_sheet.take() {
                        if sheet
                            .rows
                            .iter()
                            .any(|row| row.iter().any(|cell| !cell.trim().is_empty()))
                        {
                            sheets.push(sheet);
                        }
                    }
                }
                _ => {}
            },
            Event::Eof => break,
            _ => {}
        }
    }
    if sheets.is_empty() {
        bail!("XML 表格中没有找到可读取的数据表");
    }
    Ok(sheets)
}

pub(super) fn xml_attribute(
    event: &BytesStart<'_>,
    local_name: &[u8],
    decoder: quick_xml::encoding::Decoder,
) -> Option<String> {
    event.attributes().filter_map(Result::ok).find_map(|attr| {
        attr.key
            .local_name()
            .as_ref()
            .eq_ignore_ascii_case(local_name)
            .then(|| {
                attr.decoded_and_normalized_value(quick_xml::XmlVersion::Implicit1_0, decoder)
                    .ok()
                    .map(|value| value.into_owned())
            })
            .flatten()
    })
}
pub(super) fn scan_markup_auto_headers(path: &Path) -> Result<Vec<SourceTable>> {
    let mut result = Vec::new();
    for sheet in read_markup_workbook(path)? {
        let suggested = recommend_header(sheet.rows.iter().take(AUTO_HEADER_ROWS).cloned()) + 1;
        let header_parts = sheet
            .rows
            .iter()
            .skip(suggested - 1)
            .take(1)
            .cloned()
            .collect::<Vec<_>>();
        let width = sheet.rows.iter().map(Vec::len).max().unwrap_or(0);
        let headers = combine_header_rows_with_width(&header_parts, width);
        result.push(source_table_from_headers(
            path,
            sheet.name,
            suggested,
            1,
            suggested,
            headers,
            sheet.rows.len().saturating_sub(suggested) as u64,
        ));
    }
    Ok(result)
}

pub(super) fn scan_markup_sheet(
    path: &Path,
    sheet_name: &str,
    header_row: usize,
    header_rows: usize,
    suggested_header_row: usize,
) -> Result<SourceTable> {
    let sheets = read_markup_workbook(path)?;
    let sheet = sheets
        .iter()
        .find(|sheet| sheet.name == sheet_name)
        .with_context(|| format!("无法读取工作表 {sheet_name}"))?;
    let header_row = header_row.max(1);
    let header_rows = header_rows.clamp(1, 3);
    let required = header_row + header_rows - 1;
    if sheet.rows.len() < required {
        bail!("工作表不足 {required} 行，无法设为表头");
    }
    let width = sheet.rows.iter().map(Vec::len).max().unwrap_or(0);
    let parts = sheet
        .rows
        .iter()
        .skip(header_row - 1)
        .take(header_rows)
        .cloned()
        .collect::<Vec<_>>();
    let headers = combine_header_rows_with_width(&parts, width);
    Ok(source_table_from_headers(
        path,
        sheet.name.clone(),
        header_row,
        header_rows,
        suggested_header_row,
        headers,
        sheet.rows.len().saturating_sub(required) as u64,
    ))
}
