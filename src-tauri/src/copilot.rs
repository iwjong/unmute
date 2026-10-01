use crate::{AppState, Session};
use serde_json::{json, Value};
use std::{
    io::{BufRead, BufReader, Write},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
};
use tauri::{Emitter, Manager};

#[tauri::command]
pub fn start_copilot(app: tauri::AppHandle, state: tauri::State<AppState>) -> Result<(), String> {
    use std::os::unix::fs::PermissionsExt;
    use unmute::macos::{MacAudioBackend, Options};
    let mut slot = state.session.lock().map_err(|e| e.to_string())?;
    if slot
        .as_ref()
        .is_some_and(|s| s.thread.as_ref().is_some_and(|t| !t.is_finished()))
    {
        return Err("Stop the active capture before starting another session.".into());
    }
    drop(slot.take());
    let cache = app.path().app_cache_dir().map_err(|e| e.to_string())?;
    std::fs::create_dir_all(&cache).map_err(|e| e.to_string())?;
    let executable = cache.join("local-copilot");
    std::fs::write(
        &executable,
        include_bytes!(concat!(env!("OUT_DIR"), "/local-copilot")),
    )
    .map_err(|e| e.to_string())?;
    std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o700))
        .map_err(|e| e.to_string())?;
    let runtime = app
        .path()
        .resource_dir()
        .map_err(|e| e.to_string())?
        .join("local-runtime");
    let models = app
        .path()
        .app_data_dir()
        .map_err(|e| e.to_string())?
        .join("models");
    let listener = std::net::TcpListener::bind("127.0.0.1:0").map_err(|e| e.to_string())?;
    let port = listener.local_addr().map_err(|e| e.to_string())?.port();
    drop(listener);
    let mut child = std::process::Command::new(executable)
        .args([models.as_os_str(), std::ffi::OsStr::new(&port.to_string())])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map_err(|e| e.to_string())?;
    let input = child.stdin.take().unwrap();
    let output = child.stdout.take().unwrap();
    let stop = Arc::new(AtomicBool::new(false));
    let flag = stop.clone();
    let thread = std::thread::spawn(move || {
        let model_ready = Arc::new(AtomicBool::new(false));
        let model_reader = model_ready.clone();
        let mut server: Option<std::process::Child> = None;
        let ready = Arc::new(AtomicBool::new(false));
        let ready_reader = ready.clone();
        let events = app.clone();
        let helper_error = Arc::new(std::sync::Mutex::new(None::<String>));
        let reader_error = helper_error.clone();
        let reader = std::thread::spawn(move || {
            for line in BufReader::new(output).lines() {
                let Ok(line) = line else { break };
                if let Ok(event) = serde_json::from_str::<Value>(&line) {
                    if event["type"] == "model_ready" {
                        model_reader.store(true, Ordering::Relaxed);
                        continue;
                    }
                    if event["type"] == "error" {
                        *reader_error.lock().unwrap() =
                            event["message"].as_str().map(str::to_owned);
                    }
                    if event["status"] == "Listening" {
                        ready_reader.store(true, Ordering::Relaxed);
                        continue;
                    }
                    let _ = events.emit("copilot", event);
                }
            }
        });
        let (sender, receiver) =
            std::sync::mpsc::sync_channel::<(unmute::audio::FrameMetadata, Vec<u8>)>(100);
        let writer = std::thread::spawn(move || -> std::io::Result<()> {
            let mut input = input;
            for (metadata, pcm) in receiver {
                input.write_all(&metadata.timeline_timestamp_ns.to_le_bytes())?;
                input.write_all(&(pcm.len() as u32).to_le_bytes())?;
                input.write_all(&pcm)?;
            }
            Ok(())
        });
        let telemetry = cache.join("local-session.jsonl");
        let result = (|| -> Result<(), String> {
            while !ready.load(Ordering::Relaxed) || server.is_none() {
                if flag.load(Ordering::Relaxed) {
                    return Ok(());
                }
                if model_ready.load(Ordering::Relaxed) && server.is_none() {
                    server = Some(
                        std::process::Command::new(runtime.join("llama-server"))
                            .arg("-m")
                            .arg(models.join("Qwen3-1.7B-Q8_0.gguf"))
                            .args([
                                "--host",
                                "127.0.0.1",
                                "--port",
                                &port.to_string(),
                                "-c",
                                "4096",
                                "-np",
                                "1",
                                "--reasoning",
                                "off",
                                "--no-webui",
                                "--no-warmup",
                                "-ngl",
                                "99",
                            ])
                            .stdin(std::process::Stdio::null())
                            .stdout(std::process::Stdio::null())
                            .stderr(std::process::Stdio::null())
                            .spawn()
                            .map_err(|e| format!("Local response runtime could not start: {e}"))?,
                    );
                }
                if let Some(process) = server.as_mut() {
                    if process.try_wait().map_err(|e| e.to_string())?.is_some() {
                        return Err(
                            "Local response runtime stopped. Check available memory and restart."
                                .into(),
                        );
                    }
                }
                if child.try_wait().map_err(|e| e.to_string())?.is_some() {
                    return Err("Local model could not start. Requires macOS 26+ and the bundled local runtime.".into());
                }
                std::thread::sleep(std::time::Duration::from_millis(50));
            }
            let mut backend = MacAudioBackend::start(Options {
                telemetry: telemetry.clone(),
                recordings: None,
                system_audio_confirmed: true,
            })?;
            backend.set_remote_sink(sender.clone());
            let _ = app.emit("copilot", json!({"type":"status","status":"Listening"}));
            while !flag.load(Ordering::Relaxed) {
                if child.try_wait().map_err(|e| e.to_string())?.is_some() {
                    return Err("Local speech process stopped. Restart listening.".into());
                }
                if server
                    .as_mut()
                    .unwrap()
                    .try_wait()
                    .map_err(|e| e.to_string())?
                    .is_some()
                {
                    return Err("Local response runtime stopped. Restart listening.".into());
                }
                backend.poll(std::time::Duration::from_millis(20))?;
            }
            backend.stop()?;
            Ok(())
        })();
        // Both processes belong to this session; cancellation and startup failure reap both.
        if let Some(mut process) = server {
            let _ = process.kill();
            let _ = process.wait();
        }
        let _ = child.kill();
        let _ = child.wait();
        drop(sender);
        let _ = writer.join();
        let _ = reader.join();
        let _ = std::fs::remove_file(telemetry);
        if let Err(message) = result {
            let message = helper_error.lock().unwrap().clone().unwrap_or(message);
            let _ = app.emit("copilot", json!({"type":"error","message":message}));
        }
        let _ = app.emit("copilot", json!({"type":"status","status":"Stopped"}));
    });
    *slot = Some(Session {
        stop,
        thread: Some(thread),
    });
    Ok(())
}
