//! Desktop front-end: the egui window around [`llegion::app::App`].
#![cfg_attr(all(not(debug_assertions), windows), windows_subsystem = "windows")]

use llegion::app;
#[cfg(debug_assertions)]
use llegion::screenshots;

fn main() -> eframe::Result {
    #[allow(unused_mut)]
    let mut size = [1440.0, 920.0];
    #[cfg(debug_assertions)]
    if let Some(s) = screenshots::Capture::window_size() {
        size = s;
    }
    let options = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default()
            .with_title("Llegion")
            .with_icon(app_icon())
            .with_inner_size(size)
            .with_min_inner_size([960.0, 640.0]),
        ..Default::default()
    };
    eframe::run_native(
        "Llegion",
        options,
        Box::new(|cc| Ok(Box::new(app::App::new(cc)))),
    )
}

fn app_icon() -> eframe::egui::IconData {
    let decoder = png::Decoder::new(std::io::Cursor::new(include_bytes!("../assets/logo.png")));
    let mut reader = decoder.read_info().expect("embedded app icon PNG");
    let mut rgba = vec![0; reader.output_buffer_size().expect("icon dimensions")];
    let info = reader
        .next_frame(&mut rgba)
        .expect("embedded app icon frame");
    rgba.truncate(info.buffer_size());
    eframe::egui::IconData {
        rgba,
        width: info.width,
        height: info.height,
    }
}
