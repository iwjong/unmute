#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
#[cfg(target_os = "macos")]
mod copilot;
#[cfg(target_os = "macos")]
use copilot::{control_copilot, start_copilot};
#[cfg(not(target_os = "macos"))]
#[tauri::command]
fn start_copilot() -> Result<(), String> {
    Err("Local copilot requires macOS 26+".into())
}
#[cfg(not(target_os = "macos"))]
#[tauri::command]
fn control_copilot(_action: String) -> Result<(), String> {
    Err("macOS only".into())
}
mod desktop;
use serde_json::{json, Value};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex,
};
use tauri::Manager;
struct Session {
    control: Option<std::sync::mpsc::Sender<String>>,
    stop: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
}
impl Drop for Session {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}
#[derive(Default)]
struct AppState {
    session: Mutex<Option<Session>>,
    status: Arc<Mutex<Value>>,
}

#[tauri::command]
fn diagnostics(state: tauri::State<AppState>) -> Result<Value, String> {
    let session_active = state
        .session
        .lock()
        .map_err(|e| e.to_string())?
        .as_ref()
        .is_some_and(|s| s.thread.as_ref().is_some_and(|t| !t.is_finished()));
    let snapshot = state.status.lock().map_err(|e| e.to_string())?.clone();
    #[cfg(target_os = "macos")]
    return Ok(
        json!({"platform":"macos","preflight":unmute::macos::preflight()?,"snapshot":snapshot,"session_active":session_active}),
    );
    #[cfg(not(target_os = "macos"))]
    let _ = session_active;
    #[cfg(not(target_os = "macos"))]
    Ok(
        json!({"platform":std::env::consts::OS,"error":"Windows backend pending macOS review","snapshot":snapshot}),
    )
}
#[tauri::command]
async fn request_audio_permissions() -> Result<(), String> {
    #[cfg(target_os = "macos")]
    return tauri::async_runtime::spawn_blocking(unmute::macos::request_permissions)
        .await
        .map_err(|e| e.to_string())?;
    #[cfg(not(target_os = "macos"))]
    Err("macOS only".into())
}
#[tauri::command]
async fn setup_audio_access() -> Result<String, String> {
    static REQUESTING: AtomicBool = AtomicBool::new(false);
    if REQUESTING.swap(true, Ordering::SeqCst) {
        return Err("An audio permission request is already open in macOS.".into());
    }
    let result = request_audio_permissions().await;
    REQUESTING.store(false, Ordering::SeqCst);
    match result {
        Ok(()) => Ok("Audio access is ready.".into()),
        Err(message) if message.starts_with("PERMISSION_REQUIRED: verify System Audio Recording") =>
            Ok("Audio access requested. Confirm the macOS prompt or check System Settings → Privacy & Security → Screen & System Audio Recording.".into()),
        Err(message) => Err(message),
    }
}
#[tauri::command]
fn stop_capture(state: tauri::State<AppState>) -> Result<(), String> {
    drop(state.session.lock().map_err(|e| e.to_string())?.take());
    Ok(())
}
#[tauri::command]
fn start_capture(
    app: tauri::AppHandle,
    state: tauri::State<AppState>,
    system_audio_confirmed: bool,
    record: bool,
) -> Result<String, String> {
    #[cfg(target_os = "macos")]
    {
        use unmute::macos::{self, MacAudioBackend, Options};
        macos::gate(&macos::preflight()?, system_audio_confirmed)?;
        let mut slot = state.session.lock().map_err(|e| e.to_string())?;
        if slot
            .as_ref()
            .is_some_and(|s| s.thread.as_ref().is_some_and(|t| !t.is_finished()))
        {
            return Err("Capture already active".into());
        }
        drop(slot.take());
        let id = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(|e| e.to_string())?
            .as_nanos();
        let dir = app
            .path()
            .app_data_dir()
            .map_err(|e| e.to_string())?
            .join(format!("diagnostics/{id}"));
        let options = Options {
            telemetry: dir.join("telemetry.jsonl"),
            recordings: record.then(|| dir.clone()),
            system_audio_confirmed,
        };
        let stop = Arc::new(AtomicBool::new(false));
        let flag = stop.clone();
        let status = state.status.clone();
        *status.lock().map_err(|e| e.to_string())? =
            json!({"capture_state":"STARTING","output_directory":dir});
        let thread = std::thread::spawn(move || {
            let result = (|| -> Result<(), String> {
                let mut backend = MacAudioBackend::start(options)?;
                while !flag.load(Ordering::Relaxed) {
                    backend.poll(std::time::Duration::from_millis(20))?;
                    if let Ok(mut out) = status.lock() {
                        *out = json!(backend.snapshot());
                    }
                }
                let final_state = backend.stop()?;
                if let Ok(mut out) = status.lock() {
                    *out = json!(final_state);
                }
                Ok(())
            })();
            if let Err(e) = result {
                if let Ok(mut out) = status.lock() {
                    *out = json!({"capture_state":if e.contains("PERMISSION_REQUIRED"){"PERMISSION_REQUIRED"}else{"ACTUAL_CAPTURE_FAILURE"},"error":e});
                }
            }
        });
        *slot = Some(Session {
            control: None,
            stop,
            thread: Some(thread),
        });
        Ok(dir.display().to_string())
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (app, state, system_audio_confirmed, record);
        Err("Windows backend pending macOS review".into())
    }
}
// Product and engineering interfaces have independent native windows.
#[tauri::command]
async fn open_diagnostics(app: tauri::AppHandle) -> Result<(), String> {
    if let Some(window) = app.get_webview_window("diagnostics") {
        window.show().map_err(|e| e.to_string())?;
        window.unminimize().map_err(|e| e.to_string())?;
        return window.set_focus().map_err(|e| e.to_string());
    }
    tauri::WebviewWindowBuilder::new(
        &app,
        "diagnostics",
        tauri::WebviewUrl::App("index.html?view=diagnostics".into()),
    )
    .title("Unmute · Audio diagnostics")
    .inner_size(1040.0, 800.0)
    .min_inner_size(420.0, 600.0)
    .build()
    .map_err(|e| e.to_string())?;
    Ok(())
}

fn main() {
    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _, _| {
            desktop::show(app)
        }))
        .plugin(
            tauri_plugin_autostart::Builder::new()
                .arg("--autostart")
                .build(),
        )
        .setup(desktop::setup)
        .manage(AppState::default())
        .on_window_event(|window, event| {
            if window.label() == "main" {
                if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                    if window.hide().is_ok() {
                        api.prevent_close();
                    }
                }
            }
            if window.label() == "main" && matches!(event, tauri::WindowEvent::Destroyed) {
                let state = window.state::<AppState>();
                if let Ok(mut session) = state.session.lock() {
                    drop(session.take());
                };
            }
        })
        .invoke_handler(tauri::generate_handler![
            diagnostics,
            request_audio_permissions,
            setup_audio_access,
            start_capture,
            stop_capture,
            start_copilot,
            control_copilot,
            open_diagnostics
        ])
        .build(tauri::generate_context!())
        .expect("failed to build Unmute")
        .run(|app, event| {
            if matches!(event, tauri::RunEvent::Exit) {
                let state = app.state::<AppState>();
                if let Ok(mut session) = state.session.lock() {
                    drop(session.take());
                };
            }
            #[cfg(target_os = "macos")]
            if matches!(event, tauri::RunEvent::Reopen { .. }) {
                desktop::show(app);
            }
        });
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn ending_session_stops_and_joins_worker() {
        let stop = Arc::new(AtomicBool::new(false));
        let flag = stop.clone();
        let ended = Arc::new(AtomicBool::new(false));
        let done = ended.clone();
        let thread = std::thread::spawn(move || {
            while !flag.load(Ordering::Relaxed) {
                std::thread::sleep(std::time::Duration::from_millis(1));
            }
            done.store(true, Ordering::Relaxed);
        });
        drop(Session {
            control: None,
            stop: stop.clone(),
            thread: Some(thread),
        });
        assert!(stop.load(Ordering::Relaxed));
        assert!(ended.load(Ordering::Relaxed));
    }
}
