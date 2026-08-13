use crate::config::{self, MergeScheme};
use crate::inspect::{
    preview_merged as inspect_preview_merged, preview_source as inspect_preview_source,
    PreviewTable,
};
use crate::merge::{spawn_merge, spawn_preflight};
use crate::model::{build_output_plan, common_header_keys, MergeOptions, SourceTable};
use crate::scan::{
    collect_folder, spawn_group_reload, spawn_scan, spawn_table_reload, supported_file,
};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use tauri::{AppHandle, Emitter, State};

pub struct AppState {
    pub cancel: Arc<AtomicBool>,
}

#[derive(Clone, Serialize)]
pub struct StateSnapshot {
    pub settings: config::AppSettings,
}

#[derive(Clone, Serialize)]
pub struct PlanSnapshot {
    pub headers: Vec<String>,
    pub common_keys: Vec<String>,
}

#[derive(Clone, Serialize)]
pub struct UpdateResultDto {
    pub version: String,
    pub url: String,
    pub newer: bool,
}

#[derive(Clone, Serialize)]
pub struct UpdateFailedDto {
    pub message: String,
}

#[derive(Clone, Deserialize)]
pub struct ReloadItem {
    pub index: usize,
    pub table: SourceTable,
}

#[tauri::command]
pub fn get_state() -> StateSnapshot {
    StateSnapshot {
        settings: config::load_settings(),
    }
}

#[tauri::command]
pub fn get_plan(tables: Vec<SourceTable>, options: MergeOptions) -> PlanSnapshot {
    PlanSnapshot {
        headers: build_output_plan(&tables, &options).headers,
        common_keys: common_header_keys(&tables).into_iter().collect(),
    }
}

#[tauri::command]
pub fn get_suggestions(tables: Vec<SourceTable>) -> std::collections::HashMap<String, String> {
    crate::inspect::mapping_suggestions(&tables)
}

#[tauri::command]
pub fn path_exists(path: String) -> bool {
    PathBuf::from(&path).exists()
}

#[tauri::command]
pub fn get_log_path() -> String {
    config::log_path().display().to_string()
}

#[tauri::command]
pub fn save_text_file(path: String, content: String) -> Result<(), String> {
    std::fs::write(PathBuf::from(&path), content).map_err(|error| format!("{error}"))
}

#[tauri::command]
pub fn scan_folder(app: AppHandle, path: String) -> Result<(), String> {
    let folder = PathBuf::from(&path);
    let paths = collect_folder(&folder);
    if paths.is_empty() {
        return Err("没有找到支持的表格文件".to_owned());
    }
    let mut settings = config::load_settings();
    config::remember(&mut settings.recent_folders, path);
    let _ = config::save_settings(&settings);
    spawn_scan(paths, app);
    Ok(())
}

#[tauri::command]
pub fn scan_files(app: AppHandle, paths: Vec<String>) -> Result<(), String> {
    let mut paths: Vec<PathBuf> = paths
        .into_iter()
        .map(PathBuf::from)
        .filter(|path| supported_file(path))
        .collect();
    paths.sort();
    paths.dedup();
    if paths.is_empty() {
        return Err("没有找到支持的表格文件".to_owned());
    }
    let mut settings = config::load_settings();
    if let Some(folder) = paths.first().and_then(|path| path.parent()) {
        config::remember(&mut settings.recent_folders, folder.display().to_string());
    }
    let _ = config::save_settings(&settings);
    spawn_scan(paths, app);
    Ok(())
}

#[tauri::command]
pub fn reload_table(
    app: AppHandle,
    index: usize,
    table: SourceTable,
    header_row: usize,
    header_rows: usize,
) {
    spawn_table_reload(index, table, header_row, header_rows, app);
}

#[tauri::command]
pub fn reload_group(
    app: AppHandle,
    sources: Vec<ReloadItem>,
    header_row: usize,
    header_rows: usize,
) {
    let sources = sources
        .into_iter()
        .map(|item| (item.index, item.table))
        .collect();
    spawn_group_reload(sources, header_row, header_rows, app);
}

#[tauri::command]
pub fn preview_source(table: SourceTable, limit: usize) -> Result<PreviewTable, String> {
    inspect_preview_source(&table, limit).map_err(|error| format!("{error:#}"))
}

#[tauri::command]
pub fn preview_merged(
    tables: Vec<SourceTable>,
    options: MergeOptions,
    limit: usize,
) -> Result<PreviewTable, String> {
    inspect_preview_merged(&tables, &options, limit).map_err(|error| format!("{error:#}"))
}

#[tauri::command]
pub fn run_preflight(
    app: AppHandle,
    tables: Vec<SourceTable>,
    options: MergeOptions,
    continues_merge: bool,
) {
    spawn_preflight(tables, options, continues_merge, app);
}

#[tauri::command]
pub fn start_merge(
    app: AppHandle,
    state: State<'_, Mutex<AppState>>,
    tables: Vec<SourceTable>,
    options: MergeOptions,
    output: String,
) -> Result<(), String> {
    let output = PathBuf::from(output.trim());
    if output.as_os_str().is_empty() {
        return Err("请先选择输出文件".to_owned());
    }
    if !output
        .extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| ext.eq_ignore_ascii_case("xlsx"))
    {
        return Err("输出文件必须使用 .xlsx 扩展名".to_owned());
    }
    let cancel = {
        let mut app_state = state.lock().map_err(|error| error.to_string())?;
        app_state.cancel = Arc::new(AtomicBool::new(false));
        app_state.cancel.clone()
    };
    config::append_log(&format!(
        "开始合并：{} 个数据表 -> {}",
        tables.iter().filter(|table| table.enabled).count(),
        output.display()
    ));
    spawn_merge(tables, options, output, app, cancel);
    Ok(())
}

#[tauri::command]
pub fn cancel_merge(state: State<'_, Mutex<AppState>>) {
    if let Ok(app_state) = state.lock() {
        app_state.cancel.store(true, Ordering::Relaxed);
    }
}

#[tauri::command]
pub fn save_scheme(
    path: String,
    tables: Vec<SourceTable>,
    options: MergeOptions,
) -> Result<(), String> {
    let path = PathBuf::from(&path);
    let scheme = MergeScheme {
        format_version: 1,
        name: path
            .file_stem()
            .map(|value| value.to_string_lossy().into_owned())
            .unwrap_or_default(),
        tables,
        options,
    };
    config::save_scheme(&path, &scheme).map_err(|error| format!("{error:#}"))?;
    let mut settings = config::load_settings();
    config::remember(&mut settings.recent_schemes, path.display().to_string());
    let _ = config::save_settings(&settings);
    Ok(())
}

#[tauri::command]
pub fn open_scheme(path: String) -> Result<MergeScheme, String> {
    let path = PathBuf::from(&path);
    let scheme = config::load_scheme(&path).map_err(|error| format!("{error:#}"))?;
    let mut settings = config::load_settings();
    config::remember(&mut settings.recent_schemes, path.display().to_string());
    let _ = config::save_settings(&settings);
    Ok(scheme)
}

#[tauri::command]
pub fn check_update(app: AppHandle) {
    std::thread::spawn(move || {
        let result = (|| -> anyhow::Result<(String, String)> {
            let user_agent = format!("SheetForge/{}", env!("CARGO_PKG_VERSION"));
            let body = ureq::get("https://api.github.com/repos/Wyk521/SheetForge/releases/latest")
                .header("User-Agent", &user_agent)
                .call()?
                .body_mut()
                .read_to_string()?;
            let value: serde_json::Value = serde_json::from_str(&body)?;
            Ok((
                value["tag_name"].as_str().unwrap_or_default().to_owned(),
                value["html_url"].as_str().unwrap_or_default().to_owned(),
            ))
        })();
        match result {
            Ok((version, url)) => {
                let newer = is_newer_version(&version, env!("CARGO_PKG_VERSION"));
                let _ = app.emit(
                    "update-result",
                    UpdateResultDto {
                        version,
                        url,
                        newer,
                    },
                );
            }
            Err(error) => {
                let _ = app.emit(
                    "update-failed",
                    UpdateFailedDto {
                        message: format!("{error:#}"),
                    },
                );
            }
        }
    });
}

/// 解析 "v1.2.3" 形式的版本号；解析失败返回 None。
fn version_tuple(version: &str) -> Option<(u64, u64, u64)> {
    let trimmed = version.trim().trim_start_matches('v');
    let mut parts = trimmed.split('.');
    let major = parts.next()?.parse().ok()?;
    let minor = parts.next()?.parse().ok()?;
    let patch = parts.next()?.parse().ok()?;
    Some((major, minor, patch))
}

/// 语义化比较远程版本是否比当前版本新（字符串比较会把 0.10 误判为旧版本）。
fn is_newer_version(remote: &str, current: &str) -> bool {
    match (version_tuple(remote), version_tuple(current)) {
        (Some(remote), Some(current)) => remote > current,
        (Some(_), None) => true,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_comparison_is_semantic() {
        assert!(is_newer_version("v0.10.0", "0.3.1"));
        assert!(!is_newer_version("v0.3.1", "0.3.1"));
        assert!(!is_newer_version("v0.2.9", "0.3.1"));
        assert!(is_newer_version("1.0.0", "0.9.9"));
        assert!(!is_newer_version("junk", "0.3.1"));
    }
}
