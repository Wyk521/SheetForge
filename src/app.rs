use crate::config::{
    append_log, load_scheme, load_settings, log_path, remember, save_scheme, save_settings,
    AppSettings, MergeScheme,
};
use crate::inspect::{
    mapping_suggestions, preview_merged, preview_source, CheckIssue, IssueLevel, PreviewTable,
};
use crate::merge::{spawn_merge, spawn_preflight, MergeEvent, XLSX_MAX_DATA_ROWS};
use crate::model::{
    build_output_plan, common_header_keys, header_key, AggregateOp, JoinKind, MergeMode,
    MergeOptions, SourceTable, TransformOp,
};
use crate::scan::{
    collect_folder, spawn_group_reload, spawn_scan, spawn_table_reload, supported_file, ScanEvent,
};
use crate::{AppWindow, CheckRow, ColumnRow, MappingRow, PreviewRow, SourceRow};
use rfd::FileDialog;
use slint::{ComponentHandle, DataTransfer, ModelRc, SharedString, Timer, TimerMode, VecModel};
use std::cell::RefCell;
use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver};
use std::sync::Arc;
use std::time::Duration;

enum AppState {
    Ready,
    Scanning,
    Checking,
    Merging,
    Done {
        output: PathBuf,
        rows: u64,
        sheets: usize,
    },
    Error(String),
}

enum UpdateEvent {
    Result { version: String, url: String },
    Failed(String),
}

struct MergeApp {
    sources: Vec<SourceTable>,
    input_label: String,
    output_path: String,
    options: MergeOptions,
    hide_common_mappings: bool,
    mismatch_only: bool,
    source_search: String,
    mapping_search: String,
    selected_mapping_table: usize,
    collapsed_groups: HashSet<String>,
    preview: Option<PreviewTable>,
    preview_title: String,
    check_issues: Vec<CheckIssue>,
    scan_rx: Option<Receiver<ScanEvent>>,
    merge_rx: Option<Receiver<MergeEvent>>,
    update_rx: Option<Receiver<UpdateEvent>>,
    update_text: String,
    update_url: Option<String>,
    cancel: Arc<AtomicBool>,
    state: AppState,
    /// preflight 通过后是否继续进入合并流程（由「开始合并」触发时为 true，
    /// 由「检查报告」触发时为 false）。
    preflight_continues: bool,
    /// 是否执行过合并前检查（用于区分「尚未检查」和「检查通过」）。
    check_ran: bool,
    progress: f32,
    progress_label: String,
    warnings: Vec<String>,
    settings: AppSettings,
}

impl Default for MergeApp {
    fn default() -> Self {
        let settings = load_settings();
        Self {
            sources: Vec::new(),
            input_label: "尚未选择文件或文件夹，可直接拖放".to_owned(),
            output_path: settings.output_directory.clone(),
            options: MergeOptions::default(),
            hide_common_mappings: false,
            mismatch_only: false,
            source_search: String::new(),
            mapping_search: String::new(),
            selected_mapping_table: 0,
            collapsed_groups: HashSet::new(),
            preview: None,
            preview_title: "选择一个数据表生成预览".to_owned(),
            check_issues: Vec::new(),
            scan_rx: None,
            merge_rx: None,
            update_rx: None,
            update_text: "检查更新".to_owned(),
            update_url: None,
            cancel: Arc::new(AtomicBool::new(false)),
            state: AppState::Ready,
            preflight_continues: false,
            check_ran: false,
            progress: 0.0,
            progress_label: String::new(),
            warnings: Vec::new(),
            settings,
        }
    }
}

impl MergeApp {
    fn busy(&self) -> bool {
        matches!(
            self.state,
            AppState::Scanning | AppState::Checking | AppState::Merging
        )
    }
    fn enabled_indices(&self) -> Vec<usize> {
        self.sources
            .iter()
            .enumerate()
            .filter_map(|(index, table)| table.enabled.then_some(index))
            .collect()
    }
    fn ensure_mapping_selection(&mut self) {
        let enabled = self.enabled_indices();
        if let Some(first) = enabled.first() {
            if !enabled.contains(&self.selected_mapping_table) {
                self.selected_mapping_table = *first;
            }
        }
    }
    fn remember_output(&mut self) {
        if let Some(parent) = Path::new(&self.output_path).parent() {
            self.settings.output_directory = parent.display().to_string();
        }
        let _ = save_settings(&self.settings);
    }
    fn start_folder_scan(&mut self, folder: PathBuf) {
        let paths = collect_folder(&folder);
        remember(
            &mut self.settings.recent_folders,
            folder.display().to_string(),
        );
        let _ = save_settings(&self.settings);
        self.output_path = folder.join("合并结果.xlsx").display().to_string();
        self.start_scan(paths, folder.display().to_string());
    }
    fn start_files_scan(&mut self, mut paths: Vec<PathBuf>) {
        paths.retain(|path| supported_file(path));
        paths.sort();
        paths.dedup();
        if paths.is_empty() {
            self.state = AppState::Error("没有找到支持的表格文件".to_owned());
            return;
        }
        if let Some(folder) = paths.first().and_then(|path| path.parent()) {
            self.output_path = folder.join("合并结果.xlsx").display().to_string();
            remember(
                &mut self.settings.recent_folders,
                folder.display().to_string(),
            );
            let _ = save_settings(&self.settings);
        }
        self.start_scan(paths.clone(), format!("已选择 {} 个文件", paths.len()));
    }
    fn start_scan(&mut self, mut paths: Vec<PathBuf>, label: String) {
        let output = PathBuf::from(&self.output_path);
        paths.retain(|path| path != &output);
        if paths.is_empty() {
            self.state = AppState::Error("没有找到支持的表格文件".to_owned());
            return;
        }
        let (tx, rx) = mpsc::channel();
        self.scan_rx = Some(rx);
        self.state = AppState::Scanning;
        self.progress = 0.0;
        self.progress_label = "正在读取文件并自动识别表头…".to_owned();
        self.input_label = label;
        self.warnings.clear();
        self.preview = None;
        self.check_issues.clear();
        self.check_ran = false;
        spawn_scan(paths, tx);
    }
    fn start_table_reload(&mut self, index: usize, header_row: usize, header_rows: usize) {
        let Some(source) = self.sources.get(index).cloned() else {
            return;
        };
        let (tx, rx) = mpsc::channel();
        self.scan_rx = Some(rx);
        self.state = AppState::Scanning;
        self.progress = 0.15;
        self.progress_label = format!(
            "正在从第 {header_row} 行开始、读取 {header_rows} 行表头：{}",
            source.display_name()
        );
        spawn_table_reload(index, source, header_row, header_rows, tx);
    }
    fn start_group_reload(&mut self, path: &str) {
        let sources = self
            .sources
            .iter()
            .enumerate()
            .filter(|(_, table)| table.path.display().to_string() == path)
            .map(|(index, table)| (index, table.clone()))
            .collect::<Vec<_>>();
        let Some((_, first)) = sources.first() else {
            return;
        };
        let header_row = first.header_row;
        let header_rows = first.header_rows;
        let (tx, rx) = mpsc::channel();
        self.scan_rx = Some(rx);
        self.state = AppState::Scanning;
        self.progress = 0.15;
        self.progress_label = format!(
            "正在把从第 {header_row} 行开始、占用 {header_rows} 行的表头设置应用到整本工作簿…"
        );
        spawn_group_reload(sources, header_row, header_rows, tx);
    }
    fn start_merge(&mut self) {
        let output = PathBuf::from(self.output_path.trim());
        if output.as_os_str().is_empty() {
            self.state = AppState::Error("请先选择输出文件".to_owned());
            return;
        }
        if !output
            .extension()
            .and_then(|ext| ext.to_str())
            .is_some_and(|ext| ext.eq_ignore_ascii_case("xlsx"))
        {
            self.state = AppState::Error("输出文件必须使用 .xlsx 扩展名".to_owned());
            return;
        }
        self.run_preflight_background(true);
    }
    /// 在后台线程执行合并前检查，避免大文件时冻结界面。
    fn run_preflight_background(&mut self, continues_merge: bool) {
        if self.busy() {
            return;
        }
        let (tx, rx) = mpsc::channel();
        self.merge_rx = Some(rx);
        self.preflight_continues = continues_merge;
        self.state = AppState::Checking;
        self.progress = 0.0;
        self.progress_label = "正在执行合并前检查…".to_owned();
        spawn_preflight(self.sources.clone(), self.options.clone(), tx);
    }
    /// preflight 通过后真正启动合并：覆盖确认 + 后台合并线程。
    fn begin_merge(&mut self) {
        let output = PathBuf::from(self.output_path.trim());
        if output.exists() {
            let confirmed = rfd::MessageDialog::new()
                .set_title("覆盖已有文件")
                .set_description(format!("{} 已存在，是否覆盖？", output.display()))
                .set_buttons(rfd::MessageButtons::YesNo)
                .show();
            if confirmed != rfd::MessageDialogResult::Yes {
                self.state = AppState::Ready;
                return;
            }
        }
        self.remember_output();
        let (tx, rx) = mpsc::channel();
        self.merge_rx = Some(rx);
        self.cancel = Arc::new(AtomicBool::new(false));
        self.state = AppState::Merging;
        self.progress = 0.0;
        self.progress_label = "正在准备输出工作簿…".to_owned();
        append_log(&format!(
            "开始合并：{} 个数据表 -> {}",
            self.enabled_indices().len(),
            output.display()
        ));
        spawn_merge(
            self.sources.clone(),
            self.options.clone(),
            output,
            tx,
            self.cancel.clone(),
        );
    }
    fn show_source_preview(&mut self, index: usize) {
        let Some(table) = self.sources.get(index) else {
            return;
        };
        match preview_source(table, 30) {
            Ok(preview) => {
                self.preview_title =
                    format!("{} · 前 {} 行", table.display_name(), preview.rows.len());
                self.preview = Some(preview);
            }
            Err(error) => self.state = AppState::Error(format!("预览失败：{error:#}")),
        }
    }
    fn show_merged_preview(&mut self) {
        match preview_merged(&self.sources, &self.options, 30) {
            Ok(preview) => {
                self.preview_title = format!(
                    "合并结果预览 · {} 列 · 前 {} 行",
                    preview.headers.len(),
                    preview.rows.len()
                );
                self.preview = Some(preview);
            }
            Err(error) => self.state = AppState::Error(format!("结果预览失败：{error:#}")),
        }
    }
    fn run_preflight(&mut self) {
        self.run_preflight_background(false);
    }
    fn open_scheme(&mut self, path: &Path) {
        match load_scheme(path) {
            Ok(scheme) => {
                self.sources = scheme.tables;
                self.options = scheme.options;
                self.ensure_mapping_selection();
                self.input_label = format!("已打开方案：{}", path.display());
                self.check_issues.clear();
                self.check_ran = false;
                remember(
                    &mut self.settings.recent_schemes,
                    path.display().to_string(),
                );
                let _ = save_settings(&self.settings);
            }
            Err(error) => {
                self.state = AppState::Error(format!("打开方案失败：{error:#}"));
            }
        }
    }
    fn start_update_check(&mut self) {
        if let Some(url) = self.update_url.clone() {
            let _ = webbrowser::open(&url);
            return;
        }
        let (tx, rx) = mpsc::channel();
        self.update_rx = Some(rx);
        self.update_text = "正在检查…".to_owned();
        std::thread::spawn(move || {
            let result = (|| -> anyhow::Result<(String, String)> {
                let user_agent = format!("SheetForge/{}", env!("CARGO_PKG_VERSION"));
                let body =
                    ureq::get("https://api.github.com/repos/Wyk521/SheetForge/releases/latest")
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
            let event = match result {
                Ok((version, url)) => UpdateEvent::Result { version, url },
                Err(error) => UpdateEvent::Failed(format!("{error:#}")),
            };
            let _ = tx.send(event);
        });
    }
    fn poll_workers(&mut self) -> bool {
        let mut changed = false;
        let scan_events = self
            .scan_rx
            .as_ref()
            .map(|rx| rx.try_iter().collect::<Vec<_>>())
            .unwrap_or_default();
        for event in scan_events {
            changed = true;
            match event {
                ScanEvent::Progress { done, total, name } => {
                    self.progress = if total == 0 {
                        0.0
                    } else {
                        done as f32 / total as f32
                    };
                    self.progress_label = format!("正在扫描：{name}");
                }
                ScanEvent::Finished { tables, warnings } => {
                    self.sources = tables;
                    self.warnings = warnings;
                    self.selected_mapping_table = 0;
                    self.ensure_mapping_selection();
                    self.options.output_order.clear();
                    self.progress = 1.0;
                    self.progress_label = format!("已识别 {} 个数据表", self.sources.len());
                    self.state = AppState::Ready;
                    self.scan_rx = None;
                }
                ScanEvent::TableReloaded { index, table } => {
                    let name = table.display_name();
                    if let Some(slot) = self.sources.get_mut(index) {
                        *slot = table;
                    }
                    self.selected_mapping_table = index;
                    self.preview = None;
                    self.check_issues.clear();
                    self.check_ran = false;
                    self.progress = 1.0;
                    self.progress_label = format!("表头已刷新：{name}");
                    self.state = AppState::Ready;
                    self.scan_rx = None;
                }
                ScanEvent::TablesReloaded { tables } => {
                    let count = tables.len();
                    for (index, table) in tables {
                        if let Some(slot) = self.sources.get_mut(index) {
                            *slot = table;
                        }
                    }
                    self.preview = None;
                    self.check_issues.clear();
                    self.check_ran = false;
                    self.progress = 1.0;
                    self.progress_label = format!("已统一刷新 {count} 个数据表的表头");
                    self.state = AppState::Ready;
                    self.scan_rx = None;
                }
                ScanEvent::TableReloadFailed { index, message } => {
                    self.selected_mapping_table = index.min(self.sources.len().saturating_sub(1));
                    self.state = AppState::Error(format!("重新读取表头失败：{message}"));
                    self.scan_rx = None;
                }
                ScanEvent::Failed(message) => {
                    self.state = AppState::Error(message);
                    self.scan_rx = None;
                }
            }
        }
        let merge_events = self
            .merge_rx
            .as_ref()
            .map(|rx| rx.try_iter().collect::<Vec<_>>())
            .unwrap_or_default();
        for event in merge_events {
            changed = true;
            match event {
                MergeEvent::Progress {
                    current,
                    total,
                    label,
                } => {
                    self.progress = if total == 0 {
                        0.0
                    } else {
                        current as f32 / total as f32
                    };
                    self.progress_label = format!("{label}  ·  {current} / {total} 行");
                }
                MergeEvent::Preflight { issues } => {
                    let errors = issues
                        .iter()
                        .filter(|issue| issue.level == IssueLevel::Error)
                        .count();
                    let warnings = issues
                        .iter()
                        .filter(|issue| issue.level == IssueLevel::Warning)
                        .count();
                    self.check_issues = issues;
                    self.check_ran = true;
                    if let Some(issue) = self
                        .check_issues
                        .iter()
                        .find(|issue| issue.level == IssueLevel::Error)
                    {
                        self.state = AppState::Error(format!(
                            "合并前检查未通过：{} — {}",
                            issue.title, issue.detail
                        ));
                        self.merge_rx = None;
                    } else if self.preflight_continues {
                        self.preflight_continues = false;
                        self.begin_merge();
                    } else {
                        self.progress_label =
                            format!("检查完成：{errors} 个错误，{warnings} 个提醒");
                        self.state = AppState::Ready;
                        self.merge_rx = None;
                    }
                }
                MergeEvent::Finished {
                    output,
                    rows,
                    sheets,
                } => {
                    self.progress = 1.0;
                    self.progress_label = "合并完成".to_owned();
                    append_log(&format!(
                        "合并完成：{rows} 行，{sheets} 个 Sheet，{}",
                        output.display()
                    ));
                    self.state = AppState::Done {
                        output,
                        rows,
                        sheets,
                    };
                    self.merge_rx = None;
                }
                MergeEvent::Cancelled => {
                    self.progress_label = "已取消".to_owned();
                    append_log("用户取消合并");
                    self.state = AppState::Ready;
                    self.merge_rx = None;
                }
                MergeEvent::Failed(message) => {
                    append_log(&format!("合并失败：{message}"));
                    self.state = AppState::Error(message);
                    self.merge_rx = None;
                }
            }
        }
        let update_events = self
            .update_rx
            .as_ref()
            .map(|rx| rx.try_iter().collect::<Vec<_>>())
            .unwrap_or_default();
        for event in update_events {
            changed = true;
            match event {
                UpdateEvent::Result { version, url } => {
                    if is_newer_version(&version, env!("CARGO_PKG_VERSION")) {
                        self.update_text = format!("发现 {version}，打开下载页");
                        self.update_url = Some(url);
                    } else {
                        self.update_text = "已是最新版本".to_owned();
                    }
                    self.update_rx = None;
                }
                UpdateEvent::Failed(message) => {
                    self.update_text = "检查失败，重试".to_owned();
                    self.state = AppState::Error(format!("检查更新失败：{message}"));
                    self.update_rx = None;
                }
            }
        }
        changed
    }
}

pub fn run() -> Result<(), slint::PlatformError> {
    let ui = AppWindow::new()?;
    let state = Rc::new(RefCell::new(MergeApp::default()));
    {
        let app = state.borrow();
        ui.window().set_maximized(app.settings.window_maximized);
    }
    sync_ui(&ui, &state.borrow());
    install_callbacks(&ui, state.clone());
    let weak = ui.as_weak();
    let poll_state = state.clone();
    let timer = Timer::default();
    timer.start(TimerMode::Repeated, Duration::from_millis(80), move || {
        let Some(ui) = weak.upgrade() else {
            return;
        };
        if poll_state.borrow_mut().poll_workers() {
            sync_ui(&ui, &poll_state.borrow());
        }
    });
    let result = ui.run();
    let mut app = state.borrow_mut();
    let maximized = ui.window().is_maximized();
    app.settings.window_maximized = maximized;
    let _ = save_settings(&app.settings);
    result
}

fn install_callbacks(ui: &AppWindow, state: Rc<RefCell<MergeApp>>) {
    macro_rules! sync_callback {
        ($setter:ident, $body:expr) => {{
            let weak = ui.as_weak();
            let callback_state = state.clone();
            ui.$setter(move || {
                $body(&callback_state);
                sync_weak(&weak, &callback_state);
            });
        }};
    }
    sync_callback!(on_choose_folder, |state: &Rc<RefCell<MergeApp>>| {
        if !state.borrow().busy() {
            if let Some(folder) = FileDialog::new().pick_folder() {
                state.borrow_mut().start_folder_scan(folder);
            }
        }
    });
    sync_callback!(on_choose_files, |state: &Rc<RefCell<MergeApp>>| {
        if !state.borrow().busy() {
            if let Some(paths) = FileDialog::new()
                .add_filter(
                    "表格文件",
                    &["xlsx", "xlsm", "xls", "xlsb", "ods", "csv", "tsv"],
                )
                .pick_files()
            {
                state.borrow_mut().start_files_scan(paths);
            }
        }
    });
    sync_callback!(on_save_scheme, |state: &Rc<RefCell<MergeApp>>| {
        if let Some(path) = FileDialog::new()
            .add_filter("表格合并方案", &["json"])
            .set_file_name("合并方案.json")
            .save_file()
        {
            let app = state.borrow();
            let scheme = MergeScheme {
                format_version: 1,
                name: path
                    .file_stem()
                    .map(|v| v.to_string_lossy().into_owned())
                    .unwrap_or_default(),
                tables: app.sources.clone(),
                options: app.options.clone(),
            };
            drop(app);
            match save_scheme(&path, &scheme) {
                Ok(()) => {
                    let mut app = state.borrow_mut();
                    remember(&mut app.settings.recent_schemes, path.display().to_string());
                    let _ = save_settings(&app.settings);
                    app.progress_label = format!("方案已保存：{}", path.display());
                }
                Err(error) => {
                    state.borrow_mut().state = AppState::Error(format!("保存方案失败：{error:#}"))
                }
            }
        }
    });
    sync_callback!(on_load_scheme, |state: &Rc<RefCell<MergeApp>>| {
        if let Some(path) = FileDialog::new()
            .add_filter("表格合并方案", &["json"])
            .pick_file()
        {
            state.borrow_mut().open_scheme(&path);
        }
    });
    let weak = ui.as_weak();
    let callback_state = state.clone();
    ui.on_recent_folder_selected(move |value| {
        let mut app = callback_state.borrow_mut();
        if !app.busy() {
            app.start_folder_scan(PathBuf::from(value.to_string()));
        }
        drop(app);
        sync_weak(&weak, &callback_state);
    });
    let weak = ui.as_weak();
    let callback_state = state.clone();
    ui.on_recent_scheme_selected(move |value| {
        let mut app = callback_state.borrow_mut();
        if !app.busy() {
            app.open_scheme(&PathBuf::from(value.to_string()));
        }
        drop(app);
        sync_weak(&weak, &callback_state);
    });
    let weak = ui.as_weak();
    let callback_state = state.clone();
    ui.on_copy_cell(move |value| {
        let copied = arboard::Clipboard::new()
            .and_then(|mut clipboard| clipboard.set_text(value.to_string()));
        let mut app = callback_state.borrow_mut();
        match copied {
            Ok(()) => {
                app.progress_label =
                    format!("已复制单元格内容（{} 个字符）", value.chars().count());
            }
            Err(error) => {
                app.state = AppState::Error(format!("复制到剪贴板失败：{error}"));
            }
        }
        drop(app);
        sync_weak(&weak, &callback_state);
    });

    let weak = ui.as_weak();
    let callback_state = state.clone();
    ui.on_source_enabled_changed(move |index, enabled| {
        let mut app = callback_state.borrow_mut();
        if let Some(table) = app.sources.get_mut(index.max(0) as usize) {
            table.enabled = enabled;
        }
        app.ensure_mapping_selection();
        drop(app);
        sync_weak(&weak, &callback_state);
    });
    let weak = ui.as_weak();
    let callback_state = state.clone();
    ui.on_select_all_sources(move |enabled| {
        let mut app = callback_state.borrow_mut();
        for table in &mut app.sources {
            table.enabled = enabled;
        }
        app.ensure_mapping_selection();
        drop(app);
        sync_weak(&weak, &callback_state);
    });
    let weak = ui.as_weak();
    let callback_state = state.clone();
    ui.on_toggle_source_group(move |path| {
        let mut app = callback_state.borrow_mut();
        let path = path.to_string();
        if !app.collapsed_groups.remove(&path) {
            app.collapsed_groups.insert(path);
        }
        drop(app);
        sync_weak(&weak, &callback_state);
    });
    let weak = ui.as_weak();
    let callback_state = state.clone();
    ui.on_source_group_enabled_changed(move |path, enabled| {
        let path = path.to_string();
        let mut app = callback_state.borrow_mut();
        for table in &mut app.sources {
            if table.path.display().to_string() == path {
                table.enabled = enabled;
            }
        }
        app.ensure_mapping_selection();
        drop(app);
        sync_weak(&weak, &callback_state);
    });
    let weak = ui.as_weak();
    let callback_state = state.clone();
    ui.on_apply_source_group_header(move |path| {
        callback_state
            .borrow_mut()
            .start_group_reload(path.as_str());
        sync_weak(&weak, &callback_state);
    });
    let weak = ui.as_weak();
    let callback_state = state.clone();
    ui.on_remove_source_group(move |path| {
        let path = path.to_string();
        let app = callback_state.borrow_mut();
        let count = app
            .sources
            .iter()
            .filter(|table| table.path.display().to_string() == path)
            .count();
        drop(app);
        if count == 0 || !confirm(&format!("确定移除该工作簿及其 {count} 个数据表？"))
        {
            return;
        }
        let mut app = callback_state.borrow_mut();
        app.sources
            .retain(|table| table.path.display().to_string() != path);
        app.collapsed_groups.remove(&path);
        app.ensure_mapping_selection();
        drop(app);
        sync_weak(&weak, &callback_state);
    });
    let weak = ui.as_weak();
    let callback_state = state.clone();
    ui.on_remove_source(move |index| {
        let app = callback_state.borrow_mut();
        let index = index.max(0) as usize;
        let name = app
            .sources
            .get(index)
            .map(SourceTable::display_name)
            .unwrap_or_default();
        drop(app);
        if name.is_empty() || !confirm(&format!("确定移除数据表“{name}”？")) {
            return;
        }
        let mut app = callback_state.borrow_mut();
        if index < app.sources.len() {
            app.sources.remove(index);
        }
        app.ensure_mapping_selection();
        drop(app);
        sync_weak(&weak, &callback_state);
    });
    let weak = ui.as_weak();
    let callback_state = state.clone();
    ui.on_header_changed(move |index, row, rows| {
        callback_state.borrow_mut().start_table_reload(
            index.max(0) as usize,
            row.max(1) as usize,
            rows.clamp(1, 3) as usize,
        );
        sync_weak(&weak, &callback_state);
    });
    let weak = ui.as_weak();
    let callback_state = state.clone();
    ui.on_source_search_changed(move |value| {
        callback_state.borrow_mut().source_search = value.to_string();
        sync_weak(&weak, &callback_state);
    });

    let weak = ui.as_weak();
    let callback_state = state.clone();
    ui.on_mode_changed(move |mode| {
        callback_state.borrow_mut().options.mode = match mode {
            1 => MergeMode::Intersection,
            2 => MergeMode::Manual,
            3 => MergeMode::Consolidate,
            4 => MergeMode::Join,
            _ => MergeMode::Union,
        };
        sync_weak(&weak, &callback_state);
    });
    let weak = ui.as_weak();
    let callback_state = state.clone();
    ui.on_include_source_file_changed(move |value| {
        callback_state.borrow_mut().options.include_source_file = value;
        sync_weak(&weak, &callback_state);
    });
    let weak = ui.as_weak();
    let callback_state = state.clone();
    ui.on_include_source_sheet_changed(move |value| {
        callback_state.borrow_mut().options.include_source_sheet = value;
        sync_weak(&weak, &callback_state);
    });
    let weak = ui.as_weak();
    let callback_state = state.clone();
    ui.on_mapping_table_changed(move |position| {
        let mut app = callback_state.borrow_mut();
        if let Some(index) = app.enabled_indices().get(position.max(0) as usize) {
            app.selected_mapping_table = *index;
        }
        drop(app);
        sync_weak(&weak, &callback_state);
    });
    let weak = ui.as_weak();
    let callback_state = state.clone();
    ui.on_mapping_changed(move |index, enabled, target| {
        let mut app = callback_state.borrow_mut();
        let table = app.selected_mapping_table;
        if let Some(mapping) = app
            .sources
            .get_mut(table)
            .and_then(|table| table.mappings.get_mut(index.max(0) as usize))
        {
            mapping.enabled = enabled;
            mapping.target_name = target.to_string();
        }
        drop(app);
        sync_weak(&weak, &callback_state);
    });
    let weak = ui.as_weak();
    let callback_state = state.clone();
    ui.on_mapping_operation_changed(move |index, transform, aggregate| {
        let mut app = callback_state.borrow_mut();
        let table = app.selected_mapping_table;
        if let Some(mapping) = app
            .sources
            .get_mut(table)
            .and_then(|table| table.mappings.get_mut(index.max(0) as usize))
        {
            mapping.transform = transform_from_index(transform);
            mapping.aggregate = aggregate_from_index(aggregate);
        }
        drop(app);
        sync_weak(&weak, &callback_state);
    });
    sync_callback!(on_reset_mapping, |state: &Rc<RefCell<MergeApp>>| {
        let name = state
            .borrow()
            .sources
            .get(state.borrow().selected_mapping_table)
            .map(SourceTable::display_name)
            .unwrap_or_default();
        if name.is_empty() || !confirm(&format!("确定恢复“{name}”的所有字段映射？"))
        {
            return;
        }
        let mut app = state.borrow_mut();
        let index = app.selected_mapping_table;
        if let Some(table) = app.sources.get_mut(index) {
            for mapping in &mut table.mappings {
                mapping.target_name = mapping.source_name.clone();
                mapping.enabled = true;
                mapping.transform = TransformOp::None;
                mapping.aggregate = AggregateOp::First;
            }
        }
    });
    sync_callback!(on_toggle_common_fields, |state: &Rc<RefCell<MergeApp>>| {
        let value = state.borrow().hide_common_mappings;
        state.borrow_mut().hide_common_mappings = !value;
    });
    let weak = ui.as_weak();
    let callback_state = state.clone();
    ui.on_mapping_search_changed(move |value| {
        callback_state.borrow_mut().mapping_search = value.to_string();
        sync_weak(&weak, &callback_state);
    });
    let weak = ui.as_weak();
    let callback_state = state.clone();
    ui.on_mismatch_only_changed(move |value| {
        callback_state.borrow_mut().mismatch_only = value;
        sync_weak(&weak, &callback_state);
    });
    sync_callback!(on_apply_suggestions, |state: &Rc<RefCell<MergeApp>>| {
        let suggestions = mapping_suggestions(&state.borrow().sources);
        let mut app = state.borrow_mut();
        for table in &mut app.sources {
            for mapping in &mut table.mappings {
                if let Some(target) = suggestions.get(&mapping.source_name) {
                    mapping.target_name = target.clone();
                }
            }
        }
    });
    let weak = ui.as_weak();
    let callback_state = state.clone();
    ui.on_advanced_options_changed(
        move |keys, dedup, filter_column, filter_text, exclude, join_kind| {
            let mut app = callback_state.borrow_mut();
            app.options.key_columns = split_columns(&keys);
            app.options.deduplicate = dedup;
            app.options.filter_column = filter_column.to_string();
            app.options.filter_text = filter_text.to_string();
            app.options.filter_exclude = exclude;
            app.options.join_kind = match join_kind {
                1 => JoinKind::Inner,
                2 => JoinKind::Full,
                _ => JoinKind::Left,
            };
            drop(app);
            sync_weak(&weak, &callback_state);
        },
    );
    let weak = ui.as_weak();
    let callback_state = state.clone();
    ui.on_move_output_column(move |index, direction| {
        let mut app = callback_state.borrow_mut();
        let mut headers = build_output_plan(&app.sources, &app.options).headers;
        let index = index.max(0) as usize;
        if index < headers.len() {
            let target = (index as i32 + direction).clamp(0, headers.len().saturating_sub(1) as i32)
                as usize;
            headers.swap(index, target);
            app.options.output_order = headers;
        }
        drop(app);
        sync_weak(&weak, &callback_state);
    });

    let weak = ui.as_weak();
    let callback_state = state.clone();
    ui.on_preview_source(move |position| {
        let mut app = callback_state.borrow_mut();
        let enabled = app.enabled_indices();
        let requested = position.max(0) as usize;
        let actual = if enabled.contains(&requested) {
            requested
        } else {
            enabled.get(requested).copied().unwrap_or(requested)
        };
        app.show_source_preview(actual);
        drop(app);
        sync_weak(&weak, &callback_state);
    });
    sync_callback!(on_preview_merged, |state: &Rc<RefCell<MergeApp>>| state
        .borrow_mut()
        .show_merged_preview());
    sync_callback!(on_run_check, |state: &Rc<RefCell<MergeApp>>| state
        .borrow_mut()
        .run_preflight());
    sync_callback!(on_export_check, |state: &Rc<RefCell<MergeApp>>| {
        if let Some(path) = FileDialog::new()
            .add_filter("文本报告", &["txt"])
            .set_file_name("合并检查报告.txt")
            .save_file()
        {
            let app = state.borrow();
            let report = app
                .check_issues
                .iter()
                .map(|issue| format!("[{:?}] {}\n{}", issue.level, issue.title, issue.detail))
                .collect::<Vec<_>>()
                .join("\n\n");
            if let Err(error) = std::fs::write(&path, report) {
                drop(app);
                state.borrow_mut().state = AppState::Error(format!("导出检查报告失败：{error}"));
            }
        }
    });

    let callback_state = state.clone();
    ui.on_output_path_changed(move |path| {
        callback_state.borrow_mut().output_path = path.to_string();
    });
    sync_callback!(on_choose_output, |state: &Rc<RefCell<MergeApp>>| {
        if !state.borrow().busy() {
            if let Some(path) = FileDialog::new()
                .add_filter("Excel 工作簿", &["xlsx"])
                .set_file_name("合并结果.xlsx")
                .save_file()
            {
                state.borrow_mut().output_path = path.display().to_string();
            }
        }
    });
    sync_callback!(on_start_merge, |state: &Rc<RefCell<MergeApp>>| state
        .borrow_mut()
        .start_merge());
    sync_callback!(on_cancel_merge, |state: &Rc<RefCell<MergeApp>>| {
        if !matches!(state.borrow().state, AppState::Merging)
            || !confirm("确定取消当前合并？已写入的部分不会保留。")
        {
            return;
        }
        let mut app = state.borrow_mut();
        app.cancel.store(true, Ordering::Relaxed);
        app.progress_label = "正在取消…".to_owned();
    });
    sync_callback!(on_reveal_output, |state: &Rc<RefCell<MergeApp>>| {
        if let AppState::Done { output, .. } = &state.borrow().state {
            reveal_in_explorer(output);
        }
    });
    sync_callback!(on_check_update, |state: &Rc<RefCell<MergeApp>>| state
        .borrow_mut()
        .start_update_check());
    sync_callback!(on_open_log, |_state: &Rc<RefCell<MergeApp>>| {
        reveal_in_explorer(&log_path())
    });

    ui.on_can_drop(|data: DataTransfer| data.has_plain_text());
    let weak = ui.as_weak();
    let callback_state = state;
    ui.on_files_dropped(move |data: DataTransfer| {
        if let Ok(text) = data.plain_text() {
            let paths = paths_from_drop_text(&text);
            if !paths.is_empty() {
                callback_state.borrow_mut().start_files_scan(paths);
            }
        }
        sync_weak(&weak, &callback_state);
    });
}

fn sync_weak(weak: &slint::Weak<AppWindow>, state: &Rc<RefCell<MergeApp>>) {
    if let Some(ui) = weak.upgrade() {
        sync_ui(&ui, &state.borrow());
    }
}

fn sync_ui(ui: &AppWindow, app: &MergeApp) {
    let filter = app.source_search.trim().to_lowercase();
    let mut grouped = BTreeMap::<String, Vec<(usize, &SourceTable)>>::new();
    for (index, table) in app.sources.iter().enumerate() {
        let searchable = format!(
            "{} {} {}",
            table.path.display(),
            table.sheet_name,
            table.display_name()
        )
        .to_lowercase();
        if filter.is_empty() || searchable.contains(&filter) {
            grouped
                .entry(table.path.display().to_string())
                .or_default()
                .push((index, table));
        }
    }
    let mut source_rows = Vec::new();
    for (path, tables) in grouped {
        let collapsed = app.collapsed_groups.contains(&path);
        let file_name = Path::new(&path)
            .file_name()
            .map(|value| value.to_string_lossy().into_owned())
            .unwrap_or_else(|| path.clone());
        source_rows.push(SourceRow {
            is_group: true,
            source_index: -1,
            enabled: tables.iter().all(|(_, table)| table.enabled),
            collapsed,
            file_name: file_name.into(),
            sheet_name: "".into(),
            detail: format!(
                "{} 个数据表 · 已选 {} 个",
                tables.len(),
                tables.iter().filter(|(_, table)| table.enabled).count()
            )
            .into(),
            path: path.clone().into(),
            header_row: 1,
            header_rows: 1,
            suggested_row: 1,
        });
        if !collapsed {
            for (index, table) in tables {
                source_rows.push(SourceRow {
                    is_group: false,
                    source_index: index as i32,
                    enabled: table.enabled,
                    collapsed: false,
                    file_name: "".into(),
                    sheet_name: table.sheet_name.clone().into(),
                    detail: format!(
                        "{} 行 · {} 列",
                        format_number(table.estimated_rows),
                        table.headers.len()
                    )
                    .into(),
                    path: path.clone().into(),
                    header_row: table.header_row as i32,
                    header_rows: table.header_rows as i32,
                    suggested_row: table.suggested_header_row as i32,
                });
            }
        }
    }
    ui.set_sources(ModelRc::new(VecModel::from(source_rows)));
    let selected_count = app.sources.iter().filter(|table| table.enabled).count();
    ui.set_input_label(app.input_label.clone().into());
    ui.set_sources_summary(
        format!(
            "{} 个文件 · {} 个数据表 · 已选 {selected_count} 个",
            app.sources
                .iter()
                .map(|t| &t.path)
                .collect::<HashSet<_>>()
                .len(),
            app.sources.len()
        )
        .into(),
    );
    ui.set_source_search(app.source_search.clone().into());

    let enabled_indices = app.enabled_indices();
    let names = enabled_indices
        .iter()
        .map(|index| app.sources[*index].display_name().into())
        .collect::<Vec<SharedString>>();
    ui.set_mapping_tables(ModelRc::new(VecModel::from(names)));
    let position = enabled_indices
        .iter()
        .position(|index| *index == app.selected_mapping_table)
        .unwrap_or(0);
    ui.set_mapping_table_index(position as i32);
    ui.set_preview_table_index(position as i32);
    let common = common_header_keys(&app.sources);
    let suggestions = mapping_suggestions(&app.sources);
    let (mappings, common_count) = app
        .sources
        .get(app.selected_mapping_table)
        .map(|table| {
            let common_count = table
                .mappings
                .iter()
                .filter(|mapping| common.contains(&header_key(&mapping.source_name)))
                .count();
            let search = app.mapping_search.to_lowercase();
            let rows: Vec<MappingRow> = table
                .mappings
                .iter()
                .enumerate()
                .filter(|(_, mapping)| {
                    let common_field = common.contains(&header_key(&mapping.source_name));
                    let differs = header_key(&mapping.source_name)
                        != header_key(&mapping.target_name)
                        || suggestions.contains_key(&mapping.source_name);
                    (!app.hide_common_mappings || !common_field)
                        && (!app.mismatch_only || differs)
                        && (search.is_empty()
                            || mapping.source_name.to_lowercase().contains(&search)
                            || mapping.target_name.to_lowercase().contains(&search))
                })
                .map(|(index, mapping)| MappingRow {
                    enabled: mapping.enabled,
                    mapping_index: index as i32,
                    source_name: mapping.source_name.clone().into(),
                    target_name: mapping.target_name.clone().into(),
                    suggestion: suggestions
                        .get(&mapping.source_name)
                        .cloned()
                        .unwrap_or_default()
                        .into(),
                    transform_index: transform_index(mapping.transform),
                    aggregate_index: aggregate_index(mapping.aggregate),
                })
                .collect();
            (rows, common_count)
        })
        .unwrap_or_default();
    ui.set_mappings(ModelRc::new(VecModel::from(mappings)));
    ui.set_mapping_search(app.mapping_search.clone().into());
    ui.set_mismatch_only(app.mismatch_only);
    ui.set_common_fields_label(
        (if app.hide_common_mappings {
            format!("显示共有字段（已隐藏 {common_count}）")
        } else {
            format!("隐藏共有字段（{common_count}）")
        })
        .into(),
    );

    let plan = build_output_plan(&app.sources, &app.options);
    let output_columns = plan
        .headers
        .iter()
        .enumerate()
        .map(|(index, name)| ColumnRow {
            column_index: index as i32,
            name: name.clone().into(),
        })
        .collect::<Vec<_>>();
    ui.set_output_columns(ModelRc::new(VecModel::from(output_columns)));
    ui.set_preview_rows(ModelRc::new(VecModel::from(preview_rows(
        app.preview.as_ref(),
    ))));
    ui.set_preview_column_widths(preview_column_widths(app.preview.as_ref()));
    ui.set_preview_title(app.preview_title.clone().into());
    let check_rows = app
        .check_issues
        .iter()
        .map(|issue| CheckRow {
            level: match issue.level {
                IssueLevel::Info => 0,
                IssueLevel::Warning => 1,
                IssueLevel::Error => 2,
            },
            title: issue.title.clone().into(),
            detail: issue.detail.clone().into(),
        })
        .collect::<Vec<_>>();
    ui.set_check_rows(ModelRc::new(VecModel::from(check_rows)));
    let errors = app
        .check_issues
        .iter()
        .filter(|issue| issue.level == IssueLevel::Error)
        .count();
    let warnings = app
        .check_issues
        .iter()
        .filter(|issue| issue.level == IssueLevel::Warning)
        .count();
    ui.set_check_summary(if app.check_issues.is_empty() {
        if app.check_ran {
            "检查完成：未发现问题".into()
        } else {
            "尚未检查".into()
        }
    } else {
        format!("检查完成：{errors} 个错误，{warnings} 个提醒").into()
    });
    ui.set_check_clean(app.check_ran && app.check_issues.is_empty());
    let mut recent_folders = vec!["最近文件夹…".to_owned()];
    recent_folders.extend(app.settings.recent_folders.iter().cloned());
    ui.set_recent_folders(ModelRc::new(VecModel::from(
        recent_folders
            .iter()
            .map(|value| SharedString::from(value.as_str()))
            .collect::<Vec<_>>(),
    )));
    let mut recent_schemes = vec!["最近方案…".to_owned()];
    recent_schemes.extend(app.settings.recent_schemes.iter().cloned());
    ui.set_recent_schemes(ModelRc::new(VecModel::from(
        recent_schemes
            .iter()
            .map(|value| SharedString::from(value.as_str()))
            .collect::<Vec<_>>(),
    )));
    ui.set_recent_folders_index(0);
    ui.set_recent_schemes_index(0);

    let rows: u64 = app
        .sources
        .iter()
        .filter(|table| table.enabled)
        .map(|table| table.estimated_rows)
        .sum();
    let expected_sheets = rows.max(1).div_ceil(XLSX_MAX_DATA_ROWS as u64);
    ui.set_mode_index(match app.options.mode {
        MergeMode::Union => 0,
        MergeMode::Intersection => 1,
        MergeMode::Manual => 2,
        MergeMode::Consolidate => 3,
        MergeMode::Join => 4,
    });
    ui.set_include_source_file(app.options.include_source_file);
    ui.set_include_source_sheet(app.options.include_source_sheet);
    ui.set_deduplicate(app.options.deduplicate);
    ui.set_key_columns(app.options.key_columns.join(", ").into());
    ui.set_filter_column(app.options.filter_column.clone().into());
    ui.set_filter_text(app.options.filter_text.clone().into());
    ui.set_filter_exclude(app.options.filter_exclude);
    ui.set_join_kind_index(match app.options.join_kind {
        JoinKind::Left => 0,
        JoinKind::Inner => 1,
        JoinKind::Full => 2,
    });
    ui.set_output_path(app.output_path.clone().into());
    ui.set_app_version(env!("CARGO_PKG_VERSION").into());
    ui.set_rows_metric(format_number(rows).into());
    ui.set_columns_metric(plan.headers.len().to_string().into());
    ui.set_sheets_metric(expected_sheets.to_string().into());
    ui.set_progress(app.progress);
    ui.set_busy(app.busy());
    ui.set_has_sources(!app.sources.is_empty());
    ui.set_can_start(!app.busy() && selected_count > 0 && !plan.headers.is_empty());
    let (status, kind, reveal) = match &app.state {
        AppState::Done { rows, sheets, .. } => (
            format!("合并成功：{} 行，{} 个 Sheet", format_number(*rows), sheets),
            1,
            true,
        ),
        AppState::Error(message) => (message.clone(), 2, false),
        _ if !app.progress_label.is_empty() => (app.progress_label.clone(), 0, false),
        _ if !app.warnings.is_empty() => (
            format!(
                "{} 个文件或工作表未能读取，可在检查报告中查看",
                app.warnings.len()
            ),
            2,
            false,
        ),
        _ => (format!("当前方式：{}", app.options.mode.label()), 0, false),
    };
    ui.set_status_text(status.into());
    ui.set_status_kind(kind);
    ui.set_can_reveal(reveal);
    ui.set_show_progress(app.busy() || !app.progress_label.is_empty());
    ui.set_update_text(app.update_text.clone().into());
}

/// 预览最多展示 100 列，避免极端宽表把界面拖垮。
const PREVIEW_MAX_COLUMNS: usize = 100;

fn preview_rows(preview: Option<&PreviewTable>) -> Vec<PreviewRow> {
    let Some(preview) = preview else {
        return Vec::new();
    };
    let cell_model = |values: &[String]| {
        ModelRc::new(VecModel::from(
            values
                .iter()
                .take(PREVIEW_MAX_COLUMNS)
                .map(|value| SharedString::from(value.as_str()))
                .collect::<Vec<_>>(),
        ))
    };
    let mut rows = vec![PreviewRow {
        line_number: "表头".into(),
        cells: cell_model(&preview.headers),
        header: true,
    }];
    rows.extend(
        preview
            .rows
            .iter()
            .enumerate()
            .map(|(index, row)| PreviewRow {
                line_number: (index + 1).to_string().into(),
                cells: cell_model(row),
                header: false,
            }),
    );
    rows
}

/// 按表头和抽样内容估算每列宽度（中文按两倍宽度计），钳制在 56–320px。
fn preview_column_widths(preview: Option<&PreviewTable>) -> ModelRc<f32> {
    let Some(preview) = preview else {
        return ModelRc::default();
    };
    let mut widths = Vec::with_capacity(preview.headers.len().min(PREVIEW_MAX_COLUMNS));
    for (column, header) in preview.headers.iter().take(PREVIEW_MAX_COLUMNS).enumerate() {
        let mut units = display_units(header);
        for row in preview.rows.iter().take(100) {
            if let Some(cell) = row.get(column) {
                units = units.max(display_units(cell));
            }
        }
        widths.push((units as f32 * 7.0 + 20.0).clamp(56.0, 320.0));
    }
    ModelRc::new(VecModel::from(widths))
}

fn display_units(value: &str) -> usize {
    value
        .chars()
        .map(|ch| if ch.is_ascii() { 1 } else { 2 })
        .sum()
}
fn split_columns(value: &str) -> Vec<String> {
    value
        .split([',', '，'])
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
        .collect()
}
fn transform_from_index(index: i32) -> TransformOp {
    match index {
        1 => TransformOp::Trim,
        2 => TransformOp::Uppercase,
        3 => TransformOp::Lowercase,
        _ => TransformOp::None,
    }
}
fn aggregate_from_index(index: i32) -> AggregateOp {
    match index {
        1 => AggregateOp::Sum,
        2 => AggregateOp::UniqueJoin,
        3 => AggregateOp::TextJoin,
        _ => AggregateOp::First,
    }
}
fn transform_index(value: TransformOp) -> i32 {
    match value {
        TransformOp::None => 0,
        TransformOp::Trim => 1,
        TransformOp::Uppercase => 2,
        TransformOp::Lowercase => 3,
    }
}
fn aggregate_index(value: AggregateOp) -> i32 {
    match value {
        AggregateOp::First => 0,
        AggregateOp::Sum => 1,
        AggregateOp::UniqueJoin => 2,
        AggregateOp::TextJoin => 3,
    }
}
fn paths_from_drop_text(value: &str) -> Vec<PathBuf> {
    value
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .map(|line| {
            let line = line
                .trim_matches('"')
                .strip_prefix("file:///")
                .unwrap_or(line);
            PathBuf::from(percent_decode(line))
        })
        .flat_map(|path| {
            if path.is_dir() {
                collect_folder(&path)
            } else {
                vec![path]
            }
        })
        .filter(|path| supported_file(path))
        .collect()
}
/// 解码拖放文本中的百分号转义（%XX），支持中文等非 ASCII 路径。
fn percent_decode(value: &str) -> String {
    let bytes = value.as_bytes();
    let mut output = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%'
            && index + 2 < bytes.len()
            && bytes[index + 1].is_ascii_hexdigit()
            && bytes[index + 2].is_ascii_hexdigit()
        {
            let high = (bytes[index + 1] as char).to_digit(16).unwrap();
            let low = (bytes[index + 2] as char).to_digit(16).unwrap();
            output.push((high * 16 + low) as u8);
            index += 3;
        } else {
            output.push(bytes[index]);
            index += 1;
        }
    }
    String::from_utf8_lossy(&output).into_owned()
}

fn format_number(value: u64) -> String {
    let text = value.to_string();
    let mut output = String::with_capacity(text.len() + text.len() / 3);
    for (index, ch) in text.chars().enumerate() {
        if index > 0 && (text.len() - index).is_multiple_of(3) {
            output.push(',');
        }
        output.push(ch);
    }
    output
}
fn reveal_in_explorer(path: &Path) {
    #[cfg(target_os = "windows")]
    {
        let _ = std::process::Command::new("explorer.exe")
            .arg(format!("/select,{}", path.display()))
            .spawn();
    }
    #[cfg(not(target_os = "windows"))]
    let _ = path;
}

fn confirm(question: &str) -> bool {
    rfd::MessageDialog::new()
        .set_title("确认操作")
        .set_description(question)
        .set_buttons(rfd::MessageButtons::YesNo)
        .show()
        == rfd::MessageDialogResult::Yes
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

    #[test]
    fn percent_decode_handles_utf8_paths() {
        assert_eq!(percent_decode("C%3A%5C%E4%B8%AD%E6%96%87"), "C:\\中文");
        assert_eq!(percent_decode("a%20b.txt"), "a b.txt");
        assert_eq!(percent_decode("plain.txt"), "plain.txt");
        assert_eq!(percent_decode("bad%2"), "bad%2");
        assert_eq!(percent_decode("100%25"), "100%");
    }
}
