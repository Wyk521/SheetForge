use crate::model::{MergeOptions, SourceTable};
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MergeScheme {
    pub format_version: u32,
    pub name: String,
    pub tables: Vec<SourceTable>,
    pub options: MergeOptions,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AppSettings {
    pub output_directory: String,
    pub recent_folders: Vec<String>,
    pub recent_schemes: Vec<String>,
    pub window_maximized: bool,
    pub check_updates: bool,
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            output_directory: String::new(),
            recent_folders: Vec::new(),
            recent_schemes: Vec::new(),
            window_maximized: false,
            check_updates: true,
        }
    }
}

pub fn save_scheme(path: &Path, scheme: &MergeScheme) -> Result<()> {
    let content = serde_json::to_string_pretty(scheme)?;
    fs::write(path, content).with_context(|| format!("无法保存方案 {}", path.display()))
}

pub fn load_scheme(path: &Path) -> Result<MergeScheme> {
    let content =
        fs::read_to_string(path).with_context(|| format!("无法读取方案 {}", path.display()))?;
    let scheme: MergeScheme =
        serde_json::from_str(&content).with_context(|| "方案文件格式不正确")?;
    if scheme.format_version > 1 {
        anyhow::bail!("该方案由更高版本的软件创建，当前版本无法读取");
    }
    Ok(scheme)
}

pub fn load_settings() -> AppSettings {
    fs::read_to_string(settings_path())
        .ok()
        .and_then(|content| serde_json::from_str(&content).ok())
        .unwrap_or_default()
}

pub fn save_settings(settings: &AppSettings) -> Result<()> {
    let path = settings_path();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, serde_json::to_string_pretty(settings)?)?;
    Ok(())
}

pub fn remember(items: &mut Vec<String>, value: String) {
    items.retain(|item| item != &value);
    items.insert(0, value);
    items.truncate(8);
}

pub fn append_log(message: &str) {
    let path = log_path();
    if let Some(parent) = path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    if let Ok(mut file) = OpenOptions::new().create(true).append(true).open(path) {
        let timestamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|value| value.as_secs())
            .unwrap_or(0);
        let _ = writeln!(file, "[{timestamp}] {message}");
    }
}

pub fn log_path() -> PathBuf {
    app_data_dir().join("merge.log")
}

fn settings_path() -> PathBuf {
    app_data_dir().join("settings.json")
}

fn app_data_dir() -> PathBuf {
    std::env::var_os("APPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir)
        .join("表格合并")
}
