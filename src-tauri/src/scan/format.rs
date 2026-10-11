use super::csv::decode_csv_field;
use anyhow::{bail, Context, Result};
use encoding_rs::{UTF_16BE, UTF_16LE};
use std::{fs::File, io::Read, path::Path};
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum WorkbookFormat {
    Xlsx,
    Xls,
    Xlsb,
    Ods,
    Html,
    SpreadsheetXml,
}

pub(crate) fn is_xlsx_path(path: &Path) -> bool {
    matches!(
        path.extension()
            .and_then(|ext| ext.to_str())
            .map(|ext| ext.to_ascii_lowercase())
            .as_deref(),
        Some("xlsx" | "xlsm" | "xlam")
    )
}

pub(crate) fn is_markup_workbook(path: &Path) -> bool {
    workbook_format(path).is_ok_and(|format| {
        matches!(
            format,
            WorkbookFormat::Html | WorkbookFormat::SpreadsheetXml
        )
    })
}

pub(super) fn workbook_format(path: &Path) -> Result<WorkbookFormat> {
    let mut file = File::open(path).with_context(|| format!("无法读取 {}", path.display()))?;
    let mut prefix = vec![0_u8; 16 * 1024];
    let read = file.read(&mut prefix)?;
    prefix.truncate(read);
    if prefix.is_empty() {
        bail!("文件为空");
    }

    if prefix.starts_with(&[0xD0, 0xCF, 0x11, 0xE0, 0xA1, 0xB1, 0x1A, 0xE1]) {
        return Ok(WorkbookFormat::Xls);
    }

    let extension = path
        .extension()
        .and_then(|ext| ext.to_str())
        .map(str::to_ascii_lowercase);
    if prefix.starts_with(b"PK\x03\x04") {
        return Ok(match extension.as_deref() {
            Some("xlsb") => WorkbookFormat::Xlsb,
            Some("ods") => WorkbookFormat::Ods,
            _ => WorkbookFormat::Xlsx,
        });
    }

    let marker = decode_markup_document(&prefix).to_ascii_lowercase();
    let marker = marker.trim_start_matches(['\u{feff}', '\0', ' ', '\t', '\r', '\n']);
    if marker.starts_with("<!doctype html")
        || marker.starts_with("<html")
        || marker.starts_with("<table")
        || (marker.contains("<html") && marker.contains("<table"))
    {
        return Ok(WorkbookFormat::Html);
    }
    if marker.starts_with("<?xml")
        && (marker.contains("urn:schemas-microsoft-com:office:spreadsheet")
            || marker.contains("<workbook"))
    {
        return Ok(WorkbookFormat::SpreadsheetXml);
    }

    Ok(match extension.as_deref() {
        Some("xlsx" | "xlsm" | "xlam") => WorkbookFormat::Xlsx,
        Some("xls" | "xla") => WorkbookFormat::Xls,
        Some("xlsb") => WorkbookFormat::Xlsb,
        Some("ods") => WorkbookFormat::Ods,
        _ => bail!("无法识别表格格式"),
    })
}

pub(super) fn decode_markup_document(bytes: &[u8]) -> String {
    if let Some(bytes) = bytes.strip_prefix(&[0xFF, 0xFE]) {
        return UTF_16LE.decode(bytes).0.into_owned();
    }
    if let Some(bytes) = bytes.strip_prefix(&[0xFE, 0xFF]) {
        return UTF_16BE.decode(bytes).0.into_owned();
    }
    decode_csv_field(bytes)
}
