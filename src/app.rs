use crate::merge::{spawn_merge, MergeEvent, XLSX_MAX_DATA_ROWS};
use crate::model::{build_output_plan, MergeMode, MergeOptions, SourceTable};
use crate::scan::{collect_folder, spawn_scan, supported_file, ScanEvent};
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
            .exact_height(92.0)
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
                    });
                });
            });
    }

    fn show_sidebar(&self, ctx: &egui::Context) {
        egui::SidePanel::left("steps")
            .exact_width(226.0)
            .frame(egui::Frame::new().fill(Color32::from_rgb(248, 250, 252)).inner_margin(22.0))
            .show(ctx, |ui| {
                ui.label(RichText::new("操作流程").size(13.0).strong().color(MUTED));
                ui.add_space(18.0);
                step(ui, "01", "选择数据", !self.sources.is_empty());
                step(ui, "02", "设置合并规则", !self.sources.is_empty());
                step(ui, "03", "导出 XLSX", matches!(self.state, AppState::Done { .. }));
                ui.add_space(18.0);
                ui.separator();
                ui.add_space(14.0);
                ui.label(RichText::new("支持格式").size(12.0).strong().color(MUTED));
                ui.add_space(8.0);
                ui.label(RichText::new("XLSX  XLSM  XLS  XLSB  ODS").size(12.0).color(TEXT));
                ui.label(RichText::new("CSV  TSV").size(12.0).color(TEXT));
                ui.add_space(12.0);
                ui.label(
                    RichText::new("超过 1,048,575 条数据时自动拆分为多个 Sheet。")
                        .size(12.0)
                        .color(MUTED),
                );
            });
    }

    fn show_input_card(&mut self, ui: &mut egui::Ui, panel_height: f32) {
        card(ui, |ui| {
            ui.set_min_height((panel_height - 42.0).max(180.0));
            section_title(ui, "1  选择数据源", "可拖入文件或文件夹；文件夹会递归扫描");
            ui.add_space(14.0);
            ui.horizontal(|ui| {
                let enabled = !self.busy();
                if ui.add_enabled(enabled, primary_button("选择文件夹", 116.0)).clicked() {
                    if let Some(folder) = FileDialog::new().pick_folder() {
                        self.start_folder_scan(folder);
                    }
                }
                if ui.add_enabled(enabled, secondary_button("选择多个文件", 126.0)).clicked() {
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
                                        });
                                    });
                                });
                        }
                    });
            }
        });
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
                MergeMode::Manual => "把不同列名映射到同一个目标列",
            };
            ui.label(RichText::new(mode_help).size(11.0).color(MUTED));
            ui.add_space(12.0);
            ui.checkbox(&mut self.include_source_file, "追加“来源文件”列");
            ui.checkbox(&mut self.include_source_sheet, "追加“来源工作表”列");

            if self.mode == MergeMode::Manual && !self.sources.is_empty() {
                ui.add_space(10.0);
                ui.separator();
                ui.add_space(8.0);
                self.show_manual_mapping(ui, (panel_height - 265.0).max(90.0));
            }
        });
    }

    fn show_manual_mapping(&mut self, ui: &mut egui::Ui, mapping_height: f32) {
        self.selected_mapping_table = self
            .selected_mapping_table
            .min(self.sources.len().saturating_sub(1));
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
                for (index, table) in self.sources.iter().enumerate() {
                    ui.selectable_value(
                        &mut self.selected_mapping_table,
                        index,
                        table.display_name(),
                    );
                }
            });
        ui.add_space(6.0);
        ui.label(
            RichText::new("把不同表的列填写为相同“目标列名”，它们就会合并到同一列。")
                .size(12.0)
                .color(MUTED),
        );
        ui.add_space(8.0);
        let table = &mut self.sources[self.selected_mapping_table];
        egui::ScrollArea::vertical()
            .id_salt(("manual_mapping_scroll", self.selected_mapping_table))
            .max_height(mapping_height)
            .auto_shrink([false, false])
            .show(ui, |ui| {
                for (index, mapping) in table.mappings.iter_mut().enumerate() {
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
        self.show_sidebar(ctx);
        egui::CentralPanel::default()
            .frame(egui::Frame::new().fill(Color32::from_rgb(241, 245, 249)).inner_margin(22.0))
            .show(ctx, |ui| {
                let panel_height = ui.available_height();
                ui.columns(2, |columns| {
                    self.show_input_card(&mut columns[0], panel_height);
                    self.show_rules_card(&mut columns[1], panel_height);
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

fn step(ui: &mut egui::Ui, number: &str, label: &str, complete: bool) {
    ui.horizontal(|ui| {
        let color = if complete { BLUE } else { Color32::from_rgb(203, 213, 225) };
        egui::Frame::new().fill(color).corner_radius(16.0).inner_margin(7.0).show(ui, |ui| {
            ui.label(RichText::new(number).size(11.0).strong().color(Color32::WHITE));
        });
        ui.label(RichText::new(label).size(14.0).strong().color(if complete { TEXT } else { MUTED }));
    });
    ui.add_space(14.0);
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
