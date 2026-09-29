#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct Diagnostics {
    platform: &'static str,
    capture_state: &'static str,
    microphone_permission: &'static str,
    system_audio_permission: &'static str,
    remote_endpoint_name: Option<String>,
    message: &'static str,
}

// This is the entire IPC surface. Audio frames intentionally cannot be serialized.
#[tauri::command]
fn diagnostics() -> Diagnostics {
    Diagnostics {
        platform: std::env::consts::OS,
        capture_state: "NOT IMPLEMENTED",
        microphone_permission: "Unknown — not queried",
        system_audio_permission: "Unknown — not queried",
        remote_endpoint_name: None,
        message: "Native audio capture is not connected yet. No audio is being recorded.",
    }
}

fn main() {
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![diagnostics])
        .run(tauri::generate_context!())
        .expect("failed to run Unmute");
}
