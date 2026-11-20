//! CapDraft TTS desktop app (port of `ui/main_window.py` and `ui/settings_dialog.py`).
//!
//! The window is Slint (`ui/app.slint`); all work runs in `capdraft-core` on
//! background threads. Log lines, progress and per-caption results are queued
//! by the workers and applied to the window every 100 ms, so thousands of
//! captions never flood the UI thread.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;

#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

/// The Slint-generated window code (it has its own idioms, unwraps included).
#[allow(clippy::all, clippy::pedantic, clippy::unwrap_used, unsafe_code)]
mod ui {
    slint::include_modules!();
}
use ui::*;

fn main() -> Result<(), slint::PlatformError> {
    app::run()
}
