use crate::merge::{preview_mapped_row, CellValue};
use crate::model::{
    build_output_plan, header_key, validate_mapping_targets, MergeOptions, SourceKind, SourceTable,
};
use crate::scan::{
    csv_encoding, decode_csv_field, for_each_xlsx_preview_row, is_markup_workbook, is_xlsx_path,
    open_detected_workbook, open_xlsx_workbook, read_markup_workbook,
};
use anyhow::{Context, Result};
use calamine::{open_workbook_auto, Data, Reader};
use csv::{ByteRecord, ReaderBuilder};
use serde::Serialize;
use std::collections::{HashMap, HashSet};

#[derive(Clone, Debug, Serialize)]
pub struct PreviewTable {
    pub headers: Vec<String>,
    pub rows: Vec<Vec<String>>,
}

#[derive(Clone, Debug, Serialize)]
pub struct MergedPreviewGroup {
    pub source_index: usize,
    pub source_file: String,
    pub source_sheet: String,
    pub rows: Vec<Vec<String>>,
}

#[derive(Clone, Debug, Serialize)]
pub struct MergedPreview {
    pub headers: Vec<String>,
    pub groups: Vec<MergedPreviewGroup>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize)]
pub enum IssueLevel {
    Info,
    Warning,
    Error,
}

#[derive(Clone, Debug, Serialize)]
pub struct CheckIssue {
    pub level: IssueLevel,
    pub title: String,
    pub detail: String,
}

#[derive(Clone, Hash, PartialEq, Eq)]
struct SampleKey {
    path: std::path::PathBuf,
    version: crate::file_cache::FileVersion,
    sheet: String,
    delimiter: Option<u8>,
    header_row: usize,
    header_rows: usize,
    width: usize,
}
struct Sample {
    limit: usize,
    values: Vec<Vec<CellValue>>,
    display: Vec<Vec<String>>,
}
#[derive(Default)]
struct SampleReader {
    // Keep at most one parsed workbook alive; consecutive Sheets share its string table.
    workbook: Option<(std::path::PathBuf, crate::scan::XlsxWorkbook)>,
}
static SAMPLES: std::sync::OnceLock<
    std::sync::Mutex<crate::file_cache::Cache<SampleKey, std::sync::Arc<Sample>>>,
> = std::sync::OnceLock::new();
pub(crate) fn invalidate_source_preview(path: &std::path::Path) {
    crate::scan::invalidate_cached_source(path);
    if let Some(cache) = SAMPLES.get() {
        if let Ok(mut cache) = cache.lock() {
            cache.retain(|key| key.path != path);
        }
    }
}
fn sample_key(table: &SourceTable) -> Option<SampleKey> {
    Some(SampleKey {
        path: table.path.clone(),
        version: crate::file_cache::FileVersion::read(&table.path)?,
        sheet: table.sheet_name.clone(),
        delimiter: match table.kind {
            SourceKind::Csv { delimiter } => Some(delimiter),
            SourceKind::Workbook => None,
        },
        header_row: table.header_row,
        header_rows: table.header_rows,
        width: table.headers.len(),
    })
}
fn source_sample(
    table: &SourceTable,
    limit: usize,
    reader: &mut SampleReader,
) -> Result<std::sync::Arc<Sample>> {
    if limit == 0 {
        return Ok(std::sync::Arc::new(Sample {
            limit,
            values: vec![],
            display: vec![],
        }));
    }
    let key = sample_key(table);
    let cache = SAMPLES.get_or_init(|| {
        std::sync::Mutex::new(crate::file_cache::Cache::new(128, 64 * 1024 * 1024))
    });
    if let Some(key) = &key {
        if let Some(sample) = cache.lock().ok().and_then(|mut cache| cache.get(key)) {
            if sample.limit >= limit || sample.values.len() < sample.limit {
                return Ok(sample);
            }
        }
    }
    let mut values = Vec::new();
    let mut display = Vec::new();
    match table.kind {
        SourceKind::Csv { delimiter } => {
            let encoding = csv_encoding(&table.path)?;
            let mut csv = ReaderBuilder::new()
                .has_headers(false)
                .flexible(true)
                .delimiter(delimiter)
                .from_path(&table.path)?;
            for record in csv
                .byte_records()
                .skip(table.header_row + table.header_rows - 1)
                .take(limit)
            {
                let row: Vec<String> = record?
                    .iter()
                    .map(|field| decode_csv_field(field, encoding))
                    .collect();
                values.push(row.iter().cloned().map(CellValue::Text).collect());
                display.push(row);
            }
        }
        SourceKind::Workbook => {
            if is_xlsx_path(&table.path) {
                if !reader
                    .workbook
                    .as_ref()
                    .is_some_and(|(path, _)| path == &table.path)
                {
                    reader.workbook = None;
                    if let Ok(workbook) = open_xlsx_workbook(&table.path) {
                        reader.workbook = Some((table.path.clone(), workbook));
                    }
                }
                if let Some((_, workbook)) = &mut reader.workbook {
                    for_each_xlsx_preview_row(
                        workbook,
                        &table.path,
                        &table.sheet_name,
                        table.header_row + table.header_rows - 1,
                        limit,
                        |row| {
                            let mut text = vec![String::new(); table.headers.len()];
                            for (index, value) in row.iter().enumerate().take(text.len()) {
                                text[index] = value.to_string();
                            }
                            values.push(
                                row.into_iter()
                                    .map(CellValue::from_calamine_owned)
                                    .collect(),
                            );
                            display.push(text);
                            Ok(())
                        },
                    )?;
                } else {
                    for row in fallback_workbook_text_rows(table, limit)? {
                        values.push(row.iter().cloned().map(CellValue::from_text).collect());
                        display.push(row);
                    }
                }
            } else {
                match open_workbook_auto(&table.path) {
                    Ok(mut workbook) => {
                        let range = workbook.worksheet_range(&table.sheet_name)?;
                        for row in range
                            .rows()
                            .skip(table.header_row + table.header_rows - 1)
                            .take(limit)
                        {
                            values.push(row.iter().map(CellValue::from_calamine).collect());
                            display.push(row.iter().map(Data::to_string).collect());
                        }
                    }
                    Err(_) => {
                        for row in fallback_workbook_text_rows(table, limit)? {
                            values.push(row.iter().cloned().map(CellValue::from_text).collect());
                            display.push(row);
                        }
                    }
                }
            }
        }
    }
    if key != sample_key(table) {
        anyhow::bail!("读取期间源文件已修改，请重新读取");
    }
    let bytes = 1024
        + values
            .iter()
            .map(|row: &Vec<CellValue>| {
                row.capacity() * std::mem::size_of::<CellValue>()
                    + row
                        .iter()
                        .map(|v| match v {
                            CellValue::Text(s) => s.capacity(),
                            _ => 0,
                        })
                        .sum::<usize>()
            })
            .sum::<usize>()
        + display
            .iter()
            .map(|row: &Vec<String>| {
                row.capacity() * std::mem::size_of::<String>()
                    + row.iter().map(String::capacity).sum::<usize>()
            })
            .sum::<usize>();
    let sample = std::sync::Arc::new(Sample {
        limit,
        values,
        display,
    });
    if limit <= 1000 {
        if let Some(key) = key {
            if let Ok(mut cache) = cache.lock() {
                cache.insert(key, sample.clone(), bytes);
            }
        }
    }
    Ok(sample)
}
pub fn preview_source(table: &SourceTable, limit: usize) -> Result<PreviewTable> {
    preview_source_with_reader(table, limit, &mut SampleReader::default())
}
fn preview_source_with_reader(
    table: &SourceTable,
    limit: usize,
    reader: &mut SampleReader,
) -> Result<PreviewTable> {
    let sample = source_sample(table, limit, reader)?;
    Ok(PreviewTable {
        headers: table.headers.clone(),
        rows: sample.display.iter().take(limit).cloned().collect(),
    })
}
pub fn preview_merged(
    tables: &[SourceTable],
    options: &MergeOptions,
    limit: usize,
) -> Result<MergedPreview> {
    validate_mapping_targets(tables, options)?;
    let plan = build_output_plan(tables, options);
    let mut groups = Vec::new();
    let mut seen = HashSet::new();
    let mut reader = SampleReader::default();
    for (source_index, table) in tables.iter().enumerate().filter(|(_, table)| table.enabled) {
        let sample = source_sample(table, limit, &mut reader)?;
        let mut rows = Vec::with_capacity(sample.values.len().min(limit));
        for source_row in sample.values.iter().take(limit) {
            if let Some(output) = preview_mapped_row(source_row.clone(), table, &plan, options) {
                if !options.deduplicate || seen.insert(output.clone()) {
                    rows.push(output);
                }
            }
        }
        groups.push(MergedPreviewGroup {
            source_index,
            source_file: table
                .path
                .file_name()
                .map(|v| v.to_string_lossy().into_owned())
                .unwrap_or_default(),
            source_sheet: table.sheet_name.clone(),
            rows,
        });
    }
    Ok(MergedPreview {
        headers: plan.headers,
        groups,
    })
}

fn fallback_workbook_text_rows(table: &SourceTable, limit: usize) -> Result<Vec<Vec<String>>> {
    if is_markup_workbook(&table.path) {
        let sheets = read_markup_workbook(&table.path)?;
        let sheet = sheets
            .iter()
            .find(|sheet| sheet.name == table.sheet_name)
            .with_context(|| format!("无法读取工作表 {}", table.sheet_name))?;
        return Ok(sheet
            .rows
            .iter()
            .skip(table.header_row + table.header_rows - 1)
            .take(limit)
            .cloned()
            .collect());
    }

    let mut workbook = open_detected_workbook(&table.path)?;
    let range = workbook.worksheet_range(&table.sheet_name)?;
    Ok(range
        .rows()
        .skip(table.header_row + table.header_rows - 1)
        .take(limit)
        .map(|row| row.iter().map(Data::to_string).collect())
        .collect())
}

pub fn preflight_for_destination(
    tables: &[SourceTable],
    options: &MergeOptions,
    database: bool,
) -> Vec<CheckIssue> {
    let enabled = tables
        .iter()
        .filter(|table| table.enabled)
        .collect::<Vec<_>>();
    let mut issues = Vec::new();
    if let Err(error) = validate_mapping_targets(tables, options) {
        issues.push(issue(
            IssueLevel::Error,
            "同表目标字段重复",
            &error.to_string(),
        ));
    }
    if enabled.is_empty() {
        issues.push(issue(
            IssueLevel::Error,
            "没有数据表",
            "请至少选择一个工作表或 CSV。",
        ));
        return issues;
    }
    let plan = build_output_plan(tables, options);
    if plan.headers.is_empty() {
        issues.push(issue(
            IssueLevel::Error,
            "没有输出字段",
            "请检查映射或合并方式。",
        ));
    }
    let column_limit = if database { 1_600 } else { 16_384 };
    if plan.headers.len() > column_limit {
        issues.push(issue(
            IssueLevel::Error,
            if database {
                "列数超出 PostgreSQL 限制"
            } else {
                "列数超出 XLSX 限制"
            },
            &format!(
                "预计输出 {} 列，目标最多支持 {column_limit} 列。",
                plan.headers.len()
            ),
        ));
    }
    let rows: u64 = enabled.iter().map(|table| table.estimated_rows).sum();
    let bytes: u64 = enabled
        .iter()
        .filter_map(|table| std::fs::metadata(&table.path).ok())
        .map(|metadata| metadata.len())
        .sum();
    if database {
        issues.push(issue(
            IssueLevel::Info,
            "导入规模",
            &format!("预计向目标表导入 {rows} 行、{} 列。", plan.headers.len()),
        ));
    } else {
        let sheets = rows.div_ceil(1_048_575).max(1);
        issues.push(issue(
            IssueLevel::Info,
            "输出规模",
            &format!(
                "预计 {rows} 行、{} 列、约 {sheets} 个结果 Sheet。",
                plan.headers.len()
            ),
        ));
    }
    issues.push(issue(
        IssueLevel::Info,
        "输入文件规模",
        &format!("已选数据源合计约 {:.1} MB。", bytes as f64 / 1_048_576.0),
    ));

    let all_header_sets = enabled
        .iter()
        .map(|table| {
            table
                .headers
                .iter()
                .map(|h| header_key(h))
                .collect::<HashSet<_>>()
        })
        .collect::<Vec<_>>();
    let union = all_header_sets
        .iter()
        .flatten()
        .cloned()
        .collect::<HashSet<_>>();
    for (table, set) in enabled.iter().zip(all_header_sets.iter()) {
        if !table.path.exists() {
            issues.push(issue(
                IssueLevel::Error,
                "源文件不存在",
                &format!("{} 已被移动或删除。", table.path.display()),
            ));
            continue;
        }
        let missing = union.difference(set).count();
        if missing > 0 {
            issues.push(issue(
                IssueLevel::Warning,
                "字段不一致",
                &format!(
                    "{} 与其他来源相比缺少 {missing} 个字段。",
                    table.display_name()
                ),
            ));
        }
        let unnamed = table
            .headers
            .iter()
            .filter(|header| header.starts_with("未命名列"))
            .count();
        if unnamed > 0 {
            issues.push(issue(
                IssueLevel::Warning,
                "存在空表头",
                &format!(
                    "{} 有 {unnamed} 个空表头，已自动命名。",
                    table.display_name()
                ),
            ));
        }
        if let SourceKind::Csv { delimiter } = table.kind {
            match validate_csv(
                &table.path,
                delimiter,
                table.header_row + table.header_rows - 1,
                table.headers.len(),
                10_000,
            ) {
                Ok(check) => {
                    if let Some(line) = check.first_wider_line {
                        issues.push(issue(
                            IssueLevel::Warning,
                            "CSV 数据列多于表头",
                            &format!(
                                "{}：{} 行数据的非空列超出表头（首次出现在第 {line} 行），超出部分不会输出。",
                                table.display_name(),
                                check.wider_rows
                            ),
                        ));
                    }
                }
                Err(error) => issues.push(issue(
                    IssueLevel::Error,
                    "CSV 行格式异常",
                    &format!("{}：{error:#}", table.display_name()),
                )),
            }
        }
    }
    // PostgreSQL 目标表的列统一创建为 TEXT NULL，样本类型和“样本全空”
    // 对数据库导入没有决策价值；老 pg 导入器也不会在正式 COPY 前重复抽样。
    // 跳过这一步后，数据库预检查只做元数据、表头和 CSV 结构校验，避免导入前
    // 再打开并读取每个 XLSX 工作表。
    if !database {
        let mut sampled_types = HashMap::<String, HashSet<&'static str>>::new();
        let mut non_empty_counts = HashMap::<String, usize>::new();
        let mut sample_reader = SampleReader::default();
        for table in &enabled {
            match preview_source_with_reader(table, 100, &mut sample_reader) {
                Ok(preview) => {
                    for (column, header) in preview.headers.iter().enumerate() {
                        let key = header_key(header);
                        for row in &preview.rows {
                            let value = row
                                .get(column)
                                .map(|value| value.trim())
                                .unwrap_or_default();
                            if value.is_empty() {
                                continue;
                            }
                            *non_empty_counts.entry(key.clone()).or_default() += 1;
                            let kind = if value.parse::<f64>().is_ok() {
                                "数字"
                            } else if matches!(
                                value.to_lowercase().as_str(),
                                "true" | "false" | "是" | "否"
                            ) {
                                "布尔"
                            } else {
                                "文本"
                            };
                            sampled_types.entry(key.clone()).or_default().insert(kind);
                        }
                    }
                }
                Err(error) => issues.push(issue(
                    IssueLevel::Error,
                    "无法抽样读取",
                    &format!("{}：{error:#}", table.display_name()),
                )),
            }
        }
        for (column, kinds) in sampled_types {
            if kinds.len() > 1 {
                let mut kinds = kinds.into_iter().collect::<Vec<_>>();
                kinds.sort();
                issues.push(issue(
                    IssueLevel::Warning,
                    "字段类型混合",
                    &format!("字段“{column}”的样本同时出现：{}。", kinds.join("、")),
                ));
            }
        }
        for column in &union {
            if !non_empty_counts.contains_key(column) {
                issues.push(issue(
                    IssueLevel::Warning,
                    "字段样本全空",
                    &format!("字段“{column}”在抽样行中没有非空值。"),
                ));
            }
        }
    }
    let pairs = crate::inspect_similarity::similar_headers(union.iter().cloned().collect());
    let similar = pairs
        .iter()
        .map(|(left, right)| format!("{left} ↔ {right}"))
        .collect::<Vec<_>>();
    if !similar.is_empty() {
        issues.push(issue(
            IssueLevel::Warning,
            "疑似同义/错拼字段",
            &similar.join("；"),
        ));
    }
    if !options.filter_text.trim().is_empty() {
        if options.filter_column.trim().is_empty() {
            issues.push(issue(
                IssueLevel::Warning,
                "筛选字段未填写",
                "已设置筛选文本，但字段名为空，筛选不会生效。",
            ));
        } else if !plan
            .headers
            .iter()
            .any(|header| header_key(header) == header_key(&options.filter_column))
        {
            issues.push(issue(
                IssueLevel::Error,
                "筛选字段不存在",
                &format!(
                    "输出列中没有“{}”，筛选不会生效，请检查字段名。",
                    options.filter_column
                ),
            ));
        }
    }
    issues.sort_by_key(|item| std::cmp::Reverse(item.level));
    issues
}

#[derive(Debug, Default, PartialEq, Eq)]
pub struct CsvCheck {
    pub rows: usize,
    pub wider_rows: usize,
    pub first_wider_line: Option<usize>,
}

/// 按实际读取方式校验 CSV：跳过表头及其上方的标题行，与扫描和合并一样
/// 允许行宽不一致；只有数据行在表头之外仍有非空值时才会丢数据，需要提醒。
pub fn validate_csv(
    path: &std::path::Path,
    delimiter: u8,
    skipped_rows: usize,
    header_width: usize,
    max_rows: usize,
) -> Result<CsvCheck> {
    let encoding = csv_encoding(path)?;
    let mut reader = ReaderBuilder::new()
        .has_headers(false)
        .flexible(true)
        .delimiter(delimiter)
        .from_path(path)?;
    let mut record = ByteRecord::new();
    let mut line = 0;
    let mut check = CsvCheck::default();
    while check.rows < max_rows
        && reader
            .read_byte_record(&mut record)
            .with_context(|| format!("第 {} 行格式异常", line + 1))?
    {
        line += 1;
        if line <= skipped_rows {
            continue;
        }
        check.rows += 1;
        let mut wider = false;
        for (index, field) in record.iter().enumerate() {
            let value = decode_csv_field(field, encoding);
            wider |= index >= header_width && !value.trim().is_empty();
        }
        if wider {
            check.wider_rows += 1;
            check.first_wider_line.get_or_insert(line);
        }
    }
    Ok(check)
}

fn issue(level: IssueLevel, title: &str, detail: &str) -> CheckIssue {
    CheckIssue {
        level,
        title: title.to_owned(),
        detail: detail.to_owned(),
    }
}

#[cfg(test)]
mod cache_tests {
    use super::*;
    #[test]
    fn raw_samples_expand_and_current_mapping_never_reuses_old_results() -> Result<()> {
        let directory = tempfile::tempdir()?;
        let path = directory.path().join("source.csv");
        let mut csv = csv::Writer::from_path(&path)?;
        csv.write_record(["编号", "金额"])?;
        for index in 0..150 {
            csv.write_record([format!("00{index}"), format!("{index}.125")])?;
        }
        csv.flush()?;
        let mut table = crate::scan::scan_file(&path)?.0.remove(0);
        let mut reader = SampleReader::default();
        let first = source_sample(&table, 30, &mut reader)?;
        assert_eq!(first.values.len(), 30);
        let smaller = source_sample(&table, 5, &mut reader)?;
        assert!(std::sync::Arc::ptr_eq(&first, &smaller));
        let expanded = source_sample(&table, 100, &mut reader)?;
        assert_eq!(expanded.values.len(), 100);
        assert!(!std::sync::Arc::ptr_eq(&first, &expanded));
        table.mappings[0].target_name = "独立编号".into();
        table.mappings[1].enabled = false;
        let merged = preview_merged(&[table.clone()], &MergeOptions::default(), 5)?;
        assert_eq!(merged.headers, ["独立编号"]);
        assert_eq!(merged.groups[0].rows[0], ["000"]);
        invalidate_source_preview(&path);
        assert!(!std::sync::Arc::ptr_eq(
            &expanded,
            &source_sample(&table, 100, &mut reader)?
        ));
        // A replacement with another encoding must invalidate samples and encoding.
        drop(csv);
        let (bytes, _, errors) = encoding_rs::GBK.encode("编号,金额\n新值,9\n");
        assert!(!errors);
        std::fs::write(&path, bytes.as_ref())?;
        assert_eq!(
            preview_source(&table, 30)?.rows,
            vec![vec!["新值".to_owned(), "9".to_owned()]]
        );
        std::fs::remove_file(&path)?;
        assert!(preview_source(&table, 30).is_err());
        Ok(())
    }
    #[test]
    fn zero_row_preview_does_not_open_source() {
        let table = crate::model::SourceTable {
            path: "missing.xlsx".into(),
            sheet_name: "missing".into(),
            kind: SourceKind::Workbook,
            header_row: 1,
            header_rows: 1,
            suggested_header_row: 1,
            headers: vec!["a".into()],
            estimated_rows: 0,
            enabled: true,
            mappings: vec![],
        };
        assert!(preview_source(&table, 0).unwrap().rows.is_empty());
    }
}
