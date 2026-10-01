#[cfg(any(target_os = "macos", target_os = "windows"))]
fn run() -> Result<(), String> {
    use std::{
        io::Write,
        path::PathBuf,
        sync::{
            atomic::{AtomicBool, Ordering},
            Arc,
        },
        time::{Duration, Instant},
    };
    #[cfg(target_os = "macos")]
    use unmute::macos::{self, MacAudioBackend as Backend, Options};
    #[cfg(target_os = "windows")]
    use unmute::windows::{self, Options, WindowsAudioBackend as Backend};
    let mut args = std::env::args().skip(1);
    let command = args.next().unwrap_or_else(|| "help".into());
    let mut duration = None;
    let mut telemetry = None;
    let mut recordings = None;
    let mut confirmed = false;
    #[cfg(target_os = "windows")]
    let (mut output_endpoint, mut input_endpoint) = (None, None);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--duration" => {
                let n = args
                    .next()
                    .ok_or("missing duration")?
                    .parse::<u64>()
                    .map_err(|_| "duration must be positive integer seconds")?;
                if n == 0 {
                    return Err("duration must be positive".into());
                }
                duration = Some(Duration::from_secs(n));
            }
            "--telemetry" => {
                telemetry = Some(PathBuf::from(args.next().ok_or("missing telemetry path")?))
            }
            "--record-dir" => {
                recordings = Some(PathBuf::from(
                    args.next().ok_or("missing recording directory")?,
                ))
            }
            "--confirm-system-audio-permission" => confirmed = true,
            #[cfg(target_os = "windows")]
            "--output-endpoint" => {
                output_endpoint = Some(args.next().ok_or("missing output endpoint ID or role")?)
            }
            #[cfg(target_os = "windows")]
            "--input-endpoint" => {
                input_endpoint = Some(args.next().ok_or("missing input endpoint ID")?)
            }
            _ => return Err(format!("Unknown argument {arg}")),
        }
    }
    if command == "help" || command == "--help" {
        println!("audio-diag preflight\naudio-diag start --duration SECONDS --telemetry PATH [--record-dir DIR]\nStop: Ctrl-C. Omit --duration to run until stopped. Speakers are supported; bleed is expected.");
        #[cfg(target_os = "macos")]
        println!("macOS: audio-diag request-permissions; start requires --confirm-system-audio-permission for this exact binary.");
        #[cfg(target_os = "windows")]
        println!("Windows: audio-diag endpoints; preflight/start accept --output-endpoint ID|multimedia|communications and --input-endpoint ID. Default output: sole active output, otherwise multimedia default. Microphone: default communications input.");
        return Ok(());
    }
    #[cfg(target_os = "windows")]
    if command == "endpoints" {
        println!(
            "{}",
            serde_json::to_string_pretty(&windows::list_endpoints()?).unwrap()
        );
        return Ok(());
    }
    #[cfg(target_os = "macos")]
    if command == "request-permissions" {
        return macos::request_permissions();
    }
    if command != "start" && command != "preflight" {
        return Err(format!("Unknown command {command}"));
    }
    #[cfg(target_os = "macos")]
    let p = macos::preflight()?;
    #[cfg(target_os = "windows")]
    let p = windows::preflight(output_endpoint.as_deref(), input_endpoint.as_deref())?;
    if command == "preflight" {
        println!("{}", serde_json::to_string_pretty(&p).unwrap());
        return Ok(());
    }
    let telemetry = telemetry.ok_or("--telemetry PATH is required")?;
    let summary_path = telemetry.with_extension("summary.json");
    println!("OUTPUT ROUTE: {} [{}]", p.route.name, p.route.uid);
    #[cfg(target_os = "macos")]
    macos::gate(&p, confirmed)?;
    #[cfg(target_os = "windows")]
    let _ = confirmed;
    // Reserve the summary first: never overwrite an earlier run's evidence.
    let parent = summary_path.parent().filter(|p| !p.as_os_str().is_empty());
    if let Some(parent) = parent {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let mut summary = std::fs::File::options()
        .write(true)
        .create_new(true)
        .open(&summary_path)
        .map_err(|e| e.to_string())?;
    let stop = Arc::new(AtomicBool::new(false));
    let signal = stop.clone();
    ctrlc::set_handler(move || signal.store(true, Ordering::Relaxed)).map_err(|e| e.to_string())?;
    let mut backend = match Backend::start(Options {
        telemetry,
        recordings,
        #[cfg(target_os = "macos")]
        system_audio_confirmed: confirmed,
        #[cfg(target_os = "windows")]
        output_endpoint,
        #[cfg(target_os = "windows")]
        input_endpoint,
    }) {
        Ok(b) => b,
        Err(e) => {
            serde_json::to_writer_pretty(
                &mut summary,
                &serde_json::json!({"status":"START_FAILED","error":e}),
            )
            .map_err(|e| e.to_string())?;
            return Err(e);
        }
    };
    let started = Instant::now();
    let mut next = Instant::now();
    let mut failure = None;
    while !stop.load(Ordering::Relaxed) && duration.is_none_or(|d| started.elapsed() < d) {
        if let Err(e) = backend.poll(Duration::from_millis(20)) {
            failure = Some(e);
            break;
        }
        if Instant::now() >= next {
            next = Instant::now() + Duration::from_secs(1);
            let s = backend.snapshot();
            println!("{:7.1}s {:?} USER rms={:?} peak={:?} REMOTE rms={:?} peak={:?} drops={}/{} discontinuities={}/{} relative_drift_ns={:?}",
                s.elapsed_seconds,s.capture_state,s.user.rms,s.user.peak,s.remote.rms,s.remote.peak,
                s.user.dropped_frames,s.remote.dropped_frames,s.user.discontinuities,s.remote.discontinuities,s.relative_drift_ns);
        }
    }
    let final_state = backend.stop();
    let report = serde_json::json!({"status":if failure.is_some()||final_state.is_err(){"FAILED"}else{"CAPTURE_COMPLETE_NOT_ACCEPTANCE"},
        "requested_duration_seconds":duration.map(|v|v.as_secs()),"stopped_by_signal":stop.load(Ordering::Relaxed),
        "error":failure,"finalization_error":final_state.as_ref().err(),"snapshot":final_state.as_ref().ok(),
        "speaker_bleed":"EXPECTED_NOT_AN_M0_FAILURE",
        "microphone_quality":"PENDING_HUMAN_VALIDATION","real_meeting":"PENDING_HUMAN_VALIDATION",
        "physical_reconnect":"PENDING_HUMAN_VALIDATION","signed_app_permissions":"PENDING_HUMAN_VALIDATION"});
    serde_json::to_writer_pretty(&mut summary, &report).map_err(|e| e.to_string())?;
    summary.write_all(b"\n").map_err(|e| e.to_string())?;
    summary.sync_all().map_err(|e| e.to_string())?;
    println!("Summary: {}", summary_path.display());
    if let Some(e) = failure {
        return Err(e);
    }
    final_state.map(|_| ())
}
#[cfg(not(any(target_os = "macos", target_os = "windows")))]
fn run() -> Result<(), String> {
    Err("audio-diag supports macOS and Windows".into())
}
fn main() {
    if let Err(e) = run() {
        eprintln!("{e}");
        std::process::exit(1);
    }
}
