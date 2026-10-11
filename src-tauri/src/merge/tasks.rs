use super::{is_cancelled, merge_tables};
use crate::inspect::{preflight_for_destination, CheckIssue};
use crate::model::{MergeOptions, SourceTable};
use serde::Serialize;
use std::{
    path::PathBuf,
    sync::{atomic::AtomicBool, Arc},
};
use tauri::{AppHandle, Emitter};
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
    pub truncated_cells: u64,
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
            Ok(Some(summary)) => {
                let _ = app.emit(
                    "merge-finished",
                    MergeFinishedDto {
                        output,
                        rows: summary.rows,
                        sheets: summary.sheets,
                        truncated_cells: summary.truncated_cells,
                    },
                );
            }
            Ok(None) => {
                let _ = app.emit("merge-cancelled", ());
            }
            Err(error) => {
                if is_cancelled(&error) {
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
