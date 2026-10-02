use tauri::{
    menu::{CheckMenuItem, Menu, MenuItem, Submenu},
    tray::TrayIconBuilder,
    Emitter, Manager,
};
use tauri_plugin_autostart::ManagerExt;

pub fn show(app: &tauri::AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.unminimize();
        let _ = window.set_focus();
    }
}

// A small template waveform; macOS supplies the menu bar's light/dark color.
fn icon() -> tauri::image::Image<'static> {
    let mut rgba = vec![0; 32 * 32 * 4];
    for (x, top) in [(4, 12), (9, 7), (14, 3), (19, 7), (24, 12)] {
        for y in top..32 - top {
            for dx in 0..3 {
                rgba[(y * 32 + x + dx) * 4 + 3] = 255;
            }
        }
    }
    tauri::image::Image::new_owned(rgba, 32, 32)
}

pub fn setup(app: &mut tauri::App) -> Result<(), Box<dyn std::error::Error>> {
    let show_item = MenuItem::with_id(app, "show", "Show Window", true, None::<&str>)?;
    let hide_item = MenuItem::with_id(app, "hide", "Hide Window", true, None::<&str>)?;
    let login = CheckMenuItem::with_id(app, "login", "Launch at Login", true, false, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;
    let access = MenuItem::with_id(app, "audio", "Audio Permissions…", true, None::<&str>)?;
    let toggle = MenuItem::with_id(app, "toggle", "Start / Pause Listening", true, None::<&str>)?;
    let suggest = MenuItem::with_id(app, "suggest", "Suggest Now", true, None::<&str>)?;
    let settings = MenuItem::with_id(app, "settings", "Settings…", true, None::<&str>)?;
    let opacity = Submenu::new(app, "Background Transparency", true)?;
    for value in [0, 25, 50, 75, 100] {
        opacity.append(&MenuItem::with_id(
            app,
            format!("transparency-{value}"),
            if value == 100 {
                "100% — Text Only".to_string()
            } else {
                format!("{value}%")
            },
            true,
            None::<&str>,
        )?)?;
    }
    let menu = Menu::with_items(
        app,
        &[
            &show_item, &hide_item, &toggle, &suggest, &access, &opacity, &settings, &login, &quit,
        ],
    )?;

    // Register packaged apps only, never a transient cargo-dev executable.
    let packaged = std::env::current_exe()?
        .components()
        .any(|p| p.as_os_str().to_string_lossy().ends_with(".app"));
    if packaged {
        let marker = app.path().app_data_dir()?.join("login-item-initialized");
        let registration = (|| -> Result<(), Box<dyn std::error::Error>> {
            if !marker.exists() || app.autolaunch().is_enabled()? {
                app.autolaunch().enable()?; // Refresh the path if the app has moved.
            }
            std::fs::create_dir_all(marker.parent().unwrap())?;
            std::fs::write(marker, b"1")?;
            Ok(())
        })();
        if let Err(error) = registration {
            eprintln!("Launch at login: {error}");
            login.set_text("Launch at Login (retry)")?;
        }
    }
    login.set_checked(app.autolaunch().is_enabled().unwrap_or(false))?;
    TrayIconBuilder::with_id("unmute")
        .icon(icon())
        .icon_as_template(true)
        .tooltip("Unmute")
        .menu(&menu)
        .on_menu_event(move |app, event| match event.id.as_ref() {
            "show" => show(app),
            "hide" => {
                if let Some(window) = app.get_webview_window("main") {
                    let _ = window.hide();
                }
            }
            "audio" => {
                let app = app.clone();
                tauri::async_runtime::spawn(async move {
                    let (action, message) = match crate::setup_audio_access().await {
                        Ok(message) => ("notice", message),
                        Err(message) => ("error", message),
                    };
                    show(&app);
                    let _ = app.emit(
                        "meeting-action",
                        serde_json::json!({"action":action,"message":message}),
                    );
                });
            }
            "settings" | "toggle" | "suggest" => {
                if event.id.as_ref() == "settings" {
                    show(app);
                }
                let _ = app.emit(
                    "meeting-action",
                    serde_json::json!({"action":event.id.as_ref()}),
                );
            }
            id if id.starts_with("transparency-") => {
                if let Ok(value) = id["transparency-".len()..].parse::<u8>() {
                    let _ = app.emit(
                        "meeting-action",
                        serde_json::json!({"action":"transparency","value":value}),
                    );
                }
            }
            "login" => {
                let manager = app.autolaunch();
                let result = manager.is_enabled().and_then(|enabled| {
                    if enabled {
                        manager.disable()
                    } else {
                        manager.enable()
                    }
                });
                let _ = login.set_checked(manager.is_enabled().unwrap_or(false));
                let _ = login.set_text(if result.is_ok() {
                    "Launch at Login"
                } else {
                    "Launch at Login (retry)"
                });
                if let Err(error) = result {
                    eprintln!("Launch at login: {error}");
                }
            }
            "quit" => app.exit(0),
            _ => {}
        })
        .build(app)?;
    if !std::env::args().any(|arg| arg == "--autostart") {
        show(app.handle());
    }
    Ok(())
}
