//! Shared streaming merge pipeline. Output adapters consume the same mapped cells.
mod row;
mod tasks;
mod xlsx;

use crate::model::{build_output_plan, MergeOptions, OutputPlan, SourceKind, SourceTable};
use crate::scan::{
    for_each_csv_row, for_each_xlsx_row, is_markup_workbook, is_xlsx_path, open_detected_workbook,
    open_xlsx_workbook, read_markup_workbook,
};
use anyhow::{anyhow, Context, Result};
use calamine::{open_workbook_auto, Reader};
use row::{build_row_mapping, is_blank_source_row, mapped_row, passes_filter, row_key, RowMapping};
pub(crate) use row::{preview_mapped_row, CellValue};
use std::{
    collections::HashSet,
    path::{Path, PathBuf},
    sync::atomic::{AtomicBool, Ordering},
};
#[allow(unused_imports)]
pub use tasks::{
    spawn_merge, spawn_preflight, MergeFailedDto, MergeFinishedDto, MergeProgressDto,
    PreflightDoneDto,
};
use xlsx::{finish_sink, XlsxSink};
#[allow(unused_imports)]
pub use xlsx::{XLSX_MAX_DATA_ROWS, XLSX_MAX_ROWS};

/// 用户主动取消合并时的类型化错误，用于与真正的失败区分开。
#[derive(Debug)]
struct MergeCancelled;

impl std::fmt::Display for MergeCancelled {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "合并已取消")
    }
}

impl std::error::Error for MergeCancelled {}

/// 判断错误链是否来自用户取消，供 XLSX 与数据库两条导出路径共用。
pub(crate) fn is_cancelled(error: &anyhow::Error) -> bool {
    error.downcast_ref::<MergeCancelled>().is_some()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct MergeSummary {
    pub rows: u64,
    pub sheets: usize,
    /// 超过 XLSX 单元格 32,767 字符上限而被截断的单元格数。
    pub truncated_cells: u64,
}

pub(crate) fn merge_tables(
    tables: &[SourceTable],
    options: &MergeOptions,
    output: &Path,
    emit: &dyn Fn(u64, u64, String),
    cancel: &AtomicBool,
) -> Result<Option<MergeSummary>> {
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

/// 将 SheetForge 已应用表头修改、交集和清洗后的最终行流交给调用方。
/// PostgreSQL 适配层使用此入口，确保数据库与 XLSX 得到完全相同的业务数据。
#[cfg(test)]
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
    let mut write_row = write_row;
    stream_merged_cells(tables, options, emit, cancel, |values| {
        let row = values
            .iter()
            .map(|value| match value {
                CellValue::Empty => None,
                CellValue::Text(text) if empty_as_null && text.is_empty() => None,
                other => Some(other.as_text()),
            })
            .collect();
        write_row(row)
    })
}

/// 将最终合并行直接交给调用方，避免数据库导入先把每个单元格复制成 String。
pub(crate) fn stream_merged_cells<F>(
    tables: &[SourceTable],
    options: &MergeOptions,
    emit: &dyn Fn(u64, u64, String),
    cancel: &AtomicBool,
    write_row: F,
) -> Result<Option<(OutputPlan, u64)>>
where
    F: FnMut(&[CellValue]) -> Result<()>,
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
    let mut sink = CellCallbackSink { write_row };
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
    merge_appended(tables, &row_mappings, options, plan, sink, emit, cancel)
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
                    match open_xlsx_workbook(&table.path) {
                        Ok(mut workbook) => {
                            for &(sheet_index, sheet) in &same_file {
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
                        }
                        Err(primary_error) => merge_fallback_workbook(
                            &table.path,
                            &same_file,
                            row_mappings,
                            options,
                            plan,
                            &mut dedup,
                            sink,
                            &mut current,
                            total,
                            emit,
                            cancel,
                        )
                        .with_context(|| format!("常规 XLSX 读取失败：{primary_error:#}"))?,
                    }
                } else {
                    match open_workbook_auto(&table.path) {
                        Ok(mut workbook) => {
                            for &(sheet_index, sheet) in &same_file {
                                let range =
                                    workbook.worksheet_range(&sheet.sheet_name).with_context(
                                        || format!("无法读取 {}", sheet.display_name()),
                                    )?;
                                for row in
                                    range.rows().skip(sheet.header_row + sheet.header_rows - 1)
                                {
                                    consume_append_row(
                                        sheet,
                                        row.iter().map(CellValue::from_calamine).collect(),
                                        &row_mappings[sheet_index],
                                        options,
                                        plan,
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
                        Err(primary_error) => merge_fallback_workbook(
                            &table.path,
                            &same_file,
                            row_mappings,
                            options,
                            plan,
                            &mut dedup,
                            sink,
                            &mut current,
                            total,
                            emit,
                            cancel,
                        )
                        .with_context(|| format!("常规读取失败：{primary_error}"))?,
                    }
                }
            }
        }
    }
    Ok(Some(current))
}

#[allow(clippy::too_many_arguments)]
fn merge_fallback_workbook<S: RowSink>(
    path: &Path,
    sheets: &[(usize, &SourceTable)],
    row_mappings: &[RowMapping],
    options: &MergeOptions,
    plan: &OutputPlan,
    dedup: &mut HashSet<Vec<String>>,
    sink: &mut S,
    current: &mut u64,
    total: u64,
    emit: &dyn Fn(u64, u64, String),
    cancel: &AtomicBool,
) -> Result<()> {
    if is_markup_workbook(path) {
        let workbook =
            read_markup_workbook(path).with_context(|| format!("无法打开 {}", path.display()))?;
        for &(sheet_index, sheet) in sheets {
            let source = workbook
                .iter()
                .find(|candidate| candidate.name == sheet.sheet_name)
                .with_context(|| format!("无法读取 {}", sheet.display_name()))?;
            for row in source
                .rows
                .iter()
                .skip(sheet.header_row + sheet.header_rows - 1)
            {
                consume_append_row(
                    sheet,
                    row.iter().cloned().map(CellValue::from_text).collect(),
                    &row_mappings[sheet_index],
                    options,
                    plan,
                    dedup,
                    sink,
                    current,
                    total,
                    emit,
                    cancel,
                )?;
            }
        }
        return Ok(());
    }

    let mut workbook =
        open_detected_workbook(path).with_context(|| format!("无法打开 {}", path.display()))?;
    for &(sheet_index, sheet) in sheets {
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
                dedup,
                sink,
                current,
                total,
                emit,
                cancel,
            )?;
        }
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn consume_append_row<S: RowSink>(
    table: &SourceTable,
    values: Vec<CellValue>,
    row_mapping: &RowMapping,
    options: &MergeOptions,
    plan: &OutputPlan,
    dedup: &mut HashSet<Vec<String>>,
    sink: &mut S,
    current: &mut u64,
    total: u64,
    emit: &dyn Fn(u64, u64, String),
    cancel: &AtomicBool,
) -> Result<()> {
    if cancel.load(Ordering::Relaxed) {
        return Err(MergeCancelled.into());
    }
    if is_blank_source_row(table.kind, &values) {
        return Ok(());
    }
    let output_row = mapped_row(values, row_mapping, plan);
    if !passes_filter(&output_row, plan, options) {
        return Ok(());
    }
    if options.deduplicate {
        let key = row_key(&output_row);
        if !dedup.insert(key) {
            return Ok(());
        }
    }
    sink.write_row(&output_row)?;
    *current += 1;
    send_progress(*current, total, table.display_name(), emit);
    Ok(())
}

fn send_progress(current: u64, total: u64, label: String, emit: &dyn Fn(u64, u64, String)) {
    if current.is_multiple_of(1_000) || current == total {
        emit(current, total, format!("正在处理：{label}"));
    }
}

trait RowSink {
    fn write_row(&mut self, values: &[CellValue]) -> Result<()>;
}

struct CellCallbackSink<F> {
    write_row: F,
}

impl<F> RowSink for CellCallbackSink<F>
where
    F: FnMut(&[CellValue]) -> Result<()>,
{
    fn write_row(&mut self, values: &[CellValue]) -> Result<()> {
        (self.write_row)(values)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::xlsx::{suggested_width, XLSX_MAX_DATA_ROWS};
    use super::*;
    use crate::model::{MergeMode, TransformOp};
    use calamine::Data;
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
        std::fs::write(&path, "id,name\n,\n001,张三\n002,\n").unwrap();
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
            estimated_rows: 3,
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
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0], [Some("001".to_owned()), Some("张三".to_owned())]);
        assert_eq!(rows[1], [Some("002".to_owned()), None]);
    }
}
