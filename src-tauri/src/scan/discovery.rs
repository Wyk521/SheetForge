use std::path::{Path, PathBuf};
use walkdir::WalkDir;
pub fn supported_file(path: &Path) -> bool {
    if path
        .file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.starts_with("~$"))
    {
        return false;
    }
    matches!(
        path.extension()
            .and_then(|ext| ext.to_str())
            .map(|ext| ext.to_ascii_lowercase())
            .as_deref(),
        Some("csv" | "tsv" | "xlsx" | "xlsm" | "xls" | "xlsb" | "ods")
    )
}

pub fn collect_folder(folder: &Path) -> Vec<PathBuf> {
    let mut paths: Vec<PathBuf> = WalkDir::new(folder)
        .follow_links(false)
        .into_iter()
        .filter_map(Result::ok)
        .filter(|entry| entry.file_type().is_file() && supported_file(entry.path()))
        .map(|entry| entry.into_path())
        .collect();
    paths.sort_by_key(|path| path.to_string_lossy().to_lowercase());
    paths
}
