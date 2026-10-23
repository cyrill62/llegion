#![cfg_attr(all(not(debug_assertions), windows), windows_subsystem = "windows")]

#[macro_use]
mod icons;

mod app;
mod backend;
mod caddy;
mod config;
mod gpu;
mod logs;
mod ollama;
mod process;
#[cfg(debug_assertions)]
mod screenshots;
mod util;
mod views;

#[cfg(test)]
mod e2e;

fn main() -> eframe::Result {
    #[allow(unused_mut)]
    let mut size = [1440.0, 920.0];
    #[cfg(debug_assertions)]
    if let Some(s) = screenshots::Capture::window_size() {
        size = s;
    }
    let options = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default()
            .with_title("Ollama Control")
            .with_inner_size(size)
            .with_min_inner_size([960.0, 640.0]),
        ..Default::default()
    };
    eframe::run_native(
        "Ollama Control",
        options,
        Box::new(|cc| Ok(Box::new(app::App::new(cc)))),
    )
}
