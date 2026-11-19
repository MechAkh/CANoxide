// No console window in the release build.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() -> eframe::Result {
    canoxide::ui::run()
}
