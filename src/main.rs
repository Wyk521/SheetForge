#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]

slint::include_modules!();

mod app;
mod config;
mod inspect;
mod merge;
mod model;
mod scan;

fn main() -> Result<(), slint::PlatformError> {
    app::run()
}
