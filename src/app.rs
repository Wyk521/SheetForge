use crate::merge::{spawn_merge, MergeEvent, XLSX_MAX_DATA_ROWS};
use crate::model::{build_output_plan, common_header_keys, MergeMode, MergeOptions, SourceTable};
use crate::scan::{collect_folder, spawn_scan, spawn_table_reload, supported_file, ScanEvent};
use eframe::egui::{
    self, Align, Color32, FontData, FontDefinitions, FontFamily, FontId, Layout, RichText,
    Stroke, Vec2,
};
use rfd::FileDialog;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver};
use std::sync::Arc;

const BLUE: Color32 = Color32::from_rgb(37, 99, 235);
const BLUE_DARK: Color32 = Color32::from_rgb(30, 64, 175);
const TEXT: Color32 = Color32::from_rgb(15, 23, 42);
const MUTED: Color32 = Color32::from_rgb(100, 116, 139);
const BORDER: Color32 = Color32::from_rgb(226, 232, 240);
const SOFT_BLUE: Color32 = Color32::from_rgb(239, 246, 255);

enum AppState {
    Ready,
    Scanning,
    Merging,
    Done { output: PathBuf, rows: u64, sheets: usize },
    Error(String),
}

pub struct MergeApp {
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

impl MergeApp {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        configure_fonts(&cc.egui_ctx);
        configure_style(&cc.egui_ctx);
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

    fn busy(&self) -> bool {
        matches!(self.state, AppState::Scanning | AppState::Merging)
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
        let plan = build_output_plan(&self.sources, &options);
        if plan.headers.is_empty() {
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

    fn poll_workers(&mut self, ctx: &egui::Context) {
        let scan_events: Vec<_> = self
            .scan_rx
            .as_ref()
            .map(|rx| rx.try_iter().collect())
            .unwrap_or_default();
        for event in scan_events {
                match event {
                    ScanEvent::Progress { done, total, name } => {
                        self.progress = if total == 0 { 0.0 } else { done as f32 / total as f32 };
                        self.progress_label = format!("正在扫描：{name}");
                    }
                    ScanEvent::Finished { tables, warnings } => {
                        self.sources = tables;
                        self.warnings = warnings;
                        self.selected_mapping_table = 0;
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
                        self.progress_label =
                            format!("已按第 {header_row} 行刷新表头：{name}");
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
                match event {
                    MergeEvent::Progress { current, total, label } => {
                        self.progress = if total == 0 { 0.0 } else { current as f32 / total as f32 };
                        self.progress_label = format!("{label}  ·  {current} / {total} 行");
                    }
                    MergeEvent::Finished { output, rows, sheets } => {
                        self.progress = 1.0;
                        self.progress_label = "合并完成".to_owned();
                        self.state = AppState::Done { output, rows, sheets };
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
        if self.busy() {
            ctx.request_repaint_after(std::time::Duration::from_millis(80));
        }
    }

    fn handle_dropped_files(&mut self, ctx: &egui::Context) {
        if self.busy() {
            return;
        }
        let dropped: Vec<PathBuf> = ctx.input(|input| {
            input
                .raw
                .dropped_files
                .iter()
                .filter_map(|file| file.path.clone())
                .collect()
        });
        if dropped.is_empty() {
            return;
        }
        if dropped.len() == 1 && dropped[0].is_dir() {
            self.start_folder_scan(dropped[0].clone());
        } else {
            self.start_files_scan(dropped);
        }
    }

    fn show_header(&self, ctx: &egui::Context) {
        egui::TopBottomPanel::top("header")
            .exact_height(96.0)
            .frame(egui::Frame::new().fill(BLUE_DARK).inner_margin(20.0))
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.vertical(|ui| {
                        ui.add_space(3.0);
                        ui.label(
                            RichText::new("SheetForge  表格工坊")
                                .size(26.0)
                                .strong()
                                .color(Color32::WHITE),
                        );
                        ui.add_space(5.0);
                        ui.label(
                            RichText::new("快速、安全地合并 Excel 与 CSV")
                                .size(14.0)
                                .color(Color32::from_rgb(191, 219, 254)),
                        );
                    });
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        ui.label(
                            RichText::new("本地处理 · 数据不上传")
                                .size(13.0)
                                .color(Color32::from_rgb(219, 234, 254)),
                        );
                        header_step(
                            ui,
                            "03",
                            "导出 XLSX",
                            matches!(self.state, AppState::Done { .. }),
                        );
                        header_step(ui, "02", "设置规则", !self.sources.is_empty());
                        header_step(ui, "01", "选择数据", !self.sources.is_empty());
                    });
                });
            });
    }

    fn show_input_card(&mut self, ui: &mut egui::Ui, panel_height: f32) {
        let mut reload_request = None;
        let controls_enabled = !self.busy();
        card(ui, |ui| {
            ui.set_min_height((panel_height - 42.0).max(180.0));
            section_title(ui, "1  选择数据源", "逐表选择表头所在行，修改后自动刷新字段");
            ui.add_space(14.0);
            ui.horizontal(|ui| {
                if ui
                    .add_enabled(controls_enabled, primary_button("选择文件夹", 116.0))
                    .clicked()
                {
                    if let Some(folder) = FileDialog::new().pick_folder() {
                        self.start_folder_scan(folder);
                    }
                }
                if ui
                    .add_enabled(controls_enabled, secondary_button("选择多个文件", 126.0))
                    .clicked()
                {
                    if let Some(paths) = FileDialog::new()
                        .add_filter("表格文件", &["xlsx", "xlsm", "xls", "xlsb", "ods", "csv", "tsv"])
                        .pick_files()
                    {
                        self.start_files_scan(paths);
                    }
                }
            });
            ui.add_space(7.0);
            ui.label(RichText::new(&self.input_label).size(12.0).color(MUTED));

            if !self.sources.is_empty() {
                ui.add_space(12.0);
                let selected = self.sources.iter().filter(|table| table.enabled).count();
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new(format!("已识别 {} 个数据表 · 已选 {selected} 个", self.sources.len()))
                            .strong()
                            .color(TEXT),
                    );
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if ui.small_button("全部取消").clicked() {
                            for table in &mut self.sources {
                                table.enabled = false;
                            }
                        }
                        if ui.small_button("全选").clicked() {
                            for table in &mut self.sources {
                                table.enabled = true;
                            }
                        }
                    });
                });
                ui.add_space(8.0);
                let list_height = (panel_height - 205.0).max(90.0);
                egui::ScrollArea::vertical()
                    .id_salt("source_tables_scroll")
                    .max_height(list_height)
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        for (index, table) in self.sources.iter_mut().enumerate() {
                            let fill = if index % 2 == 0 {
                                Color32::from_rgb(248, 250, 252)
                            } else {
                                Color32::WHITE
                            };
                            egui::Frame::new()
                                .fill(fill)
                                .corner_radius(7.0)
                                .inner_margin(9.0)
                                .show(ui, |ui| {
                                    ui.set_min_width(ui.available_width());
                                    ui.horizontal(|ui| {
                                        ui.checkbox(&mut table.enabled, "");
                                        let file = table
                                            .path
                                            .file_name()
                                            .map(|v| v.to_string_lossy())
                                            .unwrap_or_default();
                                        ui.vertical(|ui| {
                                            ui.label(
                                                RichText::new(file.as_ref()).strong().color(TEXT),
                                            )
                                            .on_hover_text(table.path.display().to_string());
                                            ui.horizontal(|ui| {
                                                ui.label(
                                                    RichText::new(format!(
                                                        "{}  ·  {} 行  ·  {} 列",
                                                        table.sheet_name,
                                                        format_number(table.estimated_rows),
                                                        table.headers.len()
                                                    ))
                                                    .size(11.0)
                                                    .color(MUTED),
                                                );
                                                ui.with_layout(
                                                    Layout::right_to_left(Align::Center),
                                                    |ui| {
                                                        ui.label(
                                                            RichText::new("表头行")
                                                                .size(11.0)
                                                                .color(MUTED),
                                                        );
                                                        let mut header_row = table.header_row;
                                                        let response = ui.add_enabled(
                                                            controls_enabled,
                                                            egui::DragValue::new(&mut header_row)
                                                                .range(1..=100_000)
                                                                .speed(1.0),
                                                        );
                                                        if response.changed()
                                                            && header_row != table.header_row
                                                        {
                                                            reload_request =
                                                                Some((index, header_row));
                                                        }
                                                    },
                                                );
                                            });
                                        });
                                    });
                                });
                        }
                    });
            }
        });
        if let Some((index, header_row)) = reload_request {
            self.start_table_reload(index, header_row);
        }
    }

    fn show_rules_card(&mut self, ui: &mut egui::Ui, panel_height: f32) {
        card(ui, |ui| {
            ui.set_min_height((panel_height - 42.0).max(180.0));
            section_title(ui, "2  设置合并规则", "表头匹配不区分大小写，并会自动去除首尾空格");
            ui.add_space(14.0);
            ui.horizontal(|ui| {
                ui.selectable_value(&mut self.mode, MergeMode::Union, "并集");
                ui.selectable_value(&mut self.mode, MergeMode::Intersection, "交集");
                ui.selectable_value(&mut self.mode, MergeMode::Manual, "手动映射");
            });
            let mode_help = match self.mode {
                MergeMode::Union => "保留所有数据表中出现过的列",
                MergeMode::Intersection => "只保留所有数据表共有的列",
                MergeMode::Manual => "以并集为基础，只需修正拼写不一致或含义相同的列名",
            };
            ui.label(RichText::new(mode_help).size(11.0).color(MUTED));
            ui.add_space(12.0);
            ui.checkbox(&mut self.include_source_file, "追加“来源文件”列");
            ui.checkbox(&mut self.include_source_sheet, "追加“来源工作表”列");

            if self.mode == MergeMode::Manual && self.sources.iter().any(|table| table.enabled) {
                ui.add_space(10.0);
                ui.separator();
                ui.add_space(8.0);
                self.show_manual_mapping(ui, (panel_height - 265.0).max(90.0));
            }
        });
    }

    fn show_manual_mapping(&mut self, ui: &mut egui::Ui, mapping_height: f32) {
        let enabled_indices: Vec<usize> = self
            .sources
            .iter()
            .enumerate()
            .filter_map(|(index, table)| table.enabled.then_some(index))
            .collect();
        let Some(&first_enabled) = enabled_indices.first() else {
            return;
        };
        if !enabled_indices.contains(&self.selected_mapping_table) {
            self.selected_mapping_table = first_enabled;
        }
        let common_headers = common_header_keys(&self.sources);
        let common_count = self.sources[self.selected_mapping_table]
            .mappings
            .iter()
            .filter(|mapping| common_headers.contains(&crate::model::header_key(&mapping.source_name)))
            .count();
        ui.horizontal(|ui| {
            ui.label(RichText::new("当前数据表").strong().color(TEXT));
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if ui.small_button("恢复本表默认").clicked() {
                    let table = &mut self.sources[self.selected_mapping_table];
                    for mapping in &mut table.mappings {
                        mapping.target_name = mapping.source_name.clone();
                        mapping.enabled = true;
                    }
                }
            });
        });
        egui::ComboBox::from_id_salt("mapping_table")
            .width(ui.available_width())
            .selected_text(self.sources[self.selected_mapping_table].display_name())
            .show_ui(ui, |ui| {
                for index in &enabled_indices {
                    let table = &self.sources[*index];
                    ui.selectable_value(
                        &mut self.selected_mapping_table,
                        *index,
                        table.display_name(),
                    );
                }
            });
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            let label = if self.hide_common_mappings {
                format!("显示共有字段（已隐藏 {common_count} 项）")
            } else {
                format!("隐藏共有字段（{common_count} 项）")
            };
            if ui
                .add(secondary_button(&label, 188.0))
                .on_hover_text("共有字段会按原列名自动合并，通常无需手动修改")
                .clicked()
            {
                self.hide_common_mappings = !self.hide_common_mappings;
            }
        });
        ui.label(
            RichText::new("将写错或含义相同的列改成同一目标列名；其余字段仍按并集输出。")
                .size(12.0)
                .color(MUTED),
        );
        ui.add_space(8.0);
        let hide_common_mappings = self.hide_common_mappings;
        let table = &mut self.sources[self.selected_mapping_table];
        egui::ScrollArea::vertical()
            .id_salt(("manual_mapping_scroll", self.selected_mapping_table))
            .max_height(mapping_height)
            .auto_shrink([false, false])
            .show(ui, |ui| {
                for (index, mapping) in table
                    .mappings
                    .iter_mut()
                    .filter(|mapping| {
                        !hide_common_mappings
                            || !common_headers.contains(&crate::model::header_key(&mapping.source_name))
                    })
                    .enumerate()
                {
                    let fill = if index % 2 == 0 {
                        Color32::from_rgb(248, 250, 252)
                    } else {
                        Color32::WHITE
                    };
                    egui::Frame::new()
                        .fill(fill)
                        .corner_radius(7.0)
                        .inner_margin(9.0)
                        .show(ui, |ui| {
                            ui.set_min_width(ui.available_width());
                            ui.checkbox(&mut mapping.enabled, &mapping.source_name);
                            ui.add_space(3.0);
                            ui.label(RichText::new("映射到").size(11.0).color(MUTED));
                            let edit_width = ui.available_width();
                            ui.add_enabled(
                                mapping.enabled,
                                egui::TextEdit::singleline(&mut mapping.target_name)
                                    .desired_width(edit_width)
                                    .hint_text("留空则不输出"),
                            );
                        });
                }
            });
    }

    fn show_output_panel(&mut self, ctx: &egui::Context) {
        let options = MergeOptions {
            mode: self.mode,
            include_source_file: self.include_source_file,
            include_source_sheet: self.include_source_sheet,
        };
        let plan = build_output_plan(&self.sources, &options);
        let rows: u64 = self
            .sources
            .iter()
            .filter(|table| table.enabled)
            .map(|table| table.estimated_rows)
            .sum();
        let expected_sheets = ((rows.max(1) - 1) / XLSX_MAX_DATA_ROWS as u64) + 1;

        egui::TopBottomPanel::bottom("output_panel")
            .exact_height(202.0)
            .frame(
                egui::Frame::new()
                    .fill(Color32::WHITE)
                    .stroke(Stroke::new(1.0, BORDER))
                    .inner_margin(16.0),
            )
            .show(ctx, |ui| {
                section_title(ui, "3  导出结果", "固定操作区，无需滚动页面即可开始合并");
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    ui.label(RichText::new("输出位置").strong().color(TEXT));
                    let path_width = (ui.available_width() - 105.0).max(180.0);
                    ui.add(
                        egui::TextEdit::singleline(&mut self.output_path)
                            .desired_width(path_width)
                            .hint_text("请选择输出文件"),
                    );
                    if ui
                        .add_enabled(!self.busy(), secondary_button("浏览…", 84.0))
                        .clicked()
                    {
                        if let Some(path) = FileDialog::new()
                            .add_filter("Excel 工作簿", &["xlsx"])
                            .set_file_name("合并结果.xlsx")
                            .save_file()
                        {
                            self.output_path = path.display().to_string();
                        }
                    }
                });
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    compact_metric(ui, "预计行", &format_number(rows));
                    ui.separator();
                    compact_metric(ui, "输出列", &plan.headers.len().to_string());
                    ui.separator();
                    compact_metric(ui, "Sheet", &expected_sheets.to_string());
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        let can_start = !self.busy()
                            && self.sources.iter().any(|table| table.enabled)
                            && !plan.headers.is_empty();
                        if ui
                            .add_enabled(can_start, primary_button("开始合并", 150.0))
                            .clicked()
                        {
                            self.start_merge();
                        }
                        if matches!(self.state, AppState::Merging)
                            && ui.add(secondary_button("取消", 80.0)).clicked()
                        {
                            self.cancel.store(true, Ordering::Relaxed);
                            self.progress_label = "正在取消…".to_owned();
                        }
                    });
                });

                if self.busy() || !self.progress_label.is_empty() {
                    ui.add_space(8.0);
                    ui.add(
                        egui::ProgressBar::new(self.progress.clamp(0.0, 1.0))
                            .animate(self.busy())
                            .show_percentage(),
                    );
                    ui.add_space(3.0);
                }

                match &self.state {
                    AppState::Done { output, rows, sheets } => {
                        ui.horizontal(|ui| {
                            ui.label(
                                RichText::new(format!(
                                    "合并成功：{} 行，{} 个 Sheet",
                                    format_number(*rows),
                                    sheets
                                ))
                                .strong()
                                .color(Color32::from_rgb(22, 101, 52)),
                            );
                            if ui.small_button("在文件夹中显示").clicked() {
                                reveal_in_explorer(output);
                            }
                        });
                    }
                    AppState::Error(message) => {
                        ui.label(
                            RichText::new(message).color(Color32::from_rgb(153, 27, 27)),
                        );
                    }
                    _ if !self.progress_label.is_empty() => {
                        ui.label(RichText::new(&self.progress_label).size(12.0).color(MUTED));
                    }
                    _ => {
                        let warning = if self.warnings.is_empty() {
                            format!("当前模式：{}", self.mode.label())
                        } else {
                            format!(
                                "{} 个文件或工作表未能读取 · 当前模式：{}",
                                self.warnings.len(),
                                self.mode.label()
                            )
                        };
                        ui.label(RichText::new(warning).size(12.0).color(MUTED));
                    }
                }
            });
    }
}

impl eframe::App for MergeApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.poll_workers(ctx);
        self.handle_dropped_files(ctx);
        self.show_header(ctx);
        self.show_output_panel(ctx);
        egui::CentralPanel::default()
            .frame(egui::Frame::new().fill(Color32::from_rgb(241, 245, 249)).inner_margin(22.0))
            .show(ctx, |ui| {
                let panel_height = ui.available_height();
                let gap = 14.0;
                let content_width = (ui.available_width() - gap).max(640.0);
                let input_width = content_width * 0.44;
                let rules_width = content_width - input_width;
                ui.horizontal(|ui| {
                    ui.allocate_ui_with_layout(
                        Vec2::new(input_width, panel_height),
                        Layout::top_down(Align::Min),
                        |ui| self.show_input_card(ui, panel_height),
                    );
                    ui.add_space(gap);
                    ui.allocate_ui_with_layout(
                        Vec2::new(rules_width, panel_height),
                        Layout::top_down(Align::Min),
                        |ui| self.show_rules_card(ui, panel_height),
                    );
                });
            });
    }
}

fn configure_fonts(ctx: &egui::Context) {
    let candidates = [
        r"C:\Windows\Fonts\msyh.ttc",
        r"C:\Windows\Fonts\simhei.ttf",
        r"C:\Windows\Fonts\simsun.ttc",
    ];
    let Some(bytes) = candidates.iter().find_map(|path| std::fs::read(path).ok()) else {
        return;
    };
    let mut fonts = FontDefinitions::default();
    fonts
        .font_data
        .insert("windows_cjk".to_owned(), FontData::from_owned(bytes).into());
    fonts
        .families
        .entry(FontFamily::Proportional)
        .or_default()
        .insert(0, "windows_cjk".to_owned());
    fonts
        .families
        .entry(FontFamily::Monospace)
        .or_default()
        .push("windows_cjk".to_owned());
    ctx.set_fonts(fonts);
}

fn configure_style(ctx: &egui::Context) {
    let mut visuals = egui::Visuals::light();
    visuals.panel_fill = Color32::from_rgb(241, 245, 249);
    visuals.window_fill = Color32::WHITE;
    visuals.selection.bg_fill = BLUE;
    visuals.widgets.inactive.bg_fill = Color32::WHITE;
    visuals.widgets.inactive.weak_bg_fill = Color32::from_rgb(248, 250, 252);
    visuals.widgets.inactive.bg_stroke = Stroke::new(1.0, BORDER);
    visuals.widgets.hovered.bg_fill = SOFT_BLUE;
    visuals.widgets.hovered.bg_stroke = Stroke::new(1.0, BLUE);
    ctx.set_visuals(visuals);
    ctx.style_mut(|style| {
        style.spacing.item_spacing = Vec2::new(9.0, 8.0);
        style.spacing.button_padding = Vec2::new(14.0, 8.0);
        style.text_styles.insert(egui::TextStyle::Body, FontId::proportional(14.0));
        style.text_styles.insert(egui::TextStyle::Button, FontId::proportional(14.0));
    });
}

fn card<R>(ui: &mut egui::Ui, content: impl FnOnce(&mut egui::Ui) -> R) -> R {
    egui::Frame::new()
        .fill(Color32::WHITE)
        .stroke(Stroke::new(1.0, BORDER))
        .corner_radius(12.0)
        .inner_margin(20.0)
        .show(ui, content)
        .inner
}

fn section_title(ui: &mut egui::Ui, title: &str, subtitle: &str) {
    ui.horizontal(|ui| {
        ui.label(RichText::new(title).size(18.0).strong().color(TEXT));
        ui.add_space(8.0);
        ui.label(RichText::new(subtitle).size(12.0).color(MUTED));
    });
}

fn header_step(ui: &mut egui::Ui, number: &str, label: &str, complete: bool) {
    let fill = if complete {
        Color32::from_rgb(59, 130, 246)
    } else {
        Color32::from_rgb(51, 65, 85)
    };
    egui::Frame::new()
        .fill(fill)
        .stroke(Stroke::new(1.0, Color32::from_rgb(96, 165, 250)))
        .corner_radius(18.0)
        .inner_margin(8.0)
        .show(ui, |ui| {
            ui.label(
                RichText::new(format!("{number}  {label}"))
                    .size(12.0)
                    .strong()
                    .color(Color32::WHITE),
            );
        });
}

fn compact_metric(ui: &mut egui::Ui, label: &str, value: &str) {
    ui.label(RichText::new(label).size(11.0).color(MUTED));
    ui.label(RichText::new(value).strong().color(TEXT));
}

fn primary_button(label: &str, width: f32) -> egui::Button<'_> {
    egui::Button::new(RichText::new(label).strong().color(Color32::WHITE))
        .fill(BLUE)
        .stroke(Stroke::NONE)
        .min_size(Vec2::new(width, 38.0))
}

fn secondary_button(label: &str, width: f32) -> egui::Button<'_> {
    egui::Button::new(RichText::new(label).color(TEXT))
        .fill(Color32::WHITE)
        .stroke(Stroke::new(1.0, BORDER))
        .min_size(Vec2::new(width, 38.0))
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
        let _ = std::process::Command::new("explorer.exe").arg(argument).spawn();
    }
    #[cfg(not(target_os = "windows"))]
    let _ = path;
}
