use crate::config::{self, MergeScheme};
use crate::database::{self, DatabaseImportRequest, DatabaseProfilesDto};
use crate::inspect::{
    preview_merged as inspect_preview_merged, preview_source as inspect_preview_source,
    MergedPreview, PreviewTable,
};
use crate::merge::{spawn_merge, spawn_preflight};
use crate::model::{build_output_plan, common_header_keys, MergeOptions, SourceTable};
use crate::pg_import::config::ConnectionProfile;
use crate::pg_import::postgres::connection::ConnectionInfo;
use crate::scan::{
    collect_folder, spawn_group_reload, spawn_scan, spawn_table_reload, supported_file,
};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use tauri::{AppHandle, State};

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
pub fn path_exists(path: String) -> bool {
    PathBuf::from(&path).exists()
}

#[tauri::command]
pub fn get_log_path() -> String {
    config::log_path().display().to_string()
}

#[tauri::command]
pub fn get_database_profiles() -> Result<DatabaseProfilesDto, String> {
    database::get_profiles().map_err(|error| format!("{error:#}"))
}

#[tauri::command]
pub fn save_database_profile(
    name: String,
    profile: ConnectionProfile,
    password: Option<String>,
    remember_password: bool,
) -> Result<Option<String>, String> {
    database::save_profile(name, profile, password, remember_password)
        .map_err(|error| format!("{error:#}"))
}

#[tauri::command]
pub fn delete_database_profile(name: String) -> Result<(), String> {
    database::delete_profile(name).map_err(|error| format!("{error:#}"))
}

#[tauri::command]
pub async fn test_database_connection(
    profile_name: Option<String>,
    profile: ConnectionProfile,
    password: Option<String>,
) -> Result<ConnectionInfo, String> {
    database::test_connection(profile_name, profile, password)
        .await
        .map_err(|error| format!("{error:#}"))
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
) -> Result<MergedPreview, String> {
    inspect_preview_merged(&tables, &options, limit).map_err(|error| format!("{error:#}"))
}

#[tauri::command]
pub fn run_preflight(
    app: AppHandle,
    tables: Vec<SourceTable>,
    options: MergeOptions,
    continues_merge: bool,
    destination: Option<String>,
) {
    spawn_preflight(
        tables,
        options,
        continues_merge,
        destination.as_deref() == Some("postgres"),
        app,
    );
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
pub fn start_database_import(
    app: AppHandle,
    state: State<'_, Mutex<AppState>>,
    tables: Vec<SourceTable>,
    options: MergeOptions,
    request: DatabaseImportRequest,
) -> Result<(), String> {
    if request.profile_name.trim().is_empty() {
        return Err("请先选择数据库连接".to_owned());
    }
    let cancel = {
        let mut app_state = state.lock().map_err(|error| error.to_string())?;
        app_state.cancel = Arc::new(AtomicBool::new(false));
        app_state.cancel.clone()
    };
    config::append_log(&format!(
        "开始导入数据库：{} 个数据表 -> {}.{}",
        tables.iter().filter(|table| table.enabled).count(),
        request.schema,
        request.table
    ));
    database::spawn_database_import(tables, options, request, app, cancel);
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
