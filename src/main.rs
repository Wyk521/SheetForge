#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]

mod app;
mod merge;
mod model;
mod scan;

use app::MergeApp;
use eframe::egui;

fn main() -> eframe::Result<()> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("SheetForge 表格工坊")
            .with_inner_size([1120.0, 780.0])
            .with_min_inner_size([900.0, 640.0]),
        renderer: eframe::Renderer::Glow,
        ..Default::default()
    };
    eframe::run_native(
        "SheetForge 表格工坊",
        options,
        Box::new(|cc| Ok(Box::new(MergeApp::new(cc)))),
    )
}

