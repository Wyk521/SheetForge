use crate::model::{
    build_output_plan, header_key, source_to_output_map, MergeOptions, SourceKind, SourceTable,
};
use crate::scan::{decode_csv_field, for_each_csv_row};
use anyhow::{Context, Result};
use calamine::{open_workbook_auto, Data, Reader};
use csv::{ByteRecord, ReaderBuilder};
use std::collections::{HashMap, HashSet};

#[derive(Clone, Debug)]
pub struct PreviewTable {
    pub headers: Vec<String>,
    pub rows: Vec<Vec<String>>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum IssueLevel {
    Info,
    Warning,
    Error,
}

#[derive(Clone, Debug)]
pub struct CheckIssue {
    pub level: IssueLevel,
    pub title: String,
    pub detail: String,
}

pub fn preview_source(table: &SourceTable, limit: usize) -> Result<PreviewTable> {
    let mut rows = Vec::new();
    match table.kind {
        SourceKind::Csv { delimiter } => {
            for_each_csv_row(
                &table.path,
                delimiter,
                table.header_row,
                table.header_rows,
                |row| {
                    if rows.len() < limit {
                        rows.push(row);
                    }
                    Ok(())
                },
            )?;
        }
        SourceKind::Workbook => {
            let mut workbook = open_workbook_auto(&table.path)?;
            let range = workbook.worksheet_range(&table.sheet_name)?;
            rows.extend(
                range
                    .rows()
                    .skip(table.header_row + table.header_rows - 1)
                    .take(limit)
                    .map(|row| row.iter().map(Data::to_string).collect()),
            );
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
) -> Result<PreviewTable> {
    let plan = build_output_plan(tables, options);
    let mut rows = Vec::new();
    for table in tables.iter().filter(|table| table.enabled) {
        if rows.len() >= limit {
            break;
        }
        let preview = preview_source(table, limit - rows.len())?;
        let mapping = source_to_output_map(table, &plan, options.mode);
        for source_row in preview.rows {
            let mut output = vec![String::new(); plan.headers.len()];
            for (source, target) in &mapping {
                if let Some(value) = source_row.get(*source) {
                    output[*target] = value.clone();
                }
            }
            if let Some(index) = plan.source_file_column {
                output[index] = table
                    .path
                    .file_name()
                    .map(|name| name.to_string_lossy().into_owned())
                    .unwrap_or_default();
            }
            if let Some(index) = plan.source_sheet_column {
                output[index] = table.sheet_name.clone();
            }
            rows.push(output);
        }
    }
    Ok(PreviewTable {
        headers: plan.headers,
        rows,
    })
}

pub fn preflight(tables: &[SourceTable], options: &MergeOptions) -> Vec<CheckIssue> {
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
    if plan.headers.len() > 16_384 {
        issues.push(issue(
            IssueLevel::Error,
            "列数超出 XLSX 限制",
            &format!("预计输出 {} 列。", plan.headers.len()),
        ));
    }
    let rows: u64 = enabled.iter().map(|table| table.estimated_rows).sum();
    let sheets = rows.div_ceil(1_048_575).max(1);
    issues.push(issue(
        IssueLevel::Info,
        "输出规模",
        &format!(
            "预计 {rows} 行、{} 列、约 {sheets} 个结果 Sheet。",
            plan.headers.len()
        ),
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
        let missing = union.difference(set).count();
        if missing > 0 {
            issues.push(issue(
                IssueLevel::Warning,
                "字段不一致",
                &format!("{} 缺少并集中 {missing} 个字段。", table.display_name()),
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
            if let Err(error) = validate_csv(&table.path, delimiter, 10_000) {
                issues.push(issue(
                    IssueLevel::Error,
                    "CSV 行格式异常",
                    &format!("{}：{error:#}", table.display_name()),
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
    if matches!(
        options.mode,
        crate::model::MergeMode::Consolidate | crate::model::MergeMode::Join
    ) && options.key_columns.is_empty()
    {
        issues.push(issue(
            IssueLevel::Error,
            "缺少键字段",
            "汇总或横向关联至少需要一个键字段。",
        ));
    }
    issues.sort_by_key(|item| std::cmp::Reverse(item.level));
    issues
}

pub fn mapping_suggestions(tables: &[SourceTable]) -> HashMap<String, String> {
    let mut frequency = HashMap::<String, (String, usize)>::new();
    for table in tables.iter().filter(|table| table.enabled) {
        for header in &table.headers {
            let entry = frequency
                .entry(header_key(header))
                .or_insert((header.clone(), 0));
            entry.1 += 1;
        }
    }
    let candidates = frequency
        .values()
        .filter(|(_, count)| *count >= 2)
        .map(|(name, _)| name)
        .collect::<Vec<_>>();
    let mut result = HashMap::new();
    for table in tables.iter().filter(|table| table.enabled) {
        for header in &table.headers {
            if candidates
                .iter()
                .any(|candidate| header_key(candidate) == header_key(header))
            {
                continue;
            }
            if let Some(best) = candidates.iter().max_by(|a, b| {
                strsim::normalized_levenshtein(&header_key(header), &header_key(a)).total_cmp(
                    &strsim::normalized_levenshtein(&header_key(header), &header_key(b)),
                )
            }) {
                if strsim::normalized_levenshtein(&header_key(header), &header_key(best)) >= 0.62 {
                    result.insert(header.clone(), (*best).clone());
                }
            }
        }
    }
    result
}

pub fn validate_csv(
    path: &std::path::Path,
    delimiter: u8,
    max_rows: usize,
) -> Result<(usize, usize)> {
    let mut reader = ReaderBuilder::new()
        .has_headers(false)
        .flexible(false)
        .delimiter(delimiter)
        .from_path(path)?;
    let mut record = ByteRecord::new();
    let mut rows = 0;
    let mut width = 0;
    while rows < max_rows
        && reader
            .read_byte_record(&mut record)
            .with_context(|| format!("第 {} 行格式异常", rows + 1))?
    {
        width = width.max(record.len());
        rows += 1;
        for field in &record {
            let _ = decode_csv_field(field);
        }
    }
    Ok((rows, width))
}

fn issue(level: IssueLevel, title: &str, detail: &str) -> CheckIssue {
    CheckIssue {
        level,
        title: title.to_owned(),
        detail: detail.to_owned(),
    }
}
