use crate::merge::{spawn_merge, MergeEvent, XLSX_MAX_DATA_ROWS};
use crate::model::{
    build_output_plan, common_header_keys, header_key, MergeMode, MergeOptions, SourceTable,
};
use crate::scan::{collect_folder, spawn_scan, spawn_table_reload, supported_file, ScanEvent};
use crate::{AppWindow, MappingRow, SourceRow};
use rfd::FileDialog;
use slint::{ComponentHandle, ModelRc, SharedString, Timer, TimerMode, VecModel};
use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver};
use std::sync::Arc;
use std::time::Duration;

enum AppState {
    Ready,
    Scanning,
    Merging,
    Done {
        output: PathBuf,
        rows: u64,
        sheets: usize,
    },
    Error(String),
}

struct MergeApp {
    sources: Vec<SourceTable>,
    input_label: String,
    output_path: String,
    mode: MergeMode,
    include_source_file: bool,
    include_source_sheet: bool,
    hide_common_mappings: bool,
    selected_mapping_table: usize,
    scan_rx: Option<Receiver<ScanEvent>>,
    merge_rx: Option<Receiver<MergeEvent>>,
    cancel: Arc<AtomicBool>,
    state: AppState,
    progress: f32,
    progress_label: String,
    warnings: Vec<String>,
}

impl Default for MergeApp {
    fn default() -> Self {
        Self {
            sources: Vec::new(),
            input_label: "尚未选择文件或文件夹".to_owned(),
            output_path: String::new(),
            mode: MergeMode::Union,
            include_source_file: false,
            include_source_sheet: false,
            hide_common_mappings: false,
            selected_mapping_table: 0,
            scan_rx: None,
            merge_rx: None,
            cancel: Arc::new(AtomicBool::new(false)),
            state: AppState::Ready,
            progress: 0.0,
            progress_label: String::new(),
            warnings: Vec::new(),
        }
    }
}

impl MergeApp {
    fn busy(&self) -> bool {
        matches!(self.state, AppState::Scanning | AppState::Merging)
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

    fn start_folder_scan(&mut self, folder: PathBuf) {
        let (tx, rx) = mpsc::channel();
        self.scan_rx = Some(rx);
        self.state = AppState::Scanning;
        self.progress = 0.0;
        self.progress_label = "正在递归查找工作簿…".to_owned();
        self.input_label = folder.display().to_string();
        self.output_path = folder.join("合并结果.xlsx").display().to_string();
        self.warnings.clear();
        std::thread::spawn(move || {
            let default_output = folder.join("合并结果.xlsx");
            let mut paths = collect_folder(&folder);
            paths.retain(|path| path != &default_output);
            spawn_scan(paths, tx);
        });
    }

    fn start_files_scan(&mut self, mut paths: Vec<PathBuf>) {
        paths.retain(|path| supported_file(path));
        paths.sort();
        paths.dedup();
        if paths.is_empty() {
            self.state = AppState::Error("没有找到支持的表格文件".to_owned());
            return;
        }
        let output_folder = paths.first().and_then(|path| path.parent()).map(Path::to_owned);
        self.input_label = format!("已选择 {} 个文件", paths.len());
        if let Some(folder) = output_folder {
            self.output_path = folder.join("合并结果.xlsx").display().to_string();
        }
        let (tx, rx) = mpsc::channel();
        self.scan_rx = Some(rx);
        self.state = AppState::Scanning;
        self.progress = 0.0;
        self.progress_label = "正在读取表头…".to_owned();
        self.warnings.clear();
        spawn_scan(paths, tx);
    }

    fn start_table_reload(&mut self, index: usize, header_row: usize) {
        let Some(source) = self.sources.get(index).cloned() else {
            return;
        };
        let (tx, rx) = mpsc::channel();
        self.scan_rx = Some(rx);
        self.state = AppState::Scanning;
        self.progress = 0.15;
        self.progress_label = format!(
            "正在按第 {header_row} 行重新读取表头：{}",
            source.display_name()
        );
        spawn_table_reload(index, source, header_row, tx);
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
        let options = MergeOptions {
            mode: self.mode,
            include_source_file: self.include_source_file,
            include_source_sheet: self.include_source_sheet,
        };
        if build_output_plan(&self.sources, &options).headers.is_empty() {
            self.state = AppState::Error("没有可输出的列，请检查所选表和列映射".to_owned());
            return;
        }

        let (tx, rx) = mpsc::channel();
        self.merge_rx = Some(rx);
        self.cancel = Arc::new(AtomicBool::new(false));
        self.state = AppState::Merging;
        self.progress = 0.0;
        self.progress_label = "正在准备输出工作簿…".to_owned();
        spawn_merge(
            self.sources.clone(),
            options,
            output,
            tx,
            self.cancel.clone(),
        );
    }

    fn poll_workers(&mut self) -> bool {
        let mut changed = false;
        let scan_events: Vec<_> = self
            .scan_rx
            .as_ref()
            .map(|rx| rx.try_iter().collect())
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
                    self.progress = 1.0;
                    self.progress_label = format!("已识别 {} 个数据表", self.sources.len());
                    self.state = AppState::Ready;
                    self.scan_rx = None;
                }
                ScanEvent::TableReloaded { index, table } => {
                    let name = table.display_name();
                    let header_row = table.header_row;
                    if let Some(slot) = self.sources.get_mut(index) {
                        *slot = table;
                    }
                    self.selected_mapping_table = index;
                    self.progress = 1.0;
                    self.progress_label = format!("已按第 {header_row} 行刷新表头：{name}");
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

        let merge_events: Vec<_> = self
            .merge_rx
            .as_ref()
            .map(|rx| rx.try_iter().collect())
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
                MergeEvent::Finished {
                    output,
                    rows,
                    sheets,
                } => {
                    self.progress = 1.0;
                    self.progress_label = "合并完成".to_owned();
                    self.state = AppState::Done {
                        output,
                        rows,
                        sheets,
                    };
                    self.merge_rx = None;
                }
                MergeEvent::Cancelled => {
                    self.progress_label = "已取消".to_owned();
                    self.state = AppState::Ready;
                    self.merge_rx = None;
                }
                MergeEvent::Failed(message) => {
                    self.state = AppState::Error(message);
                    self.merge_rx = None;
                }
            }
        }
        changed
    }
}

pub fn run() -> Result<(), slint::PlatformError> {
    let ui = AppWindow::new()?;
    let state = Rc::new(RefCell::new(MergeApp::default()));
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

    ui.run()
}

fn install_callbacks(ui: &AppWindow, state: Rc<RefCell<MergeApp>>) {
    let weak = ui.as_weak();
    let callback_state = state.clone();
    ui.on_choose_folder(move || {
        if callback_state.borrow().busy() {
            return;
        }
        if let Some(folder) = FileDialog::new().pick_folder() {
            callback_state.borrow_mut().start_folder_scan(folder);
            sync_weak(&weak, &callback_state);
        }
    });

    let weak = ui.as_weak();
    let callback_state = state.clone();
    ui.on_choose_files(move || {
        if callback_state.borrow().busy() {
            return;
        }
        if let Some(paths) = FileDialog::new()
            .add_filter(
                "表格文件",
                &["xlsx", "xlsm", "xls", "xlsb", "ods", "csv", "tsv"],
            )
            .pick_files()
        {
            callback_state.borrow_mut().start_files_scan(paths);
            sync_weak(&weak, &callback_state);
        }
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
    ui.on_header_row_changed(move |index, row| {
        callback_state
            .borrow_mut()
            .start_table_reload(index.max(0) as usize, row.max(1) as usize);
        sync_weak(&weak, &callback_state);
    });

    let weak = ui.as_weak();
    let callback_state = state.clone();
    ui.on_mode_changed(move |mode| {
        callback_state.borrow_mut().mode = match mode {
            1 => MergeMode::Intersection,
            2 => MergeMode::Manual,
            _ => MergeMode::Union,
        };
        sync_weak(&weak, &callback_state);
    });

    let weak = ui.as_weak();
    let callback_state = state.clone();
    ui.on_include_source_file_changed(move |enabled| {
        callback_state.borrow_mut().include_source_file = enabled;
        sync_weak(&weak, &callback_state);
    });

    let weak = ui.as_weak();
    let callback_state = state.clone();
    ui.on_include_source_sheet_changed(move |enabled| {
        callback_state.borrow_mut().include_source_sheet = enabled;
        sync_weak(&weak, &callback_state);
    });

    let weak = ui.as_weak();
    let callback_state = state.clone();
    ui.on_mapping_table_changed(move |position| {
        let mut app = callback_state.borrow_mut();
        if let Some(actual_index) = app.enabled_indices().get(position.max(0) as usize) {
            app.selected_mapping_table = *actual_index;
        }
        drop(app);
        sync_weak(&weak, &callback_state);
    });

    let weak = ui.as_weak();
    let callback_state = state.clone();
    ui.on_mapping_changed(move |mapping_index, enabled, target| {
        let mut app = callback_state.borrow_mut();
        let table_index = app.selected_mapping_table;
        if let Some(mapping) = app
            .sources
            .get_mut(table_index)
            .and_then(|table| table.mappings.get_mut(mapping_index.max(0) as usize))
        {
            mapping.enabled = enabled;
            mapping.target_name = target.to_string();
        }
        drop(app);
        sync_weak(&weak, &callback_state);
    });

    let weak = ui.as_weak();
    let callback_state = state.clone();
    ui.on_reset_mapping(move || {
        let mut app = callback_state.borrow_mut();
        let table_index = app.selected_mapping_table;
        if let Some(table) = app.sources.get_mut(table_index) {
            for mapping in &mut table.mappings {
                mapping.target_name = mapping.source_name.clone();
                mapping.enabled = true;
            }
        }
        drop(app);
        sync_weak(&weak, &callback_state);
    });

    let weak = ui.as_weak();
    let callback_state = state.clone();
    ui.on_toggle_common_fields(move || {
        let hidden = callback_state.borrow().hide_common_mappings;
        callback_state.borrow_mut().hide_common_mappings = !hidden;
        sync_weak(&weak, &callback_state);
    });

    let callback_state = state.clone();
    ui.on_output_path_changed(move |path| {
        callback_state.borrow_mut().output_path = path.to_string();
    });

    let weak = ui.as_weak();
    let callback_state = state.clone();
    ui.on_choose_output(move || {
        if callback_state.borrow().busy() {
            return;
        }
        if let Some(path) = FileDialog::new()
            .add_filter("Excel 工作簿", &["xlsx"])
            .set_file_name("合并结果.xlsx")
            .save_file()
        {
            callback_state.borrow_mut().output_path = path.display().to_string();
            sync_weak(&weak, &callback_state);
        }
    });

    let weak = ui.as_weak();
    let callback_state = state.clone();
    ui.on_start_merge(move || {
        callback_state.borrow_mut().start_merge();
        sync_weak(&weak, &callback_state);
    });

    let weak = ui.as_weak();
    let callback_state = state.clone();
    ui.on_cancel_merge(move || {
        let mut app = callback_state.borrow_mut();
        app.cancel.store(true, Ordering::Relaxed);
        app.progress_label = "正在取消…".to_owned();
        drop(app);
        sync_weak(&weak, &callback_state);
    });

    let callback_state = state;
    ui.on_reveal_output(move || {
        if let AppState::Done { output, .. } = &callback_state.borrow().state {
            reveal_in_explorer(output);
        }
    });
}

fn sync_weak(weak: &slint::Weak<AppWindow>, state: &Rc<RefCell<MergeApp>>) {
    if let Some(ui) = weak.upgrade() {
        sync_ui(&ui, &state.borrow());
    }
}

fn sync_ui(ui: &AppWindow, app: &MergeApp) {
    let selected_count = app.sources.iter().filter(|table| table.enabled).count();
    let source_rows: Vec<SourceRow> = app
        .sources
        .iter()
        .map(|table| SourceRow {
            enabled: table.enabled,
            file_name: table
                .path
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_else(|| table.path.display().to_string())
                .into(),
            sheet_name: table.sheet_name.clone().into(),
            detail: format!(
                "{} 行  ·  {} 列",
                format_number(table.estimated_rows),
                table.headers.len()
            )
            .into(),
            header_row: table.header_row as i32,
        })
        .collect();
    ui.set_sources(ModelRc::new(VecModel::from(source_rows)));
    ui.set_input_label(app.input_label.clone().into());
    ui.set_sources_summary(
        format!("已识别 {} 个数据表 · 已选 {selected_count} 个", app.sources.len()).into(),
    );

    let enabled_indices = app.enabled_indices();
    let mapping_names: Vec<SharedString> = enabled_indices
        .iter()
        .map(|index| app.sources[*index].display_name().into())
        .collect();
    ui.set_mapping_tables(ModelRc::new(VecModel::from(mapping_names)));
    let mapping_position = enabled_indices
        .iter()
        .position(|index| *index == app.selected_mapping_table)
        .unwrap_or(0);
    ui.set_mapping_table_index(mapping_position as i32);

    let common_headers = common_header_keys(&app.sources);
    let (mapping_rows, common_count) = app
        .sources
        .get(app.selected_mapping_table)
        .map(|table| {
            let common_count = table
                .mappings
                .iter()
                .filter(|mapping| common_headers.contains(&header_key(&mapping.source_name)))
                .count();
            let rows = table
                .mappings
                .iter()
                .enumerate()
                .filter(|(_, mapping)| {
                    !app.hide_common_mappings
                        || !common_headers.contains(&header_key(&mapping.source_name))
                })
                .map(|(index, mapping)| MappingRow {
                    enabled: mapping.enabled,
                    mapping_index: index as i32,
                    source_name: mapping.source_name.clone().into(),
                    target_name: mapping.target_name.clone().into(),
                })
                .collect::<Vec<_>>();
            (rows, common_count)
        })
        .unwrap_or_default();
    ui.set_mappings(ModelRc::new(VecModel::from(mapping_rows)));
    ui.set_common_fields_label(
        if app.hide_common_mappings {
            format!("显示共有字段（已隐藏 {common_count} 项）")
        } else {
            format!("隐藏共有字段（{common_count} 项）")
        }
        .into(),
    );

    let options = MergeOptions {
        mode: app.mode,
        include_source_file: app.include_source_file,
        include_source_sheet: app.include_source_sheet,
    };
    let plan = build_output_plan(&app.sources, &options);
    let rows: u64 = app
        .sources
        .iter()
        .filter(|table| table.enabled)
        .map(|table| table.estimated_rows)
        .sum();
    let expected_sheets = ((rows.max(1) - 1) / XLSX_MAX_DATA_ROWS as u64) + 1;

    ui.set_mode_index(match app.mode {
        MergeMode::Union => 0,
        MergeMode::Intersection => 1,
        MergeMode::Manual => 2,
    });
    ui.set_include_source_file(app.include_source_file);
    ui.set_include_source_sheet(app.include_source_sheet);
    ui.set_output_path(app.output_path.clone().into());
    ui.set_rows_metric(format_number(rows).into());
    ui.set_columns_metric(plan.headers.len().to_string().into());
    ui.set_sheets_metric(expected_sheets.to_string().into());
    ui.set_progress(app.progress);
    ui.set_busy(app.busy());
    ui.set_has_sources(!app.sources.is_empty());
    ui.set_can_start(
        !app.busy()
            && app.sources.iter().any(|table| table.enabled)
            && !plan.headers.is_empty(),
    );

    let (status_text, status_kind, can_reveal) = match &app.state {
        AppState::Done {
            rows,
            sheets,
            ..
        } => (
            format!("合并成功：{} 行，{} 个 Sheet", format_number(*rows), sheets),
            1,
            true,
        ),
        AppState::Error(message) => (message.clone(), 2, false),
        _ if !app.progress_label.is_empty() => (app.progress_label.clone(), 0, false),
        _ if !app.warnings.is_empty() => (
            format!("{} 个文件或工作表未能读取", app.warnings.len()),
            2,
            false,
        ),
        _ => (format!("当前模式：{}", app.mode.label()), 0, false),
    };
    ui.set_status_text(status_text.into());
    ui.set_status_kind(status_kind);
    ui.set_can_reveal(can_reveal);
    ui.set_show_progress(app.busy() || !app.progress_label.is_empty());
}

fn format_number(value: u64) -> String {
    let text = value.to_string();
    let mut output = String::with_capacity(text.len() + text.len() / 3);
    for (index, ch) in text.chars().enumerate() {
        if index > 0 && (text.len() - index) % 3 == 0 {
            output.push(',');
        }
        output.push(ch);
    }
    output
}

fn reveal_in_explorer(path: &Path) {
    #[cfg(target_os = "windows")]
    {
        let argument = format!("/select,{}", path.display());
        let _ = std::process::Command::new("explorer.exe")
            .arg(argument)
            .spawn();
    }
    #[cfg(not(target_os = "windows"))]
    let _ = path;
}
