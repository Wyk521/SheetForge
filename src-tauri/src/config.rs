use crate::model::{build_output_plan, MergeOptions, SourceTable};
use crate::scan::refresh_saved_table;
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
    let value: serde_json::Value =
        serde_json::from_str(&content).with_context(|| "方案文件格式不正确")?;
    if let Some(mode) = value
        .pointer("/options/mode")
        .and_then(|value| value.as_str())
    {
        if !matches!(mode, "Manual" | "Intersection") {
            anyhow::bail!("方案使用了已移除的合并方式“{mode}”，请重新扫描并创建方案");
        }
    }
    let mut scheme: MergeScheme =
        serde_json::from_value(value).with_context(|| "方案文件格式不正确")?;
    if scheme.format_version > 1 {
        anyhow::bail!("该方案由更高版本的软件创建，当前版本无法读取");
    }
    let saved_output_order = build_output_plan(&scheme.tables, &scheme.options).headers;
    let mut column_order_changed = false;
    for table in &mut scheme.tables {
        if table.header_row == 0 || table.header_rows == 0 || table.header_rows > 3 {
            anyhow::bail!(
                "方案中“{}”的表头设置无效（开始行需 ≥ 1，占用行数需为 1–3）",
                table.display_name()
            );
        }
        let refreshed = refresh_saved_table(table)
            .with_context(|| format!("重新读取方案中的“{}”失败", table.display_name()))?;
        column_order_changed |= table.headers != refreshed.headers;
        *table = refreshed;
    }
    if column_order_changed && scheme.options.output_order.is_empty() {
        scheme.options.output_order = saved_output_order;
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

pub fn app_data_dir() -> PathBuf {
    std::env::var_os("APPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir)
        .join("表格合并")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{make_default_mappings, SourceKind};

    fn scheme_with_header(header_row: usize, header_rows: usize) -> MergeScheme {
        let headers = vec!["姓名".to_owned()];
        MergeScheme {
            format_version: 1,
            name: "测试方案".to_owned(),
            tables: vec![SourceTable {
                path: PathBuf::from("a.csv"),
                sheet_name: "CSV".to_owned(),
                kind: SourceKind::Csv { delimiter: b',' },
                header_row,
                header_rows,
                suggested_header_row: 1,
                mappings: make_default_mappings(&headers),
                headers,
                estimated_rows: 0,
                enabled: true,
            }],
            options: MergeOptions::default(),
        }
    }

    #[test]
    fn valid_scheme_round_trips() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("ok.json");
        let mut scheme = scheme_with_header(1, 1);
        scheme.tables[0].path = directory.path().join("a.csv");
        std::fs::write(&scheme.tables[0].path, "姓名\n张三\n").unwrap();
        save_scheme(&path, &scheme).unwrap();
        assert!(load_scheme(&path).is_ok());
    }

    #[test]
    fn scheme_refreshes_changed_source_column_order() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("scheme.json");
        let csv = directory.path().join("a.csv");
        std::fs::write(&csv, "name,city\nAlice,Paris\n").unwrap();
        let mut scheme = scheme_with_header(1, 1);
        scheme.tables[0] = crate::scan::scan_file(&csv).unwrap().0.remove(0);
        scheme.tables[0].mappings[0].target_name = "姓名".to_owned();
        save_scheme(&path, &scheme).unwrap();

        std::fs::write(&csv, "city,name\nParis,Alice\n").unwrap();
        let restored = load_scheme(&path).unwrap();
        assert_eq!(restored.tables[0].headers, vec!["city", "name"]);
        assert_eq!(restored.tables[0].mappings[1].source_index, 1);
        assert_eq!(restored.tables[0].mappings[1].target_name, "姓名");
        assert_eq!(restored.options.output_order, vec!["姓名", "city"]);
    }

    #[test]
    fn scheme_rejects_changed_source_headers() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("scheme.json");
        let csv = directory.path().join("a.csv");
        std::fs::write(&csv, "name,city\nAlice,Paris\n").unwrap();
        let mut scheme = scheme_with_header(1, 1);
        scheme.tables[0] = crate::scan::scan_file(&csv).unwrap().0.remove(0);
        save_scheme(&path, &scheme).unwrap();

        std::fs::write(&csv, "name,country\nAlice,France\n").unwrap();
        assert!(load_scheme(&path)
            .unwrap_err()
            .to_string()
            .contains("重新读取方案"));
    }

    #[test]
    fn scheme_rejects_removed_merge_mode_with_clear_message() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("old.json");
        let mut value = serde_json::to_value(scheme_with_header(1, 1)).unwrap();
        value["options"]["mode"] = serde_json::json!("Join");
        std::fs::write(&path, serde_json::to_string(&value).unwrap()).unwrap();
        let error = load_scheme(&path).unwrap_err().to_string();
        assert!(error.contains("已移除的合并方式"), "{error}");
    }

    #[test]
    fn invalid_header_settings_are_rejected() {
        let directory = tempfile::tempdir().unwrap();
        for (header_row, header_rows) in [(0, 1), (1, 0), (1, 4)] {
            let path = directory
                .path()
                .join(format!("{header_row}-{header_rows}.json"));
            save_scheme(&path, &scheme_with_header(header_row, header_rows)).unwrap();
            let error = load_scheme(&path).unwrap_err().to_string();
            assert!(error.contains("表头设置无效"), "{error}");
        }
    }
}
