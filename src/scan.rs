use crate::model::{make_default_mappings, normalize_headers, SourceKind, SourceTable};
use anyhow::{Context, Result};
use calamine::{open_workbook_auto, Reader};
use csv::{ByteRecord, ReaderBuilder};
use encoding_rs::GBK;
use std::fs::File;
use std::io::{BufReader, Read};
use std::path::{Path, PathBuf};
use std::sync::mpsc::Sender;
use walkdir::WalkDir;

#[derive(Debug)]
pub enum ScanEvent {
    Progress { done: usize, total: usize, name: String },
    Finished { tables: Vec<SourceTable>, warnings: Vec<String> },
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

pub fn spawn_scan(paths: Vec<PathBuf>, tx: Sender<ScanEvent>) {
    std::thread::spawn(move || {
        let total = paths.len();
        let mut tables = Vec::new();
        let mut warnings = Vec::new();

        for (index, path) in paths.into_iter().enumerate() {
            let name = path
                .file_name()
                .map(|v| v.to_string_lossy().into_owned())
                .unwrap_or_else(|| path.display().to_string());
            let _ = tx.send(ScanEvent::Progress {
                done: index,
                total,
                name,
            });
            match scan_file(&path) {
                Ok(mut found) => tables.append(&mut found),
                Err(error) => warnings.push(format!("{}：{error:#}", path.display())),
            }
        }

        if tables.is_empty() && !warnings.is_empty() {
            let _ = tx.send(ScanEvent::Failed(warnings.join("\n")));
        } else {
            let _ = tx.send(ScanEvent::Finished { tables, warnings });
        }
    });
}

pub fn scan_file(path: &Path) -> Result<Vec<SourceTable>> {
    match path
        .extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| ext.to_ascii_lowercase())
        .as_deref()
    {
        Some("csv" | "tsv") => scan_csv(path).map(|table| vec![table]),
        Some("xlsx" | "xlsm" | "xls" | "xlsb" | "ods") => scan_workbook(path),
        _ => Ok(Vec::new()),
    }
}

fn scan_csv(path: &Path) -> Result<SourceTable> {
    let delimiter = detect_delimiter(path)?;
    let mut reader = ReaderBuilder::new()
        .has_headers(false)
        .flexible(true)
        .delimiter(delimiter)
        .from_path(path)
        .with_context(|| "无法打开 CSV")?;
    let mut records = reader.byte_records();
    let header_record = records
        .next()
        .transpose()
        .with_context(|| "无法读取 CSV 表头")?
        .unwrap_or_default();
    let headers = normalize_headers(header_record.iter().map(decode_csv_field));
    let mut rows = 0_u64;
    for record in records {
        record.with_context(|| "CSV 数据格式错误")?;
        rows += 1;
    }
    let mappings = make_default_mappings(&headers);
    Ok(SourceTable {
        path: path.to_owned(),
        sheet_name: "CSV".to_owned(),
        kind: SourceKind::Csv { delimiter },
        headers,
        estimated_rows: rows,
        enabled: true,
        mappings,
    })
}

fn scan_workbook(path: &Path) -> Result<Vec<SourceTable>> {
    let mut workbook = open_workbook_auto(path).with_context(|| "无法打开工作簿")?;
    let sheet_names = workbook.sheet_names().to_vec();
    let mut result = Vec::new();

    for sheet_name in sheet_names {
        let range = workbook
            .worksheet_range(&sheet_name)
            .with_context(|| format!("无法读取工作表 {sheet_name}"))?;
        let Some(first_row) = range.rows().next() else {
            continue;
        };
        let headers = normalize_headers(first_row.iter().map(ToString::to_string));
        let mappings = make_default_mappings(&headers);
        result.push(SourceTable {
            path: path.to_owned(),
            sheet_name,
            kind: SourceKind::Workbook,
            headers,
            estimated_rows: range.height().saturating_sub(1) as u64,
            enabled: true,
            mappings,
        });
    }
    Ok(result)
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
    let mut sample = vec![0_u8; 16 * 1024];
    let read = file.read(&mut sample)?;
    sample.truncate(read);
    let first_line = sample
        .split(|byte| *byte == b'\n' || *byte == b'\r')
        .find(|line| !line.is_empty())
        .unwrap_or_default();
    let candidates = [b',', b'\t', b';', b'|'];
    Ok(candidates
        .into_iter()
        .max_by_key(|delimiter| first_line.iter().filter(|byte| **byte == *delimiter).count())
        .unwrap_or(b','))
}

pub fn decode_csv_field(bytes: &[u8]) -> String {
    let bytes = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(bytes);
    if let Ok(value) = std::str::from_utf8(bytes) {
        return value.to_owned();
    }
    let (value, _, _) = GBK.decode(bytes);
    value.into_owned()
}

pub fn for_each_csv_row<F>(path: &Path, delimiter: u8, mut callback: F) -> Result<()>
where
    F: FnMut(Vec<String>) -> Result<()>,
{
    let mut reader = ReaderBuilder::new()
        .has_headers(false)
        .flexible(true)
        .delimiter(delimiter)
        .from_path(path)?;
    let mut record = ByteRecord::new();
    let _ = reader.read_byte_record(&mut record)?;
    while reader.read_byte_record(&mut record)? {
        callback(record.iter().map(decode_csv_field).collect())?;
    }
    Ok(())
}
