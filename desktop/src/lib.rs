//! PatternStudio desktop (Tauri): calls the tracing engine in-process.
//!
//! The same React frontend used by the web server runs inside the Tauri
//! webview; when `window.__TAURI_INTERNALS__` is present the frontend
//! switches from HTTP calls to the commands in [`commands`].

pub mod commands;

pub fn run() {
    use commands::{
        add_recent_project, batch_trace, cancel_batch_trace, cancel_trace, generate_pattern,
        list_images, list_patterns, open_folder, preprocess_file, read_image, recent_projects,
        redraw_file, remove_recent_project, save_bytes, save_enhanced_image, save_settings,
        save_svg, smart_file, get_settings,
        trace_bytes, trace_file,
    };

    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![
            trace_bytes,
            trace_file,
            cancel_trace,
            smart_file,
            open_folder,
            list_images,
            read_image,
            preprocess_file,
            redraw_file,
            get_settings,
            save_settings,
            save_enhanced_image,
            batch_trace,
            cancel_batch_trace,
            generate_pattern,
            list_patterns,
            save_svg,
            save_bytes,
            recent_projects,
            add_recent_project,
            remove_recent_project
        ])
        .run(tauri::generate_context!())
        .expect("error while running PatternStudio desktop");
}
