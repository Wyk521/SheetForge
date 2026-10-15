use super::header::{combine_header_rows, recommend_header, AUTO_HEADER_ROWS};
use crate::model::make_default_mappings;
use crate::model::{SourceKind, SourceTable};
use anyhow::{Context, Result};
use csv::{ByteRecord, ReaderBuilder};
use encoding_rs::{Encoding, GBK, UTF_8};
use std::path::Path;
use std::{
    fs::File,
    io::{BufReader, Read},
};
const SAMPLE_BYTES: usize = 256 * 1024;
const EXACT_COUNT_LIMIT: u64 = 64 * 1024 * 1024;
pub(crate) fn csv_encoding(path: &Path) -> Result<&'static Encoding> {
    let mut file = File::open(path)?;
    let mut decoder = UTF_8.new_decoder_without_bom_handling();
    let mut input = [0; 8192];
    // 足够容纳整个输入块及跨块字符，即使无效字节被展开为替换字符。
    let mut output = [0; 32768];
    loop {
        let read = file.read(&mut input)?;
        let (_, _, _, malformed) = decoder.decode_to_utf8(&input[..read], &mut output, read == 0);
        if malformed {
            return Ok(GBK);
        }
        if read == 0 {
            return Ok(UTF_8);
        }
    }
}

pub(super) fn csv_encoding_suspicious(path: &Path, delimiter: u8) -> Result<bool> {
    let encoding = csv_encoding(path)?;
    let mut reader = ReaderBuilder::new()
        .has_headers(false)
        .flexible(true)
        .delimiter(delimiter)
        .from_path(path)?;
    let mut record = ByteRecord::new();
    let mut checked = 0;
    while checked < 50 && reader.read_byte_record(&mut record)? {
        if record
            .iter()
            .any(|field| decode_csv_field_with_quality(field, encoding).1)
        {
            return Ok(true);
        }
        checked += 1;
    }
    Ok(false)
}

pub(super) fn scan_csv_auto(path: &Path) -> Result<SourceTable> {
    let encoding = csv_encoding(path)?;
    let delimiter = detect_delimiter(path)?;
    let sample = sample_csv_records(path, delimiter, AUTO_HEADER_ROWS + 4)?;
    let suggested = recommend_header(sample.iter().map(|row| {
        row.iter()
            .map(|field| decode_csv_field(field, encoding))
            .collect::<Vec<_>>()
    })) + 1;
    scan_csv(path, delimiter, suggested, 1, suggested)
}

pub(super) fn scan_csv(
    path: &Path,
    delimiter: u8,
    header_row: usize,
    header_rows: usize,
    suggested_header_row: usize,
) -> Result<SourceTable> {
    let encoding = csv_encoding(path)?;
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
        header_parts.push(
            record
                .iter()
                .map(|field| decode_csv_field(field, encoding))
                .collect::<Vec<_>>(),
        );
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

pub(super) fn sample_csv_records(
    path: &Path,
    delimiter: u8,
    limit: usize,
) -> Result<Vec<ByteRecord>> {
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

pub(super) fn count_remaining_csv_rows(
    path: &Path,
    delimiter: u8,
    skipped_rows: usize,
) -> Result<u64> {
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

pub fn decode_csv_field(bytes: &[u8], encoding: &'static Encoding) -> String {
    decode_csv_field_with_quality(bytes, encoding).0
}

/// 解码 CSV 字段，同时返回解码质量：为 true 表示出现了替换字符（U+FFFD），
/// 通常是文件既非 UTF-8 也非 GBK 编码的信号。
pub fn decode_csv_field_with_quality(bytes: &[u8], encoding: &'static Encoding) -> (String, bool) {
    let bytes = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(bytes);
    let (value, _, had_errors) = encoding.decode(bytes);
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
    let encoding = csv_encoding(path)?;
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
        callback(
            record
                .iter()
                .map(|field| decode_csv_field(field, encoding))
                .collect(),
        )?;
    }
    Ok(())
}
