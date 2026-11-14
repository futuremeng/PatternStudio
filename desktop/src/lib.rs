//! PatternStudio desktop (Tauri): calls the tracing engine in-process.
//!
//! The same React frontend used by the web server runs inside the Tauri
//! webview; when `window.__TAURI_INTERNALS__` is present the frontend
//! switches from HTTP calls to the commands in [`commands`].

pub mod commands;

/// Persisted main-window geometry (restored on next launch so the window
/// reopens at the size and place the user left it — a core Mac expectation).
#[derive(serde::Deserialize, serde::Serialize)]
struct WindowState {
    width: u32,
    height: u32,
    x: i32,
    y: i32,
    maximized: bool,
}

fn window_state_path<R: tauri::Runtime, M: tauri::Manager<R>>(
    mgr: &M,
) -> Option<std::path::PathBuf> {
    mgr.path()
        .app_data_dir()
        .ok()
        .map(|d| d.join("window-state.json"))
}

fn restore_window_state(win: &tauri::WebviewWindow) {
    let Some(path) = window_state_path(win) else {
        return;
    };
    let Ok(data) = std::fs::read_to_string(path) else {
        return;
    };
    let Ok(state) = serde_json::from_str::<WindowState>(&data) else {
        return;
    };
    if state.width >= 400 && state.height >= 300 {
        let _ = win.set_size(tauri::Size::Logical(tauri::LogicalSize::new(
            state.width as f64,
            state.height as f64,
        )));
    }
    if state.x >= 0 && state.y >= 0 {
        let _ = win.set_position(tauri::Position::Logical(tauri::LogicalPosition::new(
            state.x as f64,
            state.y as f64,
        )));
    }
    if state.maximized {
        let _ = win.maximize();
    }
}

fn save_window_state(win: &tauri::Window) {
    let Some(path) = window_state_path(win) else {
        return;
    };
    let Ok(size) = win.inner_size() else { return };
    let scale = win.scale_factor().unwrap_or(1.0);
    let pos = win.outer_position().ok();
    let state = WindowState {
        width: size.width,
        height: size.height,
        x: pos.map(|p| (p.x as f64 / scale) as i32).unwrap_or(-1),
        y: pos.map(|p| (p.y as f64 / scale) as i32).unwrap_or(-1),
        maximized: win.is_maximized().unwrap_or(false),
    };
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    if let Ok(data) = serde_json::to_string(&state) {
        let _ = std::fs::write(path, data);
    }
}

/// Native menu bar (macOS HIG: the menu bar is the primary command
/// discovery surface). Custom items carry ids that the frontend listens for
/// via the "menu-command" event; predefined items map to native selectors
/// (Quit/Hide/Cut/Copy/Paste/…), which WKWebView routes into the webview.
fn build_menu(app: &tauri::AppHandle) -> tauri::Result<tauri::menu::Menu<tauri::Wry>> {
    use tauri::menu::{MenuBuilder, MenuItem, PredefinedMenuItem, SubmenuBuilder};

    let settings = MenuItem::with_id(app, "settings", "Settings…", true, Some("CmdOrCtrl+,"))?;
    let open_folder_item = MenuItem::with_id(
        app,
        "open-folder",
        "Open Folder…",
        true,
        Some("CmdOrCtrl+O"),
    )?;
    let undo_item = MenuItem::with_id(app, "undo", "Undo", true, Some("CmdOrCtrl+Z"))?;
    let redo_item = MenuItem::with_id(app, "redo", "Redo", true, Some("Shift+CmdOrCtrl+Z"))?;
    let appearance_item = MenuItem::with_id(
        app,
        "toggle-appearance",
        "Toggle Appearance",
        true,
        Some("Alt+CmdOrCtrl+T"),
    )?;
    let help_item = MenuItem::with_id(app, "help", "PatternStudio Help", true, None::<&str>)?;

    let app_submenu = SubmenuBuilder::new(app, "PatternStudio")
        .item(&PredefinedMenuItem::about(app, None, None)?)
        .separator()
        .item(&settings)
        .separator()
        .services()
        .separator()
        .hide()
        .hide_others()
        .show_all()
        .separator()
        .quit()
        .build()?;
    let file_submenu = SubmenuBuilder::new(app, "File")
        .item(&open_folder_item)
        .separator()
        .close_window()
        .build()?;
    let edit_submenu = SubmenuBuilder::new(app, "Edit")
        .item(&undo_item)
        .item(&redo_item)
        .separator()
        .cut()
        .copy()
        .paste()
        .select_all()
        .build()?;
    let view_submenu = SubmenuBuilder::new(app, "View")
        .fullscreen()
        .separator()
        .item(&appearance_item)
        .build()?;
    let window_submenu = SubmenuBuilder::new(app, "Window")
        .minimize()
        .maximize()
        .build()?;
    let help_submenu = SubmenuBuilder::new(app, "Help").item(&help_item).build()?;

    MenuBuilder::new(app)
        .items(&[
            &app_submenu,
            &file_submenu,
            &edit_submenu,
            &view_submenu,
            &window_submenu,
            &help_submenu,
        ])
        .build()
}

pub fn run() {
    use commands::{
        add_recent_project, batch_trace, cancel_batch_trace, cancel_trace, generate_pattern,
        get_settings, layer_file, list_images, list_lineage, list_patterns, open_folder,
        preprocess_file, read_image, recent_projects, redraw_file, remove_recent_project,
        save_bytes, save_project_image, save_settings, save_svg, smart_file, trace_bytes,
        trace_file, vectorize_file,
    };

    use tauri::Emitter;

    tauri::Builder::default()
        .on_menu_event(|window, event| {
            // Forward custom menu commands to the frontend, which owns the
            // actions (the UI lives in the webview). Predefined items are
            // handled natively and never reach this handler.
            let id = event.id().as_ref();
            match id {
                "help" => {
                    #[cfg(target_os = "macos")]
                    let _ = std::process::Command::new("open")
                        .arg("https://github.com/futuremeng/PatternStudio")
                        .spawn();
                }
                _ => {
                    let _ = window.emit("menu-command", serde_json::json!({ "cmd": id }));
                }
            }
        })
        .on_window_event(|window, event| match event {
            tauri::WindowEvent::CloseRequested { .. } | tauri::WindowEvent::Destroyed => {
                save_window_state(window);
            }
            _ => {}
        })
        .setup(|app| {
            use tauri::Manager;
            let menu = build_menu(app.handle())?;
            app.set_menu(menu)?;
            // Config windows exist by the time setup runs — reopen at the
            // saved size/place.
            if let Some(win) = app.get_webview_window("main") {
                restore_window_state(&win);
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            trace_bytes,
            trace_file,
            vectorize_file,
            layer_file,
            cancel_trace,
            smart_file,
            open_folder,
            list_images,
            list_lineage,
            read_image,
            preprocess_file,
            redraw_file,
            get_settings,
            save_settings,
            save_project_image,
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
