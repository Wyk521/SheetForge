use super::{csv::scan_csv, preserve_mappings, scan_file, workbook::scan_workbook_sheet};
use crate::model::{SourceKind, SourceTable};
use serde::Serialize;
use std::{
    collections::VecDeque,
    path::PathBuf,
    sync::{Arc, Mutex},
};
use tauri::{AppHandle, Emitter};
#[derive(Clone, Serialize)]
pub struct ScanProgressDto {
    pub done: usize,
    pub total: usize,
    pub name: String,
}

#[derive(Clone, Serialize)]
pub struct ScanFinishedDto {
    pub tables: Vec<SourceTable>,
    pub warnings: Vec<String>,
}

#[derive(Clone, Serialize)]
pub struct ScanFailedDto {
    pub message: String,
}

#[derive(Clone, Serialize)]
pub struct TableReloadedDto {
    pub index: usize,
    pub table: SourceTable,
}

#[derive(Clone, Serialize)]
pub struct TableReloadFailedDto {
    pub index: usize,
    pub message: String,
}

#[derive(Clone, Serialize)]
pub struct TablesReloadedDto {
    pub tables: Vec<TableReloadedDto>,
    pub failures: Vec<String>,
}

pub fn spawn_scan(paths: Vec<PathBuf>, app: AppHandle) {
    std::thread::spawn(move || {
        let mut paths = paths;
        paths.sort();
        paths.dedup();
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
            let _ = app.emit(
                "scan-progress",
                ScanProgressDto {
                    done: index + 1,
                    total,
                    name,
                },
            );
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
            let _ = app.emit(
                "scan-failed",
                ScanFailedDto {
                    message: warnings.join("\n"),
                },
            );
        } else {
            let _ = app.emit("scan-finished", ScanFinishedDto { tables, warnings });
        }
    });
}

/// 扫描单个文件，返回数据表和该文件的警告（如可疑编码）。
pub fn spawn_table_reload(
    index: usize,
    source: SourceTable,
    header_row: usize,
    header_rows: usize,
    app: AppHandle,
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
                let _ = app.emit("table-reloaded", TableReloadedDto { index, table });
            }
            Err(error) => {
                let _ = app.emit(
                    "table-reload-failed",
                    TableReloadFailedDto {
                        index,
                        message: format!("{}：{error:#}", source.display_name()),
                    },
                );
            }
        }
    });
}

pub fn spawn_group_reload(
    sources: Vec<(usize, SourceTable)>,
    header_row: usize,
    header_rows: usize,
    app: AppHandle,
) {
    std::thread::spawn(move || {
        let mut tables = Vec::with_capacity(sources.len());
        let mut failures = Vec::new();
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
                    tables.push(TableReloadedDto { index, table });
                }
                Err(error) => {
                    // 单表失败不影响其他表：记录失败并继续，最后统一提交成功部分，
                    // 避免「统一表头」时一个坏 Sheet 导致整批刷新结果丢失。
                    // 失败随最终事件一起返回，不能单独发事件，否则前端会在
                    // 其余 Sheet 仍在刷新时提前解除忙碌。
                    failures.push(format!("{}：{error:#}", source.display_name()));
                }
            }
        }
        let _ = app.emit("tables-reloaded", TablesReloadedDto { tables, failures });
    });
}
