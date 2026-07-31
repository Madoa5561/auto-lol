#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app_icon;
mod champion_picker;
mod config;
mod core;
mod lcu;
#[path = "modern_windows_app.rs"]
mod windows_app;
mod worker;

fn main() {
    if let Err(error) = windows_app::run() {
        windows_app::show_fatal_error(&error);
    }
}
