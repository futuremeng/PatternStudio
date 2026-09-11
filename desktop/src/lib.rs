//! PatternStudio desktop (Tauri): calls the tracing engine in-process.
//!
//! The same React frontend used by the web server runs inside the Tauri
//! webview; when `window.__TAURI_INTERNALS__` is present the frontend
//! switches from HTTP calls to the commands in [`commands`].

pub mod commands;

pub fn run() {
    use commands::{generate_pattern, list_patterns, save_svg, trace_bytes};

    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![
            trace_bytes,
            generate_pattern,
            list_patterns,
            save_svg
        ])
        .run(tauri::generate_context!())
        .expect("error while running PatternStudio desktop");
}
