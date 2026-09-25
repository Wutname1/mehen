// Prevents an extra console window on Windows in release builds.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    // Decided before Tauri starts: a background check never opens a window
    // or takes the single-instance slot the app uses.
    let args: Vec<String> = std::env::args().collect();
    if mehen_lib::background_check_requested(&args) {
        std::process::exit(mehen_lib::run_background_check(&args));
    }
    mehen_lib::run()
}
