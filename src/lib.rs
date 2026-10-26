//! llegion: a UI-independent core plus two front-ends behind cargo features,
//! the egui desktop app (`gui`, binary `llegion`) and the terminal UI (`tui`, binary `llegion-tui`).

#[cfg(feature = "gui")]
#[macro_use]
mod icons;

pub mod backend;
pub mod chat_store;
pub mod config;
pub mod control;
pub mod ferron;
pub mod gpu;
pub mod logs;
pub mod ollama;
pub mod process;
pub mod util;

#[cfg(feature = "gui")]
pub mod app;
#[cfg(all(feature = "gui", debug_assertions))]
pub mod screenshots;
#[cfg(feature = "gui")]
mod views;

#[cfg(feature = "tui")]
pub mod tui;

#[cfg(test)]
mod e2e;
