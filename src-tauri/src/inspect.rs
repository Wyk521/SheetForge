use crate::merge::{preview_mapped_row, CellValue};
use crate::model::{build_output_plan, header_key, MergeOptions, SourceKind, SourceTable};
use crate::scan::{
    csv_encoding, decode_csv_field, for_each_xlsx_row, is_markup_workbook, is_xlsx_path,
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

pub fn preview_source(table: &SourceTable, limit: usize) -> Result<PreviewTable> {
    let mut rows = Vec::new();
    match table.kind {
        SourceKind::Csv { delimiter } => {
            let encoding = csv_encoding(&table.path)?;
            let mut reader = ReaderBuilder::new()
                .has_headers(false)
                .flexible(true)
                .delimiter(delimiter)
                .from_path(&table.path)?;
            let skip = table.header_row + table.header_rows - 1;
            for record in reader.byte_records().skip(skip).take(limit) {
                rows.push(
                    record?
                        .iter()
                        .map(|field| decode_csv_field(field, encoding))
                        .collect(),
                );
            }
        }
        SourceKind::Workbook => {
            if is_xlsx_path(&table.path) {
                match open_xlsx_workbook(&table.path) {
                    Ok(mut workbook) => {
                        for_each_xlsx_row(
                            &mut workbook,
                            &table.sheet_name,
                            table.header_row + table.header_rows - 1,
                            Some(limit),
                            |row| {
                                let mut values = vec![String::new(); table.headers.len()];
                                for (index, value) in row.iter().enumerate().take(values.len()) {
                                    values[index] = value.to_string();
                                }
                                rows.push(values);
                                Ok(())
                            },
                        )?;
                    }
                    Err(primary_error) => rows.extend(
                        fallback_workbook_text_rows(table, limit)
                            .with_context(|| format!("常规 XLSX 读取失败：{primary_error:#}"))?,
                    ),
                }
            } else {
                match open_workbook_auto(&table.path) {
                    Ok(mut workbook) => {
                        let range = workbook.worksheet_range(&table.sheet_name)?;
                        rows.extend(
                            range
                                .rows()
                                .skip(table.header_row + table.header_rows - 1)
                                .take(limit)
                                .map(|row| row.iter().map(Data::to_string).collect()),
                        );
                    }
                    Err(primary_error) => rows.extend(
                        fallback_workbook_text_rows(table, limit)
                            .with_context(|| format!("常规读取失败：{primary_error}"))?,
                    ),
                }
            }
        }
    }
    Ok(PreviewTable {
        headers: table.headers.clone(),
        rows,
    })
}

pub fn preview_merged(
    tables: &[SourceTable],
    options: &MergeOptions,
    limit: usize,
) -> Result<MergedPreview> {
    let plan = build_output_plan(tables, options);
    let mut groups = Vec::new();
    let mut seen = HashSet::new();
    for (source_index, table) in tables.iter().enumerate().filter(|(_, table)| table.enabled) {
        let source_rows = preview_source_values(table, limit)?;
        let mut rows = Vec::with_capacity(source_rows.len());
        for source_row in source_rows {
            if let Some(output) = preview_mapped_row(source_row, table, &plan, options) {
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
                .map(|name| name.to_string_lossy().into_owned())
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

fn preview_source_values(table: &SourceTable, limit: usize) -> Result<Vec<Vec<CellValue>>> {
    let mut rows = Vec::new();
    match table.kind {
        SourceKind::Csv { delimiter } => {
            let encoding = csv_encoding(&table.path)?;
            let mut reader = ReaderBuilder::new()
                .has_headers(false)
                .flexible(true)
                .delimiter(delimiter)
                .from_path(&table.path)?;
            let skip = table.header_row + table.header_rows - 1;
            for record in reader.byte_records().skip(skip).take(limit) {
                rows.push(
                    record?
                        .iter()
                        .map(|value| CellValue::Text(decode_csv_field(value, encoding)))
                        .collect(),
                );
            }
        }
        SourceKind::Workbook => {
            if is_xlsx_path(&table.path) {
                match open_xlsx_workbook(&table.path) {
                    Ok(mut workbook) => {
                        for_each_xlsx_row(
                            &mut workbook,
                            &table.sheet_name,
                            table.header_row + table.header_rows - 1,
                            Some(limit),
                            |row| {
                                rows.push(row.iter().map(CellValue::from_calamine).collect());
                                Ok(())
                            },
                        )?;
                    }
                    Err(primary_error) => rows.extend(
                        fallback_workbook_text_rows(table, limit)
                            .with_context(|| format!("常规 XLSX 读取失败：{primary_error:#}"))?
                            .into_iter()
                            .map(|row| row.into_iter().map(CellValue::from_text).collect()),
                    ),
                }
            } else {
                match open_workbook_auto(&table.path) {
                    Ok(mut workbook) => {
                        let range = workbook.worksheet_range(&table.sheet_name)?;
                        rows.extend(
                            range
                                .rows()
                                .skip(table.header_row + table.header_rows - 1)
                                .take(limit)
                                .map(|row| row.iter().map(CellValue::from_calamine).collect()),
                        );
                    }
                    Err(primary_error) => rows.extend(
                        fallback_workbook_text_rows(table, limit)
                            .with_context(|| format!("常规读取失败：{primary_error}"))?
                            .into_iter()
                            .map(|row| row.into_iter().map(CellValue::from_text).collect()),
                    ),
                }
            }
        }
    }
    Ok(rows)
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
        for table in &enabled {
            match preview_source(table, 100) {
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
    let mut similar = Vec::new();
    let names = union.iter().collect::<Vec<_>>();
    for left in 0..names.len() {
        for right in left + 1..names.len() {
            let score = strsim::normalized_levenshtein(names[left], names[right]);
            if (0.72..1.0).contains(&score) {
                similar.push(format!("{} ↔ {}", names[left], names[right]));
            }
            if similar.len() >= 8 {
                break;
            }
        }
        if similar.len() >= 8 {
            break;
        }
    }
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
