//! The one macOS backend. Both desktop and audio-diag own this same type.
use crate::{
    audio::{CaptureClock, FrameMetadata, NormalizedFrame, Source, SourceFormat},
    diagnostics::*,
};
use rubato::{
    Resampler, SincFixedIn, SincInterpolationParameters, SincInterpolationType, WindowFunction,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    ffi::{c_char, c_void, CStr},
    fs::{self, File},
    io::{BufWriter, Write},
    path::PathBuf,
    sync::{
        atomic::{AtomicU64, Ordering},
        mpsc::{self, Receiver, SyncSender},
    },
    time::{Duration, Instant},
};

unsafe extern "C" {
    fn um_preflight() -> *mut c_char;
    fn um_free(p: *mut c_char);
    fn um_timebase(numer: *mut u32, denom: *mut u32, origin: *mut u64);
    fn um_request_microphone() -> i32;
    fn um_start(
        remote: i32,
        callback: extern "C" fn(*mut c_void, *const f32, u32, u64, f64, u32),
        context: *mut c_void,
        error: *mut i32,
    ) -> *mut c_void;
    fn um_stop(handle: *mut c_void) -> i32;
    fn um_invalid(handle: *mut c_void) -> u64;
    fn um_alive(handle: *mut c_void) -> i32;
    fn um_resources(memory: *mut u64, cpu_seconds: *mut f64) -> i32;
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Preflight {
    pub microphone: String,
    pub system_audio: String,
    pub system_audio_evidence: String,
    pub route: Route,
    pub input_name: String,
    pub input_id: u32,
    pub input_sample_rate: f64,
    pub input_channels: u32,
}
pub fn preflight() -> Result<Preflight, String> {
    unsafe {
        let p = um_preflight();
        if p.is_null() {
            return Err("preflight allocation failed".into());
        }
        let result =
            serde_json::from_slice(CStr::from_ptr(p).to_bytes()).map_err(|e| e.to_string());
        um_free(p);
        result
    }
}
pub fn gate(p: &Preflight, system_audio_confirmed: bool) -> Result<(), String> {
    let mut needed = vec![];
    if p.microphone != "GRANTED" {
        needed.push(format!("Microphone ({})", p.microphone));
    }
    if matches!(
        p.system_audio.as_str(),
        "DENIED" | "NOT_DETERMINED" | "RESTRICTED"
    ) || !system_audio_confirmed
    {
        needed.push(format!(
            "System Audio Recording ({}; confirm in macOS settings for this exact binary)",
            p.system_audio
        ));
    }
    if !needed.is_empty() {
        return Err(format!("PERMISSION_REQUIRED: {}", needed.join("; ")));
    }
    if p.route.id == 0 {
        return Err("OUTPUT_ROUTE_UNAVAILABLE".into());
    }
    Ok(())
}
extern "C" fn discard(_: *mut c_void, _: *const f32, _: u32, _: u64, _: f64, _: u32) {}
/// Permission setup only, not a validation run. No samples are retained or analyzed.
pub fn request_permissions() -> Result<(), String> {
    let status = unsafe { um_request_microphone() };
    if status != 1 {
        return Err("PERMISSION_REQUIRED: Microphone; grant access then rerun".into());
    }
    let mut error = 0;
    let handle = unsafe { um_start(1, discard, std::ptr::null_mut(), &mut error) };
    if handle.is_null() {
        return Err(format!("PERMISSION_REQUIRED: System Audio Recording; permission setup OSStatus={error}; verify grant before investigating capture"));
    }
    std::thread::sleep(Duration::from_secs(2));
    let status = unsafe { um_stop(handle) };
    if status != 0 {
        return Err(format!(
            "Permission setup teardown failed: OSStatus={status}; permission remains unverified"
        ));
    }
    Err("PERMISSION_REQUIRED: verify System Audio Recording in settings, then explicitly confirm for the next run".into())
}

struct Raw {
    source: usize,
    generation: u64,
    sequence: u64,
    ticks: u64,
    rate: f64,
    channels: u32,
    samples: Vec<f32>,
}
struct CallbackContext {
    sender: SyncSender<Raw>,
    source: usize,
    generation: u64,
    sequence: AtomicU64,
    dropped: AtomicU64,
}
extern "C" fn captured(
    context: *mut c_void,
    samples: *const f32,
    count: u32,
    ticks: u64,
    rate: f64,
    channels: u32,
) {
    // Native validates format/count; this final bound protects allocation at the FFI boundary.
    if context.is_null() || samples.is_null() || count == 0 || count > 32768 {
        return;
    }
    let c = unsafe { &*(context as *const CallbackContext) };
    let sequence = c.sequence.fetch_add(1, Ordering::Relaxed);
    let raw = Raw {
        source: c.source,
        generation: c.generation,
        sequence,
        ticks,
        rate,
        channels,
        samples: unsafe { std::slice::from_raw_parts(samples, count as usize) }.to_vec(),
    };
    if c.sender.try_send(raw).is_err() {
        c.dropped.fetch_add(1, Ordering::Relaxed);
    }
}
struct NativeStream {
    handle: *mut c_void,
    context: Option<Box<CallbackContext>>,
}
static TEARDOWN_FAILURES: AtomicU64 = AtomicU64::new(0);
impl NativeStream {
    fn start(source: usize, generation: u64, sender: SyncSender<Raw>) -> Result<Self, String> {
        let mut context = Box::new(CallbackContext {
            sender,
            source,
            generation,
            sequence: AtomicU64::new(0),
            dropped: AtomicU64::new(0),
        });
        let mut error = 0;
        let handle = unsafe {
            um_start(
                source as i32,
                captured,
                (&mut *context as *mut CallbackContext).cast(),
                &mut error,
            )
        };
        if handle.is_null() {
            return Err(format!("Core Audio start failed, source={source}, OSStatus={error}. Verify TCC grant if permission has changed."));
        }
        Ok(Self {
            handle,
            context: Some(context),
        })
    }
    fn drops(&self) -> u64 {
        self.context
            .as_ref()
            .unwrap()
            .dropped
            .load(Ordering::Relaxed)
            + unsafe { um_invalid(self.handle) }
    }
    fn alive(&self) -> bool {
        unsafe { um_alive(self.handle) != 0 }
    }
}
impl Drop for NativeStream {
    fn drop(&mut self) {
        let status = unsafe { um_stop(self.handle) };
        if status != 0 {
            TEARDOWN_FAILURES.fetch_add(1, Ordering::Relaxed);
            eprintln!("Native teardown failed: OSStatus={status}; callback context retained for memory safety");
            if let Some(context) = self.context.take() {
                Box::leak(context);
            }
        }
    }
}

struct Normalizer {
    resampler: SincFixedIn<f32>,
    pending: Vec<f32>,
    pending_ns: Option<f64>,
    rate: f64,
    consumed: u64,
    produced: u64,
    delay: usize,
    trim: usize,
}
impl Normalizer {
    fn new(rate: f64) -> Result<Self, String> {
        let resampler = SincFixedIn::new(
            16000.0 / rate,
            1.0,
            SincInterpolationParameters {
                sinc_len: 128,
                f_cutoff: 0.95,
                interpolation: SincInterpolationType::Cubic,
                oversampling_factor: 128,
                window: WindowFunction::BlackmanHarris2,
            },
            1024,
            1,
        )
        .map_err(|e| e.to_string())?;
        let delay = resampler.output_delay();
        Ok(Self {
            resampler,
            pending: vec![],
            pending_ns: None,
            rate,
            consumed: 0,
            produced: 0,
            delay,
            trim: delay,
        })
    }
    fn push(&mut self, samples: &[f32], time_ns: i64) -> Result<Vec<(i64, Vec<f32>)>, String> {
        // Anchor each new callback to its native capture time, retaining only the prior partial block.
        if self.pending.is_empty() {
            self.pending_ns = Some(time_ns as f64);
        }
        self.pending.extend_from_slice(samples);
        let mut result = vec![];
        while self.pending.len() >= 1024 {
            let input: Vec<f32> = self.pending.drain(..1024).collect();
            let output = self
                .resampler
                .process(&[input], None)
                .map_err(|e| e.to_string())?
                .remove(0);
            let skip = self.trim.min(output.len());
            self.trim -= skip;
            let offset = (self.produced as f64 + skip as f64 - self.delay as f64) * 62500.0
                - self.consumed as f64 * 1e9 / self.rate;
            let timestamp = (self.pending_ns.unwrap() + offset).round() as i64;
            self.produced += output.len() as u64;
            self.consumed += 1024;
            self.pending_ns = self.pending_ns.map(|v| v + 1024.0 * 1e9 / self.rate);
            if skip < output.len() {
                result.push((timestamp, output[skip..].to_vec()));
            }
        }
        // Reanchor remaining input to the exact native callback it came from, not arrival time.
        self.pending_ns = Some(
            time_ns as f64 + (samples.len() as f64 - self.pending.len() as f64) * 1e9 / self.rate,
        );
        Ok(result)
    }
}

#[derive(Clone, Debug, Serialize, Default)]
pub struct SourceStats {
    pub sequence: u64,
    pub native_sequence: Option<u64>,
    pub generation: u64,
    pub rms: Option<f64>,
    pub peak: Option<f64>,
    pub samples: u64,
    pub dropped_frames: u64,
    pub discontinuities: u64,
    pub drift_ns: Option<f64>,
    pub last_timeline_ns: Option<i64>,
    pub first_timeline_ns: Option<i64>,
    pub original_sample_rate: Option<f64>,
    pub original_channels: Option<u32>,
    pub last_packet_elapsed_seconds: Option<f64>,
    pub discarded_tail_native_samples: u64,
    pub discarded_filter_tail_output_samples: u64,
}
#[derive(Clone, Debug, Serialize)]
pub struct Snapshot {
    pub capture_state: CaptureState,
    pub elapsed_seconds: f64,
    pub route: Route,
    pub microphone_permission: String,
    pub system_audio_permission: String,
    pub user: SourceStats,
    pub remote: SourceStats,
    pub correlation: Option<Correlation>,
    pub relative_drift_ns: Option<f64>,
    pub initial_source_offset_ns: Option<i128>,
    pub restarts: u64,
    pub route_changes: u64,
    pub warning: Option<String>,
    pub memory_bytes: Option<u64>,
    pub cpu_percent: Option<f64>,
    pub teardown_failures: u64,
}
#[derive(Clone)]
pub struct Options {
    pub telemetry: PathBuf,
    pub recordings: Option<PathBuf>,
    pub system_audio_confirmed: bool,
}

pub struct MacAudioBackend {
    user: Option<NativeStream>,
    remote: Option<NativeStream>,
    receiver: Receiver<Raw>,
    sender: SyncSender<Raw>,
    timeline: HostTimeline,
    start: Instant,
    next_route: Instant,
    next_sample: Instant,
    next_resources: Instant,
    route: Route,
    input_id: u32,
    input_format: (f64, u32),
    route_changes: u64,
    user_restarts: u64,
    recovery: Recovery,
    normalizers: [Option<Normalizer>; 2],
    pending_discontinuity: [bool; 2],
    drift: [Drift; 2],
    rolling: [RollingAudio; 2],
    stats: [SourceStats; 2],
    snapshot: Snapshot,
    telemetry: BufWriter<File>,
    recordings: [Option<WavWriter>; 2],
    cumulative_drops: [u64; 2],
    last_cpu: Option<(Instant, f64)>,
    stopped: bool,
    remote_sink: Option<SyncSender<(FrameMetadata, Vec<u8>)>>,
    teardown_baseline: u64,
}
impl MacAudioBackend {
    /// Optional product consumer; capture, normalization and diagnostics remain unchanged.
    pub fn set_remote_sink(&mut self, sink: SyncSender<(FrameMetadata, Vec<u8>)>) {
        self.remote_sink = Some(sink);
    }

    pub fn start(options: Options) -> Result<Self, String> {
        let p = preflight()?;
        gate(&p, options.system_audio_confirmed)?;
        if let Some(parent) = options
            .telemetry
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
        {
            fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        let telemetry = BufWriter::new(
            File::options()
                .write(true)
                .create_new(true)
                .open(&options.telemetry)
                .map_err(|e| e.to_string())?,
        );
        let mut recordings = [None, None];
        if let Some(dir) = options.recordings {
            fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
            recordings = [
                Some(WavWriter::create(&dir.join("USER.wav")).map_err(|e| e.to_string())?),
                Some(WavWriter::create(&dir.join("REMOTE.wav")).map_err(|e| e.to_string())?),
            ];
        }
        let (mut numer, mut denom, mut origin) = (0, 0, 0);
        unsafe { um_timebase(&mut numer, &mut denom, &mut origin) };
        let timeline = HostTimeline::new(origin, numer, denom)?;
        let start = Instant::now();
        let (sender, receiver) = mpsc::sync_channel(256);
        let warning = p.route.kind != "HEADPHONES";
        let state = if warning {
            CaptureState::Warning
        } else {
            CaptureState::Starting
        };
        let snapshot = Snapshot { capture_state:state,elapsed_seconds:0.0,route:p.route.clone(),
            microphone_permission:p.microphone.clone(),system_audio_permission:"HUMAN_CONFIRMED — not OS-queried".into(),
            user:SourceStats::default(),remote:SourceStats::default(),correlation:None,relative_drift_ns:None,
            initial_source_offset_ns:None,restarts:0,route_changes:0,
            warning:warning.then(||"Speaker/unknown route: USER may contain remote speech through acoustic bleed. Capture is permitted; source isolation is unvalidated.".into()),
            memory_bytes:None,cpu_percent:None,teardown_failures:0 };
        let mut backend = Self {
            user: None,
            remote: None,
            receiver,
            sender,
            timeline,
            start,
            next_route: start,
            next_sample: start,
            next_resources: start,
            route: p.route,
            input_id: p.input_id,
            input_format: (p.input_sample_rate, p.input_channels),
            route_changes: 0,
            user_restarts: 0,
            recovery: Recovery {
                state,
                warning,
                ..Recovery::default()
            },
            normalizers: [None, None],
            pending_discontinuity: [false, false],
            drift: [Drift::default(), Drift::default()],
            rolling: [RollingAudio::default(), RollingAudio::default()],
            stats: [SourceStats::default(), SourceStats::default()],
            snapshot,
            telemetry,
            recordings,
            cumulative_drops: [0, 0],
            last_cpu: None,
            stopped: false,
            remote_sink: None,
            teardown_baseline: TEARDOWN_FAILURES.load(Ordering::Relaxed),
        };
        backend.event(json!({"type":"session_start","clock":timeline,"preflight":preflight()?,"system_audio_permission_evidence":"human confirmation",
            "format":{"rate":16000,"channels":1,"encoding":"PCM_S16LE"},"speaker_mode_allowed":true}))?;
        backend.user = Some(NativeStream::start(0, 0, backend.sender.clone())?);
        backend.remote = Some(NativeStream::start(1, 0, backend.sender.clone())?);
        backend.event(json!({"type":"capture_started","route":backend.route}))?;
        Ok(backend)
    }
    fn event(&mut self, mut value: Value) -> Result<(), String> {
        value["monotonic_ns"] = json!(self.start.elapsed().as_nanos());
        serde_json::to_writer(&mut self.telemetry, &value).map_err(|e| e.to_string())?;
        self.telemetry.write_all(b"\n").map_err(|e| e.to_string())
    }
    pub fn snapshot(&self) -> Snapshot {
        self.snapshot.clone()
    }
    fn reset_normalizer(&mut self, source: usize) {
        if let Some(n) = self.normalizers[source].take() {
            self.stats[source].discarded_tail_native_samples += n.pending.len() as u64;
            self.stats[source].discarded_filter_tail_output_samples +=
                ((n.consumed as f64 * 16000.0 / n.rate).floor() as u64)
                    .saturating_sub(n.produced.saturating_sub(n.delay as u64));
        }
        self.pending_discontinuity[source] = true;
        self.stats[source].rms = None;
        self.stats[source].peak = None;
        self.stats[source].last_packet_elapsed_seconds = None;
        self.rolling[source].clear();
        self.snapshot.correlation = None;
    }
    fn process(&mut self, raw: Raw) -> Result<(), String> {
        let s = raw.source;
        if raw.generation != self.stats[s].generation {
            return Ok(());
        } // Old route queue cannot enter rebuilt stream.
        let native_ns = self.timeline.native_ns(raw.ticks);
        let time = self.timeline.timeline_ns(raw.ticks)?;
        let d = self.drift[s].observe(native_ns, raw.samples.len(), raw.rate);
        let gap = self.stats[s]
            .native_sequence
            .is_some_and(|seq| raw.sequence != seq + 1)
            || d.backwards
            || d.step_ns.abs() > 2_000_000.0;
        let format_changed = self.stats[s]
            .original_sample_rate
            .is_some_and(|r| r != raw.rate)
            || self.stats[s]
                .original_channels
                .is_some_and(|c| c != raw.channels);
        let discontinuity = gap
            || format_changed
            || (raw.generation > 0 && self.stats[s].native_sequence.is_none());
        if discontinuity {
            self.stats[s].discontinuities += 1;
            self.reset_normalizer(s);
        }
        if self.normalizers[s].is_none() {
            self.normalizers[s] = Some(Normalizer::new(raw.rate)?);
        }
        self.stats[s].native_sequence = Some(raw.sequence);
        self.stats[s].original_sample_rate = Some(raw.rate);
        self.stats[s].original_channels = Some(raw.channels);
        self.stats[s].drift_ns = Some(d.drift_ns);
        self.stats[s].last_packet_elapsed_seconds = Some(self.start.elapsed().as_secs_f64());
        self.event(json!({"type":"native_frame","source":source_name(s),"native_host_ticks":raw.ticks,
            "native_capture_ns":native_ns,"timeline_ns":time,"native_sequence":raw.sequence,"generation":raw.generation,
            "native_samples":raw.samples.len(),"native_rate":raw.rate,"native_channels":raw.channels,
            "drift":d,"discontinuity":discontinuity}))?;
        let frames = self.normalizers[s]
            .as_mut()
            .unwrap()
            .push(&raw.samples, time)?;
        for (timestamp, samples) in frames {
            let pcm: Vec<u8> = samples
                .iter()
                .flat_map(|v| {
                    ((v.clamp(-1.0, 1.0) * 32768.0)
                        .round()
                        .clamp(-32768.0, 32767.0) as i16)
                        .to_le_bytes()
                })
                .collect();
            let metadata = FrameMetadata {
                source: if s == 0 { Source::User } else { Source::Remote },
                original_format: SourceFormat {
                    sample_rate: raw.rate.round() as u32,
                    channels: raw.channels as u16,
                },
                capture_timestamp_ns: self.timeline.native_from_timeline_ns(timestamp)?,
                capture_clock: CaptureClock::MacHostTime,
                timeline_timestamp_ns: timestamp,
                sequence: self.stats[s].sequence,
                discontinuity: self.pending_discontinuity[s],
            };
            let frame = NormalizedFrame::new(metadata, pcm)
                .map_err(|e| format!("invalid normalized frame: {e:?}"))?;
            // Consumers see only normalized samples; the CLI and UI never implement capture.
            let normalized: Vec<f32> = frame
                .pcm_le()
                .chunks_exact(2)
                .map(|b| i16::from_le_bytes([b[0], b[1]]) as f32 / 32768.0)
                .collect();
            let (rms, peak) = levels(&normalized);
            self.rolling[s].push(timestamp, normalized);
            let recording_offset = self.recordings[s]
                .as_mut()
                .map(|w| w.write(frame.pcm_le()))
                .transpose()
                .map_err(|e| e.to_string())?;
            self.stats[s].rms = Some(rms);
            self.stats[s].peak = Some(peak);
            self.stats[s].first_timeline_ns.get_or_insert(timestamp);
            self.stats[s].last_timeline_ns = Some(timestamp);
            self.stats[s].samples += frame.sample_count() as u64;
            self.event(json!({"type":"normalized_frame","source":source_name(s),"sequence":self.stats[s].sequence,
                "native_sequence":raw.sequence,"generation":raw.generation,"timeline_ns":timestamp,
                "capture_ns":metadata.capture_timestamp_ns,"timestamp_basis":"AudioTimeStamp.mHostTime + resampler sample offset",
                "sample_count":frame.sample_count(),"rms":rms,"peak":peak,"drift_ns":d.drift_ns,
                "discontinuity":metadata.discontinuity,"recording_sample_offset":recording_offset}))?;
            if s == 1 {
                if let Some(sink) = &self.remote_sink {
                    sink.try_send((metadata, frame.pcm_le().to_vec()))
                        .map_err(|_| {
                            "Local transcription fell behind or disconnected".to_string()
                        })?;
                }
            }
            self.stats[s].sequence += 1;
            self.pending_discontinuity[s] = false;
        }
        Ok(())
    }
    pub fn poll(&mut self, timeout: Duration) -> Result<(), String> {
        if self.stopped {
            return Err("capture already stopped".into());
        }
        if TEARDOWN_FAILURES.load(Ordering::Relaxed) > self.teardown_baseline {
            self.event(json!({"type":"native_teardown_failure"}))?;
            return Err("Native teardown failed; restart the diagnostic application".into());
        }
        if let Ok(raw) = self.receiver.recv_timeout(timeout) {
            self.process(raw)?;
        }
        // Bound processing per poll so route monitoring and stop remain responsive.
        for _ in 0..32 {
            match self.receiver.try_recv() {
                Ok(raw) => self.process(raw)?,
                Err(_) => break,
            }
        }
        let now = Instant::now();
        if now >= self.next_route {
            self.next_route = now + Duration::from_millis(500);
            let p = preflight()?;
            if p.microphone != "GRANTED" {
                self.recovery.state = CaptureState::PermissionRequired;
                return Err(format!(
                    "PERMISSION_REQUIRED: Microphone ({})",
                    p.microphone
                ));
            }
            if p.input_id != self.input_id
                || (p.input_sample_rate, p.input_channels) != self.input_format
            {
                self.event(
                    json!({"type":"input_route_changed","old":self.input_id,"new":p.input_id}),
                )?;
                self.cumulative_drops[0] += self.user.as_ref().map_or(0, NativeStream::drops);
                drop(self.user.take());
                self.stats[0].generation += 1;
                self.stats[0].native_sequence = None;
                self.reset_normalizer(0);
                self.user = Some(NativeStream::start(
                    0,
                    self.stats[0].generation,
                    self.sender.clone(),
                )?);
                self.input_id = p.input_id;
                self.input_format = (p.input_sample_rate, p.input_channels);
                self.user_restarts += 1;
            }
            if p.route != self.route || self.remote.is_none() {
                if p.route != self.route {
                    self.route_changes += 1;
                }
                self.cumulative_drops[1] += self.remote.as_ref().map_or(0, NativeStream::drops);
                self.reset_normalizer(1);
                let generation = self.recovery.generation + 1;
                let sender = self.sender.clone();
                let events = self.recovery.rebuild(&mut self.remote, &p.route, || {
                    if p.route.id == 0 {
                        Err("output route unavailable".into())
                    } else {
                        NativeStream::start(1, generation, sender)
                    }
                });
                self.stats[1].generation = generation;
                self.stats[1].native_sequence = None;
                self.route = p.route;
                for event in events {
                    self.event(json!({"type":"route_recovery","event":event,"route":self.route,"generation":generation}))?;
                }
                // Failed rebuilds retry on the next 500 ms route probe, with no clock reset.
            }
            if self.user.as_ref().is_some_and(|s| !s.alive()) {
                return Err("USER device is no longer alive".into());
            }
            if self.remote.as_ref().is_some_and(|s| !s.alive()) {
                self.cumulative_drops[1] += self.remote.as_ref().map_or(0, NativeStream::drops);
                drop(self.remote.take());
                self.event(json!({"type":"remote_device_not_alive"}))?;
            }
        }
        if now >= self.next_resources {
            self.next_resources = now + Duration::from_secs(10);
            let (mut memory, mut cpu) = (0, 0.0);
            if unsafe { um_resources(&mut memory, &mut cpu) } == 0 {
                self.snapshot.memory_bytes = Some(memory);
                self.snapshot.cpu_percent = self
                    .last_cpu
                    .map(|(t, c)| 100.0 * (cpu - c) / now.duration_since(t).as_secs_f64());
                self.last_cpu = Some((now, cpu));
                self.event(json!({"type":"resources","memory_bytes":memory,"cpu_percent":self.snapshot.cpu_percent}))?;
            }
        }
        if now >= self.next_sample {
            self.next_sample = now + Duration::from_secs(1);
            self.snapshot.correlation = None;
            if let (Some(u), Some(r)) = (self.rolling[0].end(), self.rolling[1].end()) {
                if let Some(start) = u.min(r + 300_000_000).checked_sub(800_000_000) {
                    self.snapshot.correlation = match (
                        self.rolling[1].window(start, 8000),
                        self.rolling[0].window(start, 12800),
                    ) {
                        (Some(remote), Some(user)) => correlate(&remote, &user, start),
                        _ => None,
                    };
                }
            }
            self.update_snapshot();
            self.event(json!({"type":"telemetry","snapshot":self.snapshot}))?;
            self.telemetry.flush().map_err(|e| e.to_string())?;
        }
        Ok(())
    }
    fn update_snapshot(&mut self) {
        for (s, stream) in [&self.user, &self.remote].iter().enumerate() {
            self.stats[s].dropped_frames =
                self.cumulative_drops[s] + stream.as_ref().map_or(0, NativeStream::drops);
        }
        let elapsed = self.start.elapsed().as_secs_f64();
        let user_stalled = elapsed > 5.0
            && self.stats[0]
                .last_packet_elapsed_seconds
                .is_none_or(|t| elapsed - t > 5.0);
        let state = if self.remote.is_none() || user_stalled {
            CaptureState::ActualCaptureFailure
        } else if self.recovery.warning {
            CaptureState::Warning
        } else if self.stats[1]
            .last_packet_elapsed_seconds
            .is_none_or(|t| elapsed - t > 2.0)
        {
            CaptureState::IdleNoRenderAudio
        } else {
            CaptureState::ActiveAudio
        };
        self.snapshot.capture_state = state;
        self.snapshot.elapsed_seconds = elapsed;
        self.snapshot.route = self.route.clone();
        self.snapshot.user = self.stats[0].clone();
        self.snapshot.remote = self.stats[1].clone();
        for source in [&mut self.snapshot.user, &mut self.snapshot.remote] {
            if source
                .last_packet_elapsed_seconds
                .is_none_or(|t| elapsed - t > 2.0)
            {
                source.rms = None;
                source.peak = None;
                self.snapshot.correlation = None;
            }
        }
        self.snapshot.restarts = self.recovery.restarts + self.user_restarts;
        self.snapshot.teardown_failures =
            TEARDOWN_FAILURES.load(Ordering::Relaxed) - self.teardown_baseline;
        self.snapshot.route_changes = self.route_changes;
        self.snapshot.relative_drift_ns = self.stats[0]
            .drift_ns
            .zip(self.stats[1].drift_ns)
            .map(|(u, r)| u - r);
        self.snapshot.initial_source_offset_ns = self.stats[0]
            .first_timeline_ns
            .zip(self.stats[1].first_timeline_ns)
            .map(|(u, r)| u as i128 - r as i128);
        self.snapshot.warning=self.recovery.warning.then(||"Potential USER/REMOTE acoustic contamination on speaker or unknown output route. Observational metrics are not an isolation PASS/FAIL gate.".into());
    }
    pub fn stop(&mut self) -> Result<Snapshot, String> {
        if self.stopped {
            return Ok(self.snapshot());
        }
        self.update_snapshot();
        drop(self.user.take());
        drop(self.remote.take());
        self.snapshot.teardown_failures =
            TEARDOWN_FAILURES.load(Ordering::Relaxed) - self.teardown_baseline;
        while let Ok(raw) = self.receiver.try_recv() {
            self.process(raw)?;
        }
        // ponytail: retain <1024 native samples as a reported tail, rather than zero-pad a recording.
        for s in 0..2 {
            self.reset_normalizer(s);
        }
        self.snapshot.user = self.stats[0].clone();
        self.snapshot.remote = self.stats[1].clone();
        for recording in self.recordings.iter_mut().flatten() {
            recording.finish().map_err(|e| e.to_string())?;
        }
        self.snapshot.capture_state = CaptureState::Stopped;
        self.snapshot.elapsed_seconds = self.start.elapsed().as_secs_f64();
        self.event(json!({"type":"session_stop","snapshot":self.snapshot}))?;
        self.telemetry.flush().map_err(|e| e.to_string())?;
        self.stopped = true;
        if self.snapshot.teardown_failures > 0 {
            return Err(
                "Native teardown failed; inspect telemetry and restart the test application".into(),
            );
        }
        Ok(self.snapshot())
    }
}
impl Drop for MacAudioBackend {
    fn drop(&mut self) {
        let _ = self.stop();
    }
}
fn source_name(s: usize) -> &'static str {
    if s == 0 {
        "USER"
    } else {
        "REMOTE"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn permission_gate_does_not_require_headphones_and_never_assumes_permission() {
        let mut p = Preflight {
            microphone: "NOT_DETERMINED".into(),
            system_audio: "UNKNOWN".into(),
            system_audio_evidence: "".into(),
            input_name: "".into(),
            input_id: 1,
            input_sample_rate: 48000.0,
            input_channels: 1,
            route: Route {
                id: 1,
                uid: "speaker".into(),
                name: "Speakers".into(),
                kind: "SPEAKERS".into(),
                transport: 0,
                data_source: 0,
                terminal: 0,
                sample_rate: 48000.0,
                channels: 2,
            },
        };
        assert!(gate(&p, false).unwrap_err().contains("Microphone"));
        p.microphone = "GRANTED".into();
        assert!(gate(&p, false).unwrap_err().contains("System Audio"));
        assert!(gate(&p, true).is_ok());
        p.system_audio = "DENIED".into();
        assert!(gate(&p, true).is_err());
        p.system_audio = "NOT_DETERMINED".into();
        assert!(gate(&p, true).is_err());
        p.microphone = "DENIED".into();
        assert!(gate(&p, true).is_err());
    }
    #[test]
    fn resampler_uses_capture_timestamps_and_preserves_signal() {
        let mut n = Normalizer::new(48000.0).unwrap();
        let mut output = vec![];
        for i in 0..48 {
            let samples = vec![0.25; 1000];
            output.extend(
                n.push(&samples, 1_000_000_000 + i * 1_000_000_000 / 48)
                    .unwrap(),
            );
        }
        assert_eq!(output[0].0, 1_000_000_000);
        let total: usize = output.iter().map(|(_, s)| s.len()).sum();
        assert!((15_500..16_001).contains(&total));
        assert!((output.last().unwrap().1[100] - 0.25).abs() < 0.001);
        assert!(output.windows(2).all(|w| w[1].0 > w[0].0));
    }
}
