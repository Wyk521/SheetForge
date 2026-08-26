use crate::inspect::{preflight_for_destination, CheckIssue};
use crate::model::{
    build_output_plan, header_key, source_to_output_map, AggregateOp, JoinKind, MergeMode,
    MergeOptions, OutputPlan, SourceKind, SourceTable, TransformOp,
};
use crate::scan::{for_each_csv_row, for_each_xlsx_row, is_xlsx_path};
use anyhow::{anyhow, Context, Result};
use calamine::{open_workbook, open_workbook_auto, Data, Reader, Xlsx};
use rust_xlsxwriter::{Color, ExcelDateTime, Format, FormatAlign, FormatBorder, Workbook};
use serde::Serialize;
use std::collections::{HashMap, HashSet};
use std::fs::File;
use std::io::BufReader;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tauri::{AppHandle, Emitter};

pub const XLSX_MAX_ROWS: u32 = 1_048_576;
pub const XLSX_MAX_DATA_ROWS: u32 = XLSX_MAX_ROWS - 1;

#[derive(Clone, Serialize)]
pub struct MergeProgressDto {
    pub current: u64,
    pub total: u64,
    pub label: String,
}

#[derive(Clone, Serialize)]
pub struct MergeFinishedDto {
    pub output: PathBuf,
    pub rows: u64,
    pub sheets: usize,
}

#[derive(Clone, Serialize)]
pub struct MergeFailedDto {
    pub message: String,
}

#[derive(Clone, Serialize)]
pub struct PreflightDoneDto {
    pub issues: Vec<CheckIssue>,
    pub continues_merge: bool,
}

/// 用户主动取消合并时的类型化错误，用于与真正的失败区分开。
#[derive(Debug)]
struct MergeCancelled;

impl std::fmt::Display for MergeCancelled {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "合并已取消")
    }
}

impl std::error::Error for MergeCancelled {}

pub fn spawn_merge(
    tables: Vec<SourceTable>,
    options: MergeOptions,
    output: PathBuf,
    app: AppHandle,
    cancel: Arc<AtomicBool>,
) {
    std::thread::spawn(move || {
        let emit_progress = |current: u64, total: u64, label: String| {
            let _ = app.emit(
                "merge-progress",
                MergeProgressDto {
                    current,
                    total,
                    label,
                },
            );
        };
        match merge_tables(&tables, &options, &output, &emit_progress, &cancel) {
            Ok(Some((rows, sheets))) => {
                let _ = app.emit(
                    "merge-finished",
                    MergeFinishedDto {
                        output,
                        rows,
                        sheets,
                    },
                );
            }
            Ok(None) => {
                let _ = app.emit("merge-cancelled", ());
            }
            Err(error) => {
                if error.downcast_ref::<MergeCancelled>().is_some() {
                    let _ = app.emit("merge-cancelled", ());
                } else {
                    let _ = app.emit(
                        "merge-failed",
                        MergeFailedDto {
                            message: format!("{error:#}"),
                        },
                    );
                }
            }
        }
    });
}

/// 在后台线程执行合并前检查，避免大文件时冻结界面。
pub fn spawn_preflight(
    tables: Vec<SourceTable>,
    options: MergeOptions,
    continues_merge: bool,
    database: bool,
    app: AppHandle,
) {
    std::thread::spawn(move || {
        let issues = preflight_for_destination(&tables, &options, database);
        let _ = app.emit(
            "preflight-done",
            PreflightDoneDto {
                issues,
                continues_merge,
            },
        );
    });
}

pub(crate) fn merge_tables(
    tables: &[SourceTable],
    options: &MergeOptions,
    output: &Path,
    emit: &dyn Fn(u64, u64, String),
    cancel: &AtomicBool,
) -> Result<Option<(u64, usize)>> {
    let enabled = tables
        .iter()
        .filter(|table| table.enabled)
        .collect::<Vec<_>>();
    if enabled.is_empty() {
        return Err(anyhow!("没有勾选任何表"));
    }
    let plan = build_output_plan(tables, options);
    if plan.headers.is_empty() {
        return Err(anyhow!("合并后没有可输出的列，请检查合并方式或字段映射"));
    }
    if plan.headers.len() > 16_384 {
        return Err(anyhow!("输出列数超过 XLSX 的 16,384 列限制"));
    }

    let mut sink = XlsxSink::new(plan.clone())?;
    let rows = merge_into_sink(&enabled, options, &plan, &mut sink, emit, cancel)?;
    match rows {
        Some(rows) => finish_sink(sink, output, rows, cancel),
        None => Ok(None),
    }
}

/// 将 SheetForge 已应用表头修改、交并集、清洗、汇总或关联后的最终行流交给调用方。
/// PostgreSQL 适配层使用此入口，确保数据库与 XLSX 得到完全相同的业务数据。
pub(crate) fn stream_merged_rows<F>(
    tables: &[SourceTable],
    options: &MergeOptions,
    empty_as_null: bool,
    emit: &dyn Fn(u64, u64, String),
    cancel: &AtomicBool,
    write_row: F,
) -> Result<Option<(OutputPlan, u64)>>
where
    F: FnMut(Vec<Option<String>>) -> Result<()>,
{
    let enabled = tables
        .iter()
        .filter(|table| table.enabled)
        .collect::<Vec<_>>();
    if enabled.is_empty() {
        return Err(anyhow!("没有勾选任何表"));
    }
    let plan = build_output_plan(tables, options);
    if plan.headers.is_empty() {
        return Err(anyhow!("合并后没有可导入的列，请检查合并方式或字段映射"));
    }
    let mut sink = CallbackSink {
        write_row,
        empty_as_null,
        total_rows: 0,
    };
    let rows = merge_into_sink(&enabled, options, &plan, &mut sink, emit, cancel)?;
    Ok(rows.map(|rows| (plan, rows)))
}

fn merge_into_sink<S: RowSink>(
    tables: &[&SourceTable],
    options: &MergeOptions,
    plan: &OutputPlan,
    sink: &mut S,
    emit: &dyn Fn(u64, u64, String),
    cancel: &AtomicBool,
) -> Result<Option<u64>> {
    // 这些映射只依赖于表头和用户配置。之前每一行都会重新构建哈希表、
    // 规范化表头并查找转换规则，在百万行数据上会把纯粹的配置开销放大数亿次。
    let row_mappings = tables
        .iter()
        .map(|table| build_row_mapping(table, plan, options.mode))
        .collect::<Vec<_>>();
    match options.mode {
        MergeMode::Consolidate => {
            merge_consolidated(tables, &row_mappings, options, plan, sink, emit, cancel)
        }
        MergeMode::Join => merge_joined(tables, &row_mappings, options, plan, sink, emit, cancel),
        _ => merge_appended(tables, &row_mappings, options, plan, sink, emit, cancel),
    }
}

fn merge_appended<S: RowSink>(
    tables: &[&SourceTable],
    row_mappings: &[RowMapping],
    options: &MergeOptions,
    plan: &OutputPlan,
    sink: &mut S,
    emit: &dyn Fn(u64, u64, String),
    cancel: &AtomicBool,
) -> Result<Option<u64>> {
    let total = tables.iter().map(|table| table.estimated_rows).sum();
    let mut current = 0_u64;
    let mut dedup = HashSet::new();
    let key_indices = key_indices(plan, &options.key_columns);
    let mut processed_workbooks = HashSet::<PathBuf>::new();

    for (table_index, table) in tables.iter().enumerate() {
        if cancel.load(Ordering::Relaxed) {
            return Ok(None);
        }
        match table.kind {
            SourceKind::Csv { delimiter } => {
                let mut consume = |row: Vec<CellValue>| {
                    consume_append_row(
                        table,
                        row,
                        &row_mappings[table_index],
                        options,
                        plan,
                        &key_indices,
                        &mut dedup,
                        sink,
                        &mut current,
                        total,
                        emit,
                        cancel,
                    )
                };
                for_each_csv_row(
                    &table.path,
                    delimiter,
                    table.header_row,
                    table.header_rows,
                    |row| consume(row.into_iter().map(CellValue::Text).collect()),
                )
                .with_context(|| format!("处理 {} 时失败", table.display_name()))?;
            }
            SourceKind::Workbook => {
                if !processed_workbooks.insert(table.path.clone()) {
                    continue;
                }
                let same_file = tables
                    .iter()
                    .enumerate()
                    .filter(|(_, candidate)| {
                        matches!(candidate.kind, SourceKind::Workbook)
                            && candidate.path == table.path
                    })
                    .map(|(index, candidate)| (index, *candidate))
                    .collect::<Vec<_>>();
                if is_xlsx_path(&table.path) {
                    let mut workbook: Xlsx<BufReader<File>> = open_workbook(&table.path)
                        .with_context(|| format!("无法打开 {}", table.path.display()))?;
                    for (sheet_index, sheet) in same_file {
                        for_each_xlsx_row(
                            &mut workbook,
                            &sheet.sheet_name,
                            sheet.header_row + sheet.header_rows - 1,
                            None,
                            |row| {
                                consume_append_row(
                                    sheet,
                                    row.iter().map(CellValue::from_calamine).collect(),
                                    &row_mappings[sheet_index],
                                    options,
                                    plan,
                                    &key_indices,
                                    &mut dedup,
                                    sink,
                                    &mut current,
                                    total,
                                    emit,
                                    cancel,
                                )
                            },
                        )
                        .with_context(|| format!("处理 {} 时失败", sheet.display_name()))?;
                    }
                } else {
                    let mut workbook = open_workbook_auto(&table.path)
                        .with_context(|| format!("无法打开 {}", table.path.display()))?;
                    for (sheet_index, sheet) in same_file {
                        let range = workbook
                            .worksheet_range(&sheet.sheet_name)
                            .with_context(|| format!("无法读取 {}", sheet.display_name()))?;
                        for row in range.rows().skip(sheet.header_row + sheet.header_rows - 1) {
                            consume_append_row(
                                sheet,
                                row.iter().map(CellValue::from_calamine).collect(),
                                &row_mappings[sheet_index],
                                options,
                                plan,
                                &key_indices,
                                &mut dedup,
                                sink,
                                &mut current,
                                total,
                                emit,
                                cancel,
                            )?;
                        }
                    }
                }
            }
        }
    }
    Ok(Some(current))
}

#[allow(clippy::too_many_arguments)]
fn consume_append_row<S: RowSink>(
    table: &SourceTable,
    values: Vec<CellValue>,
    row_mapping: &RowMapping,
    options: &MergeOptions,
    plan: &OutputPlan,
    key_indices: &[usize],
    dedup: &mut HashSet<String>,
    sink: &mut S,
    current: &mut u64,
    total: u64,
    emit: &dyn Fn(u64, u64, String),
    cancel: &AtomicBool,
) -> Result<()> {
    if cancel.load(Ordering::Relaxed) {
        return Err(MergeCancelled.into());
    }
    let output_row = mapped_row(values, row_mapping, plan);
    if !passes_filter(&output_row, plan, options) {
        return Ok(());
    }
    if options.deduplicate {
        let key = row_key(&output_row, key_indices);
        if !dedup.insert(key) {
            return Ok(());
        }
    }
    sink.write_row(&output_row)?;
    *current += 1;
    send_progress(*current, total, table.display_name(), emit);
    Ok(())
}

fn merge_consolidated<S: RowSink>(
    tables: &[&SourceTable],
    row_mappings: &[RowMapping],
    options: &MergeOptions,
    plan: &OutputPlan,
    sink: &mut S,
    emit: &dyn Fn(u64, u64, String),
    cancel: &AtomicBool,
) -> Result<Option<u64>> {
    let keys = key_indices(plan, &options.key_columns);
    if keys.is_empty() {
        return Err(anyhow!("按键汇总至少需要一个有效的键字段"));
    }
    let operations = aggregate_operations(tables, plan);
    let mut groups = HashMap::<String, Vec<CellValue>>::new();
    let total = tables.iter().map(|table| table.estimated_rows).sum();
    let mut current = 0;
    for (table_index, table) in tables.iter().enumerate() {
        for_each_table_row(table, |values| {
            if cancel.load(Ordering::Relaxed) {
                return Err(MergeCancelled.into());
            }
            let row = mapped_row(values, &row_mappings[table_index], plan);
            if passes_filter(&row, plan, options) {
                let key = row_key(&row, &keys);
                groups
                    .entry(key)
                    .and_modify(|existing| {
                        aggregate_rows(existing, &row, &operations, &options.text_join_separator)
                    })
                    .or_insert(row);
            }
            current += 1;
            send_progress(current, total, table.display_name(), emit);
            Ok(())
        })?;
    }
    if cancel.load(Ordering::Relaxed) {
        return Ok(None);
    }
    let mut rows = groups.into_iter().collect::<Vec<_>>();
    rows.sort_by(|left, right| left.0.cmp(&right.0));
    for (_, row) in rows {
        sink.write_row(&row)?;
    }
    Ok(Some(sink.total_rows()))
}

fn merge_joined<S: RowSink>(
    tables: &[&SourceTable],
    row_mappings: &[RowMapping],
    options: &MergeOptions,
    plan: &OutputPlan,
    sink: &mut S,
    emit: &dyn Fn(u64, u64, String),
    cancel: &AtomicBool,
) -> Result<Option<u64>> {
    let keys = key_indices(plan, &options.key_columns);
    if keys.is_empty() {
        return Err(anyhow!("横向关联至少需要一个有效的键字段"));
    }
    let mut current_rows = Vec::<Vec<CellValue>>::new();
    let total = tables.iter().map(|table| table.estimated_rows).sum();
    let mut progress = 0;
    for (table_index, table) in tables.iter().enumerate() {
        let mut incoming = HashMap::<String, Vec<CellValue>>::new();
        for_each_table_row(table, |values| {
            if cancel.load(Ordering::Relaxed) {
                return Err(MergeCancelled.into());
            }
            let row = mapped_row(values, &row_mappings[table_index], plan);
            incoming
                .entry(row_key(&row, &keys))
                .and_modify(|existing| fill_empty(existing, &row))
                .or_insert(row);
            progress += 1;
            send_progress(progress, total, table.display_name(), emit);
            Ok(())
        })?;
        if table_index == 0 {
            current_rows = incoming.into_values().collect();
            continue;
        }
        let mut matched = HashSet::new();
        current_rows.retain_mut(|row| {
            let key = row_key(row, &keys);
            if let Some(other) = incoming.get(&key) {
                fill_empty(row, other);
                matched.insert(key);
                true
            } else {
                options.join_kind != JoinKind::Inner
            }
        });
        if options.join_kind == JoinKind::Full {
            current_rows.extend(
                incoming
                    .into_iter()
                    .filter(|(key, _)| !matched.contains(key))
                    .map(|(_, row)| row),
            );
        }
    }
    if cancel.load(Ordering::Relaxed) {
        return Ok(None);
    }
    current_rows.sort_by_key(|row| row_key(row, &keys));
    for row in current_rows {
        if passes_filter(&row, plan, options) {
            sink.write_row(&row)?;
        }
    }
    Ok(Some(sink.total_rows()))
}

fn for_each_table_row<F>(table: &SourceTable, mut callback: F) -> Result<()>
where
    F: FnMut(Vec<CellValue>) -> Result<()>,
{
    match table.kind {
        SourceKind::Csv { delimiter } => for_each_csv_row(
            &table.path,
            delimiter,
            table.header_row,
            table.header_rows,
            |row| callback(row.into_iter().map(CellValue::Text).collect()),
        ),
        SourceKind::Workbook => {
            if is_xlsx_path(&table.path) {
                let mut workbook: Xlsx<BufReader<File>> = open_workbook(&table.path)?;
                for_each_xlsx_row(
                    &mut workbook,
                    &table.sheet_name,
                    table.header_row + table.header_rows - 1,
                    None,
                    |row| callback(row.iter().map(CellValue::from_calamine).collect()),
                )?;
                Ok(())
            } else {
                let mut workbook = open_workbook_auto(&table.path)?;
                let range = workbook.worksheet_range(&table.sheet_name)?;
                for row in range.rows().skip(table.header_row + table.header_rows - 1) {
                    callback(row.iter().map(CellValue::from_calamine).collect())?;
                }
                Ok(())
            }
        }
    }
}

struct RowMapping {
    /// (源列、输出列、转换操作、是否可以从输入行直接移动值)。
    columns: Vec<(usize, usize, TransformOp, bool)>,
    source_file: Option<String>,
    source_sheet: Option<String>,
}

fn build_row_mapping(table: &SourceTable, plan: &OutputPlan, mode: MergeMode) -> RowMapping {
    let columns = source_to_output_map(table, plan, mode);
    let mut source_use_count = HashMap::<usize, usize>::new();
    for (source_index, _) in &columns {
        *source_use_count.entry(*source_index).or_default() += 1;
    }

    // 与 mapped_row 之前的 find 语义保持一致：重复 source_index 时取第一条规则。
    let mut transforms = vec![None; table.headers.len()];
    for mapping in &table.mappings {
        if let Some(transform) = transforms.get_mut(mapping.source_index) {
            if transform.is_none() {
                *transform = Some(mapping.transform);
            }
        }
    }

    let columns = columns
        .into_iter()
        .map(|(source_index, output_index)| {
            let transform = transforms
                .get(source_index)
                .and_then(|value| *value)
                .unwrap_or(TransformOp::None);
            let can_move = transform == TransformOp::None
                && source_use_count.get(&source_index).copied() == Some(1);
            (source_index, output_index, transform, can_move)
        })
        .collect();

    RowMapping {
        columns,
        source_file: plan.source_file_column.map(|_| {
            table
                .path
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_default()
        }),
        source_sheet: plan.source_sheet_column.map(|_| table.sheet_name.clone()),
    }
}

fn mapped_row(
    values: Vec<CellValue>,
    row_mapping: &RowMapping,
    plan: &OutputPlan,
) -> Vec<CellValue> {
    let mut values = values;
    let mut output = vec![CellValue::Empty; plan.headers.len()];
    for &(source_index, output_index, transform, can_move) in &row_mapping.columns {
        if source_index < values.len() {
            let value = if can_move {
                std::mem::replace(&mut values[source_index], CellValue::Empty)
            } else {
                values[source_index].transformed(transform)
            };
            if output[output_index].is_empty() && !value.is_empty() {
                output[output_index] = value;
            }
        }
    }
    if let Some(index) = plan.source_file_column {
        output[index] = CellValue::Text(row_mapping.source_file.clone().unwrap_or_default());
    }
    if let Some(index) = plan.source_sheet_column {
        output[index] = CellValue::Text(row_mapping.source_sheet.clone().unwrap_or_default());
    }
    output
}

fn key_indices(plan: &OutputPlan, columns: &[String]) -> Vec<usize> {
    let requested = columns
        .iter()
        .map(|column| header_key(column))
        .collect::<HashSet<_>>();
    plan.headers
        .iter()
        .enumerate()
        .filter(|(_, header)| requested.contains(&header_key(header)))
        .map(|(index, _)| index)
        .collect()
}

fn row_key(row: &[CellValue], indices: &[usize]) -> String {
    let selected = if indices.is_empty() {
        (0..row.len()).collect::<Vec<_>>()
    } else {
        indices.to_vec()
    };
    selected
        .into_iter()
        .map(|index| row.get(index).map(CellValue::as_text).unwrap_or_default())
        .collect::<Vec<_>>()
        .join("\u{001f}")
}

fn passes_filter(row: &[CellValue], plan: &OutputPlan, options: &MergeOptions) -> bool {
    if options.filter_text.is_empty() {
        return true;
    }
    let Some(index) = plan
        .headers
        .iter()
        .position(|header| header_key(header) == header_key(&options.filter_column))
    else {
        return true;
    };
    let contains = row
        .get(index)
        .map(CellValue::as_text)
        .unwrap_or_default()
        .to_lowercase()
        .contains(&options.filter_text.to_lowercase());
    if options.filter_exclude {
        !contains
    } else {
        contains
    }
}

fn aggregate_operations(tables: &[&SourceTable], plan: &OutputPlan) -> Vec<AggregateOp> {
    plan.headers
        .iter()
        .map(|header| {
            tables
                .iter()
                .flat_map(|table| &table.mappings)
                .find(|mapping| header_key(&mapping.target_name) == header_key(header))
                .map(|mapping| mapping.aggregate)
                .unwrap_or(AggregateOp::First)
        })
        .collect()
}

fn aggregate_rows(
    existing: &mut [CellValue],
    incoming: &[CellValue],
    operations: &[AggregateOp],
    separator: &str,
) {
    for (index, current) in existing.iter_mut().enumerate() {
        let other = incoming.get(index).cloned().unwrap_or(CellValue::Empty);
        match operations.get(index).copied().unwrap_or(AggregateOp::First) {
            AggregateOp::First => {
                if current.is_empty() {
                    *current = other;
                }
            }
            AggregateOp::Sum => {
                // 整数用 i128 累加避免金额/大数在浮点累加中丢精度；
                // 结果在 f64 安全范围内输出为数值(Excel 可参与计算)，
                // 超出 2^53 才降级为文本以保住每一位精度。
                if let (Ok(current_int), Ok(other_int)) = (
                    current.as_text().trim().parse::<i128>(),
                    other.as_text().trim().parse::<i128>(),
                ) {
                    let total = current_int + other_int;
                    // Excel 对超过 15 位的整数会强制科学计数法显示；
                    // 14 位以内保持数值(可参与计算)，超过则降级为文本保原样，杜绝科学计数法。
                    if (total as f64).abs() < 1e14 {
                        *current = CellValue::Number(total as f64);
                    } else {
                        *current = CellValue::Text(total.to_string());
                    }
                } else {
                    let sum = current.as_number().unwrap_or(0.0) + other.as_number().unwrap_or(0.0);
                    *current = CellValue::Number(sum);
                }
            }
            AggregateOp::UniqueJoin => {
                let mut values = current
                    .as_text()
                    .split(separator)
                    .filter(|value| !value.is_empty())
                    .map(str::to_owned)
                    .collect::<Vec<_>>();
                let value = other.as_text();
                if !value.is_empty() && !values.contains(&value) {
                    values.push(value);
                }
                *current = CellValue::Text(values.join(separator));
            }
            AggregateOp::TextJoin => {
                let value = other.as_text();
                if !value.is_empty() {
                    let first = current.as_text();
                    *current = CellValue::Text(if first.is_empty() {
                        value
                    } else {
                        format!("{first}{separator}{value}")
                    });
                }
            }
        }
    }
}

fn fill_empty(existing: &mut [CellValue], incoming: &[CellValue]) {
    for (current, other) in existing.iter_mut().zip(incoming) {
        if current.is_empty() && !other.is_empty() {
            *current = other.clone();
        }
    }
}

fn send_progress(current: u64, total: u64, label: String, emit: &dyn Fn(u64, u64, String)) {
    if current.is_multiple_of(1_000) || current == total {
        emit(current, total, format!("正在处理：{label}"));
    }
}

fn finish_sink(
    sink: XlsxSink,
    output: &Path,
    rows: u64,
    cancel: &AtomicBool,
) -> Result<Option<(u64, usize)>> {
    if cancel.load(Ordering::Relaxed) {
        return Ok(None);
    }
    let sheets = sink.sheet_count();
    sink.save(output)?;
    Ok(Some((rows, sheets)))
}

#[derive(Clone, Debug)]
enum CellValue {
    Empty,
    Text(String),
    Integer(i64),
    Number(f64),
    Boolean(bool),
    /// Excel 日期/时间，由源文件解析出的年月日时分秒组成。
    DateTime {
        year: u16,
        month: u8,
        day: u8,
        hour: u8,
        minute: u8,
        second: u8,
    },
}

impl CellValue {
    fn is_empty(&self) -> bool {
        matches!(self, Self::Empty) || matches!(self, Self::Text(value) if value.is_empty())
    }
    fn as_text(&self) -> String {
        match self {
            Self::Empty => String::new(),
            Self::Text(v) => v.clone(),
            Self::Integer(v) => v.to_string(),
            Self::Number(v) => v.to_string(),
            Self::Boolean(v) => v.to_string(),
            Self::DateTime {
                year,
                month,
                day,
                hour,
                minute,
                second,
            } if *hour == 0 && *minute == 0 && *second == 0 => {
                format!("{year:04}-{month:02}-{day:02}")
            }
            Self::DateTime {
                year,
                month,
                day,
                hour,
                minute,
                second,
            } => format!("{year:04}-{month:02}-{day:02} {hour:02}:{minute:02}:{second:02}"),
        }
    }
    fn as_number(&self) -> Option<f64> {
        match self {
            Self::Integer(v) => Some(*v as f64),
            Self::Number(v) => Some(*v),
            Self::Text(v) => v.replace(',', "").parse().ok(),
            _ => None,
        }
    }
    fn transformed(&self, operation: TransformOp) -> Self {
        if operation == TransformOp::None {
            self.clone()
        } else {
            Self::Text(operation.apply(&self.as_text()))
        }
    }
    fn from_calamine(value: &Data) -> Self {
        match value {
            Data::Empty => Self::Empty,
            Data::String(v) => Self::Text(v.clone()),
            Data::Int(v) => Self::Integer(*v),
            Data::Float(v) => Self::Number(*v),
            Data::Bool(v) => Self::Boolean(*v),
            Data::DateTime(v) => {
                // 时长格式（如 [hh]:mm:ss）保持数值文本。
                if v.is_duration() {
                    Self::Text(v.as_f64().to_string())
                } else {
                    let (year, month, day, hour, minute, second, _millis) = v.to_ymd_hms_milli();
                    Self::DateTime {
                        year,
                        month,
                        day,
                        hour,
                        minute,
                        second,
                    }
                }
            }
            Data::DateTimeIso(v) | Data::DurationIso(v) => Self::Text(v.clone()),
            Data::Error(v) => Self::Text(v.to_string()),
        }
    }
}

trait RowSink {
    fn write_row(&mut self, values: &[CellValue]) -> Result<()>;
    fn total_rows(&self) -> u64;
}

struct CallbackSink<F> {
    write_row: F,
    empty_as_null: bool,
    total_rows: u64,
}

impl<F> RowSink for CallbackSink<F>
where
    F: FnMut(Vec<Option<String>>) -> Result<()>,
{
    fn write_row(&mut self, values: &[CellValue]) -> Result<()> {
        let row = values
            .iter()
            .map(|value| match value {
                CellValue::Empty => None,
                CellValue::Text(text) if self.empty_as_null && text.is_empty() => None,
                other => Some(other.as_text()),
            })
            .collect();
        (self.write_row)(row)?;
        self.total_rows += 1;
        Ok(())
    }

    fn total_rows(&self) -> u64 {
        self.total_rows
    }
}

struct XlsxSink {
    workbook: Workbook,
    plan: OutputPlan,
    header_format: Format,
    current_sheet: usize,
    row_in_sheet: u32,
    max_data_rows: u32,
    total_rows: u64,
}

impl XlsxSink {
    fn new(plan: OutputPlan) -> Result<Self> {
        Self::new_with_limit(plan, XLSX_MAX_DATA_ROWS)
    }
    fn new_with_limit(plan: OutputPlan, max_data_rows: u32) -> Result<Self> {
        if max_data_rows == 0 || max_data_rows > XLSX_MAX_DATA_ROWS {
            return Err(anyhow!("无效的 Sheet 行数限制"));
        }
        let header_format = Format::new()
            .set_bold()
            .set_font_color(Color::White)
            .set_background_color(Color::RGB(0x2563EB))
            .set_align(FormatAlign::Center)
            .set_border(FormatBorder::Thin);
        let mut sink = Self {
            workbook: Workbook::new(),
            plan,
            header_format,
            current_sheet: 0,
            row_in_sheet: 0,
            max_data_rows,
            total_rows: 0,
        };
        sink.add_sheet()?;
        Ok(sink)
    }
    fn add_sheet(&mut self) -> Result<()> {
        let index = self.workbook.worksheets().len();
        let worksheet = self.workbook.add_worksheet_with_constant_memory();
        worksheet.set_name(format!("合并结果_{:03}", index + 1))?;
        worksheet.set_freeze_panes(1, 0)?;
        for (column, header) in self.plan.headers.iter().enumerate() {
            worksheet.write_with_format(0, column as u16, header, &self.header_format)?;
            worksheet.set_column_width(column as u16, suggested_width(header))?;
        }
        self.current_sheet = index;
        self.row_in_sheet = 0;
        Ok(())
    }
    fn write_row(&mut self, values: &[CellValue]) -> Result<()> {
        if self.row_in_sheet >= self.max_data_rows {
            self.add_sheet()?;
        }
        let excel_row = self.row_in_sheet + 1;
        let worksheet = self.workbook.worksheet_from_index(self.current_sheet)?;
        for (column, value) in values.iter().enumerate() {
            match value {
                CellValue::Empty => {}
                CellValue::Text(v) => {
                    worksheet.write_string(excel_row, column as u16, v)?;
                }
                CellValue::Integer(v) => {
                    // f64 只能精确表示 2^53 以内的整数；超出的整数值（如 18 位
                    // 编号）以文本写出，避免尾数被静默截断。
                    if (*v as f64) as i64 == *v {
                        worksheet.write_number(excel_row, column as u16, *v as f64)?;
                    } else {
                        worksheet.write_string(excel_row, column as u16, v.to_string())?;
                    }
                }
                CellValue::Number(v) => {
                    worksheet.write_number(excel_row, column as u16, *v)?;
                }
                CellValue::Boolean(v) => {
                    worksheet.write_boolean(excel_row, column as u16, *v)?;
                }
                CellValue::DateTime {
                    year,
                    month,
                    day,
                    hour,
                    minute,
                    second,
                } => {
                    let datetime = ExcelDateTime::from_ymd(*year, *month, *day)?.and_hms(
                        *hour as u16,
                        *minute,
                        *second,
                    )?;
                    let format = if *hour == 0 && *minute == 0 && *second == 0 {
                        Format::new().set_num_format("yyyy-mm-dd")
                    } else {
                        Format::new().set_num_format("yyyy-mm-dd hh:mm:ss")
                    };
                    worksheet.write_datetime_with_format(
                        excel_row,
                        column as u16,
                        &datetime,
                        &format,
                    )?;
                }
            }
        }
        self.row_in_sheet += 1;
        self.total_rows += 1;
        Ok(())
    }
    fn sheet_count(&self) -> usize {
        self.current_sheet + 1
    }
    fn save(mut self, output: &Path) -> Result<()> {
        self.workbook
            .save(output)
            .with_context(|| format!("无法写入 {}，请确认文件未被 Excel 占用", output.display()))
    }
}

impl RowSink for XlsxSink {
    fn write_row(&mut self, values: &[CellValue]) -> Result<()> {
        XlsxSink::write_row(self, values)
    }

    fn total_rows(&self) -> u64 {
        self.total_rows
    }
}

fn suggested_width(header: &str) -> f64 {
    let units: usize = header
        .chars()
        .map(|ch| if ch.is_ascii() { 1 } else { 2 })
        .sum();
    (units as f64 + 4.0).clamp(12.0, 32.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn data_capacity_reserves_header() {
        assert_eq!(XLSX_MAX_DATA_ROWS, 1_048_575);
    }
    #[test]
    fn chinese_headers_get_wider_columns() {
        assert_eq!(suggested_width("订单编号"), 12.0);
        assert!(suggested_width("a very long english heading") > 20.0);
    }
    #[test]
    fn sink_splits_rows_and_repeats_headers() {
        let plan = OutputPlan {
            headers: vec!["姓名".to_owned()],
            source_file_column: None,
            source_sheet_column: None,
        };
        let mut sink = XlsxSink::new_with_limit(plan, 2).unwrap();
        for value in ["甲", "乙", "丙"] {
            sink.write_row(&[CellValue::Text(value.to_owned())])
                .unwrap();
        }
        assert_eq!(sink.sheet_count(), 2);
        let output =
            std::env::temp_dir().join(format!("sheet-merge-test-{}.xlsx", std::process::id()));
        sink.save(&output).unwrap();
        let mut workbook = open_workbook_auto(&output).unwrap();
        assert_eq!(workbook.sheet_names().len(), 2);
        assert_eq!(
            workbook.worksheet_range("合并结果_001").unwrap().height(),
            3
        );
        let _ = std::fs::remove_file(output);
    }
    #[test]
    fn transformations_are_applied() {
        assert_eq!(
            CellValue::Text(" Ab ".into())
                .transformed(TransformOp::Lowercase)
                .as_text(),
            "ab"
        );
    }
    #[test]
    fn manual_mapping_can_reuse_a_source_value() {
        let headers = vec!["原始值".to_owned()];
        let mut mappings = crate::model::make_default_mappings(&headers);
        mappings.push(crate::model::ColumnMapping {
            source_index: 0,
            source_name: "原始值".to_owned(),
            target_name: "复制值".to_owned(),
            enabled: true,
            transform: TransformOp::None,
            aggregate: AggregateOp::First,
        });
        let table = SourceTable {
            path: PathBuf::from("customers.csv"),
            sheet_name: "CSV".to_owned(),
            kind: SourceKind::Csv { delimiter: b',' },
            header_row: 1,
            header_rows: 1,
            suggested_header_row: 1,
            headers,
            estimated_rows: 1,
            enabled: true,
            mappings,
        };
        let options = MergeOptions {
            mode: MergeMode::Manual,
            ..MergeOptions::default()
        };
        let plan = build_output_plan(std::slice::from_ref(&table), &options);
        let mapping = build_row_mapping(&table, &plan, options.mode);
        let row = mapped_row(vec![CellValue::Text("001".to_owned())], &mapping, &plan);
        assert_eq!(
            row.iter().map(CellValue::as_text).collect::<Vec<_>>(),
            ["001", "001"]
        );
    }
    #[test]
    fn cancelled_merge_does_not_create_partial_output() {
        let headers = vec!["id".to_owned()];
        let table = SourceTable {
            path: PathBuf::from("does-not-need-to-exist.csv"),
            sheet_name: "CSV".to_owned(),
            kind: SourceKind::Csv { delimiter: b',' },
            header_row: 1,
            header_rows: 1,
            suggested_header_row: 1,
            mappings: crate::model::make_default_mappings(&headers),
            headers,
            estimated_rows: 1,
            enabled: true,
        };
        let output = std::env::temp_dir().join(format!("cancelled-{}.xlsx", std::process::id()));
        let cancel = AtomicBool::new(true);
        let result = merge_tables(
            &[table],
            &MergeOptions::default(),
            &output,
            &|_, _, _| {},
            &cancel,
        )
        .unwrap();
        assert!(result.is_none());
        assert!(!output.exists());
    }
    #[test]
    fn excel_datetime_components_format_and_round_trip() {
        let date = CellValue::DateTime {
            year: 2024,
            month: 1,
            day: 1,
            hour: 0,
            minute: 0,
            second: 0,
        };
        assert_eq!(date.as_text(), "2024-01-01");
        let datetime = CellValue::DateTime {
            year: 2024,
            month: 1,
            day: 1,
            hour: 6,
            minute: 30,
            second: 0,
        };
        assert_eq!(datetime.as_text(), "2024-01-01 06:30:00");

        let plan = OutputPlan {
            headers: vec!["日期".to_owned()],
            source_file_column: None,
            source_sheet_column: None,
        };
        let mut sink = XlsxSink::new_with_limit(plan, 10).unwrap();
        sink.write_row(&[date]).unwrap();
        let output = std::env::temp_dir().join(format!("datetime-{}.xlsx", std::process::id()));
        sink.save(&output).unwrap();
        let mut workbook = open_workbook_auto(&output).unwrap();
        let range = workbook.worksheet_range("合并结果_001").unwrap();
        match range.get_value((1, 0)) {
            Some(Data::DateTime(value)) => assert_eq!(value.as_f64(), 45_292.0),
            other => panic!("expected DateTime cell, got {other:?}"),
        }
        let _ = std::fs::remove_file(output);
    }
    #[test]
    fn large_integers_fall_back_to_text() {
        let plan = OutputPlan {
            headers: vec!["id".to_owned()],
            source_file_column: None,
            source_sheet_column: None,
        };
        let mut sink = XlsxSink::new_with_limit(plan, 10).unwrap();
        let huge = 9_007_199_254_740_993_i64; // 2^53 + 1，f64 无法精确表示
        sink.write_row(&[CellValue::Integer(huge)]).unwrap();
        let output = std::env::temp_dir().join(format!("big-int-{}.xlsx", std::process::id()));
        sink.save(&output).unwrap();
        let mut workbook = open_workbook_auto(&output).unwrap();
        let range = workbook.worksheet_range("合并结果_001").unwrap();
        match range.get_value((1, 0)) {
            Some(Data::String(value)) => assert_eq!(value, "9007199254740993"),
            other => panic!("expected text fallback, got {other:?}"),
        }
        let _ = std::fs::remove_file(output);
    }

    #[test]
    fn database_row_stream_keeps_manual_headers_and_nulls() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("customers.csv");
        std::fs::write(&path, "id,name\n001,张三\n002,\n").unwrap();
        let headers = vec!["id".to_owned(), "name".to_owned()];
        let mut mappings = crate::model::make_default_mappings(&headers);
        mappings[0].target_name = "客户编号".to_owned();
        mappings[1].target_name = "客户姓名".to_owned();
        let table = SourceTable {
            path,
            sheet_name: "CSV".to_owned(),
            kind: SourceKind::Csv { delimiter: b',' },
            header_row: 1,
            header_rows: 1,
            suggested_header_row: 1,
            headers,
            estimated_rows: 2,
            enabled: true,
            mappings,
        };
        let options = MergeOptions {
            mode: MergeMode::Manual,
            ..MergeOptions::default()
        };
        let mut rows = Vec::new();
        let result = stream_merged_rows(
            &[table],
            &options,
            true,
            &|_, _, _| {},
            &AtomicBool::new(false),
            |row| {
                rows.push(row);
                Ok(())
            },
        )
        .unwrap()
        .unwrap();
        assert_eq!(result.0.headers, ["客户编号", "客户姓名"]);
        assert_eq!(rows[0], [Some("001".to_owned()), Some("张三".to_owned())]);
        assert_eq!(rows[1], [Some("002".to_owned()), None]);
    }
}
