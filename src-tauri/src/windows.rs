//! WASAPI shared-mode capture. All COM objects stay on the owning polling thread.
//! Windows' audio engine performs channel conversion and resampling to PCM16/16 kHz.
use crate::{audio::*, diagnostics::*};
use ::windows::{
    core::{HSTRING, PWSTR},
    Win32::{
        Devices::FunctionDiscovery::PKEY_Device_FriendlyName,
        Foundation::FILETIME,
        Media::Audio::*,
        System::{
            Com::{
                StructuredStorage::{PropVariantClear, PropVariantToStringAlloc},
                *,
            },
            Performance::{QueryPerformanceCounter, QueryPerformanceFrequency},
            ProcessStatus::{GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS_EX},
            Threading::{GetCurrentProcess, GetProcessTimes},
        },
    },
};
use serde::Serialize;
use serde_json::{json, Value};
use std::{
    fs::{self, File},
    io::{BufWriter, Write},
    marker::PhantomData,
    path::PathBuf,
    rc::Rc,
    time::{Duration, Instant},
};

fn win<T>(r: ::windows::core::Result<T>) -> Result<T, String> {
    r.map_err(|e| format!("WASAPI: {e} (HRESULT {:#010x})", e.code().0))
}
struct Com(PhantomData<Rc<()>>);
impl Com {
    fn new() -> Result<Self, String> {
        win(unsafe { CoInitializeEx(None, COINIT_MULTITHREADED).ok() })?;
        Ok(Self(PhantomData))
    }
}
impl Drop for Com {
    fn drop(&mut self) {
        unsafe { CoUninitialize() };
    }
}
unsafe fn owned_string(p: PWSTR) -> String {
    let text = p
        .to_string()
        .unwrap_or_else(|_| "<invalid device name>".into());
    CoTaskMemFree(Some(p.0.cast()));
    text
}
fn enumerator() -> Result<IMMDeviceEnumerator, String> {
    win(unsafe { CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL) })
}
fn device_id(device: &IMMDevice) -> Result<String, String> {
    Ok(unsafe { owned_string(win(device.GetId())?) })
}
#[derive(Clone, Debug, Serialize)]
pub struct Endpoint {
    pub uid: String,
    pub name: String,
    pub sample_rate: u32,
    pub channels: u16,
    pub active_sessions: u32,
}
fn endpoint(device: &IMMDevice, render: bool) -> Result<Endpoint, String> {
    unsafe {
        let uid = device_id(device)?;
        let properties = win(device.OpenPropertyStore(STGM_READ))?;
        let mut value = win(properties.GetValue(&PKEY_Device_FriendlyName))?;
        let name = PropVariantToStringAlloc(&value);
        let clear = PropVariantClear(&mut value);
        // Free the returned string even if clearing the property fails.
        let name = owned_string(win(name)?);
        win(clear)?;
        let client: IAudioClient = win(device.Activate(CLSCTX_ALL, None))?;
        let format = win(client.GetMixFormat())?;
        let (sample_rate, channels) = ((*format).nSamplesPerSec, (*format).nChannels);
        CoTaskMemFree(Some(format.cast()));
        if sample_rate == 0 || channels == 0 {
            return Err("Invalid endpoint mix format".into());
        }
        let mut active_sessions = 0;
        if render {
            let manager: IAudioSessionManager2 = win(device.Activate(CLSCTX_ALL, None))?;
            let sessions = win(manager.GetSessionEnumerator())?;
            for i in 0..win(sessions.GetCount())? {
                if win(win(sessions.GetSession(i))?.GetState())? == AudioSessionStateActive {
                    active_sessions += 1;
                }
            }
        }
        Ok(Endpoint {
            uid,
            name,
            sample_rate,
            channels,
            active_sessions,
        })
    }
}
fn endpoints(e: &IMMDeviceEnumerator, flow: EDataFlow) -> Result<Vec<Endpoint>, String> {
    unsafe {
        let devices = win(e.EnumAudioEndpoints(flow, DEVICE_STATE_ACTIVE))?;
        (0..win(devices.GetCount())?)
            .map(|i| endpoint(&win(devices.Item(i))?, flow == eRender))
            .collect()
    }
}
fn default_id(e: &IMMDeviceEnumerator, flow: EDataFlow, role: ERole) -> Option<String> {
    unsafe {
        e.GetDefaultAudioEndpoint(flow, role)
            .ok()
            .and_then(|d| device_id(&d).ok())
    }
}
#[derive(Clone, Debug, Serialize)]
pub struct Preflight {
    pub microphone: String,
    pub system_audio: String,
    pub input_name: String,
    pub route: Endpoint,
    pub input: Endpoint,
    pub outputs: Vec<Endpoint>,
    pub inputs: Vec<Endpoint>,
    pub default_multimedia_output: Option<String>,
    pub default_communications_output: Option<String>,
    pub output_selection: String,
}
// An explicit ID handles per-app routing; auto selects the sole endpoint with active sessions.
// Multiple active outputs are ambiguous: never silently claim the wrong meeting endpoint.
fn choose_output(
    outputs: &[Endpoint],
    requested: Option<&str>,
    multimedia: Option<&str>,
    communications: Option<&str>,
) -> Result<String, String> {
    let id = match requested {
        Some("multimedia") => multimedia.ok_or("No default multimedia output")?,
        Some("communications") => communications.ok_or("No default communications output")?,
        Some(id) => id,
        None => {
            let active: Vec<_> = outputs.iter().filter(|d| d.active_sessions > 0).collect();
            match active.as_slice() {
                [one] => &one.uid,
                [] => multimedia.ok_or("No default output")?,
                _ => return Err("Multiple outputs have active audio sessions; use --output-endpoint ID matching the meeting application's speaker setting (see `audio-diag endpoints`)".into()),
            }
        }
    };
    outputs
        .iter()
        .find(|d| d.uid == id)
        .map(|d| d.uid.clone())
        .ok_or_else(|| format!("Output endpoint unavailable: {id}"))
}
fn inspect(output: Option<&str>, input: Option<&str>) -> Result<Preflight, String> {
    let e = enumerator()?;
    let outputs = endpoints(&e, eRender)?;
    let inputs = endpoints(&e, eCapture)?;
    let multimedia = default_id(&e, eRender, eMultimedia);
    let communications = default_id(&e, eRender, eCommunications);
    let selected = choose_output(
        &outputs,
        output,
        multimedia.as_deref(),
        communications.as_deref(),
    )?;
    let input_id = input
        .map(str::to_owned)
        .or_else(|| default_id(&e, eCapture, eCommunications))
        .ok_or("No microphone endpoint")?;
    let input = inputs
        .iter()
        .find(|d| d.uid == input_id)
        .cloned()
        .ok_or("Microphone endpoint unavailable")?;
    Ok(Preflight {
        microphone: "CHECKED_ON_START".into(),
        system_audio: "WASAPI_LOOPBACK".into(),
        input_name: input.name.clone(),
        route: outputs.iter().find(|d| d.uid == selected).unwrap().clone(),
        input,
        outputs,
        inputs,
        default_multimedia_output: multimedia,
        default_communications_output: communications,
        output_selection: output
            .unwrap_or("auto: sole active output, otherwise multimedia default")
            .into(),
    })
}
pub fn preflight(output: Option<&str>, input: Option<&str>) -> Result<Preflight, String> {
    let _com = Com::new()?;
    inspect(output, input)
}
pub fn list_endpoints() -> Result<Value, String> {
    let _com = Com::new()?;
    let e = enumerator()?;
    Ok(
        json!({"outputs":endpoints(&e,eRender)?, "inputs":endpoints(&e,eCapture)?,
        "default_multimedia_output":default_id(&e,eRender,eMultimedia),
        "default_communications_output":default_id(&e,eRender,eCommunications),
        "default_communications_input":default_id(&e,eCapture,eCommunications)}),
    )
}
fn qpc_100ns() -> Result<u64, String> {
    let (mut count, mut frequency) = (0, 0);
    unsafe {
        win(QueryPerformanceFrequency(&mut frequency))?;
        win(QueryPerformanceCounter(&mut count))?;
    }
    if count < 0 || frequency <= 0 {
        return Err("Invalid QPC clock".into());
    }
    Ok((count as u128 * 10_000_000 / frequency as u128) as u64)
}
struct Packet {
    pcm: Vec<u8>,
    ticks: u64,
    position: u64,
    flags: u32,
}
struct Stream {
    capture: IAudioCaptureClient,
    client: IAudioClient,
    endpoint: Endpoint,
    buffer_frames: u32,
    started: bool,
}
impl Stream {
    fn start(endpoint: Endpoint, remote: bool) -> Result<Self, String> {
        unsafe {
            let e = enumerator()?;
            let device = win(e.GetDevice(&HSTRING::from(&endpoint.uid)))?;
            let client: IAudioClient = win(device.Activate(CLSCTX_ALL, None))?;
            let format = WAVEFORMATEX {
                wFormatTag: 1,
                nChannels: CHANNELS,
                nSamplesPerSec: SAMPLE_RATE,
                nAvgBytesPerSec: SAMPLE_RATE * 2,
                nBlockAlign: 2,
                wBitsPerSample: 16,
                cbSize: 0,
            };
            let flags = AUDCLNT_STREAMFLAGS_AUTOCONVERTPCM
                | AUDCLNT_STREAMFLAGS_SRC_DEFAULT_QUALITY
                | if remote {
                    AUDCLNT_STREAMFLAGS_LOOPBACK
                } else {
                    0
                };
            win(client.Initialize(AUDCLNT_SHAREMODE_SHARED, flags, 2_000_000, 0, &format, None))
                .map_err(|e| format!("{} [{}]: {e}; for USER access errors enable Windows microphone access for desktop apps", endpoint.name, endpoint.uid))?;
            let capture = win(client.GetService())?;
            let buffer_frames = win(client.GetBufferSize())?;
            win(client.Start())?;
            Ok(Self {
                client,
                capture,
                endpoint,
                buffer_frames,
                started: true,
            })
        }
    }
    fn read(&self) -> Result<Option<Packet>, String> {
        unsafe {
            if win(self.capture.GetNextPacketSize())? == 0 {
                return Ok(None);
            }
            let (mut data, mut count, mut flags, mut position, mut ticks) =
                (std::ptr::null_mut(), 0, 0, 0, 0);
            win(self.capture.GetBuffer(
                &mut data,
                &mut count,
                &mut flags,
                Some(&mut position),
                Some(&mut ticks),
            ))?;
            let result = if count == 0 || count > self.buffer_frames || count > SAMPLE_RATE * 2 {
                Err("Invalid WASAPI packet length".into())
            } else if flags & AUDCLNT_BUFFERFLAGS_SILENT.0 as u32 != 0 {
                Ok(Packet {
                    pcm: vec![0; count as usize * 2],
                    ticks,
                    position,
                    flags,
                })
            } else if data.is_null() {
                Err("Null WASAPI packet".into())
            } else {
                Ok(Packet {
                    pcm: std::slice::from_raw_parts(data, count as usize * 2).to_vec(),
                    ticks,
                    position,
                    flags,
                })
            };
            win(self.capture.ReleaseBuffer(count))?;
            result.map(Some)
        }
    }
    fn stop(&mut self) -> Result<(), String> {
        if self.started {
            win(unsafe { self.client.Stop() })?;
            self.started = false;
        }
        Ok(())
    }
}
impl Drop for Stream {
    fn drop(&mut self) {
        let _ = self.stop();
    }
}
#[derive(Clone, Debug, Default, Serialize)]
pub struct SourceStats {
    pub sequence: u64,
    pub native_sequence: u64,
    pub generation: u64,
    pub rms: Option<f64>,
    pub peak: Option<f64>,
    pub samples: u64,
    pub dropped_frames: u64,
    pub missing_samples: u64,
    pub timestamp_errors: u64,
    pub discontinuities: u64,
    pub backwards_timestamps: u64,
    pub drift_ns: Option<f64>,
    pub last_timeline_ns: Option<i64>,
    pub first_timeline_ns: Option<i64>,
    pub original_sample_rate: u32,
    pub original_channels: u16,
    pub last_packet_elapsed_seconds: Option<f64>,
}
#[derive(Clone, Debug, Serialize)]
pub struct Snapshot {
    pub capture_state: CaptureState,
    pub elapsed_seconds: f64,
    pub route: Endpoint,
    pub input: Endpoint,
    pub user: SourceStats,
    pub remote: SourceStats,
    pub relative_drift_ns: Option<f64>,
    pub initial_source_offset_ns: Option<i128>,
    pub restarts: u64,
    pub route_changes: u64,
    pub memory_bytes: Option<u64>,
    pub working_set_bytes: Option<u64>,
    pub cpu_percent: Option<f64>,
    pub teardown_failures: u64,
    pub warning: Option<String>,
}
pub struct Options {
    pub telemetry: PathBuf,
    pub recordings: Option<PathBuf>,
    pub output_endpoint: Option<String>,
    pub input_endpoint: Option<String>,
}
pub struct WindowsAudioBackend {
    streams: [Stream; 2],
    // Drop COM interfaces before uninitializing their apartment. Rc marker prevents moving threads.
    _com: Com,
    options: Options,
    timeline: HostTimeline,
    start: Instant,
    next_sample: Instant,
    next_route: Instant,
    stats: [SourceStats; 2],
    drift: [Drift; 2],
    previous_end: [Option<u64>; 2],
    pending_discontinuity: [bool; 2],
    snapshot: Snapshot,
    telemetry: BufWriter<File>,
    recordings: [Option<WavWriter>; 2],
    last_cpu: Option<(Instant, f64)>,
    stopped: bool,
}
impl WindowsAudioBackend {
    pub fn start(options: Options) -> Result<Self, String> {
        let com = Com::new()?;
        let p = inspect(
            options.output_endpoint.as_deref(),
            options.input_endpoint.as_deref(),
        )?;
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
        if let Some(dir) = &options.recordings {
            fs::create_dir_all(dir).map_err(|e| e.to_string())?;
            recordings = [
                Some(WavWriter::create(&dir.join("USER.wav")).map_err(|e| e.to_string())?),
                Some(WavWriter::create(&dir.join("REMOTE.wav")).map_err(|e| e.to_string())?),
            ];
        }
        // GetBuffer already converts QPC to 100 ns; do NOT apply QPC frequency a second time.
        let timeline = HostTimeline::new(qpc_100ns()?, 100, 1)?;
        let start = Instant::now();
        let streams = [
            Stream::start(p.input.clone(), false)?,
            Stream::start(p.route.clone(), true)?,
        ];
        let snapshot = Snapshot {
            capture_state: CaptureState::Starting,
            elapsed_seconds: 0.0,
            route: p.route.clone(),
            input: p.input.clone(),
            user: SourceStats::default(),
            remote: SourceStats::default(),
            relative_drift_ns: None,
            initial_source_offset_ns: None,
            restarts: 0,
            route_changes: 0,
            memory_bytes: None,
            working_set_bytes: None,
            cpu_percent: None,
            teardown_failures: 0,
            warning: None,
        };
        let mut backend = Self {
            streams,
            _com: com,
            options,
            timeline,
            start,
            next_sample: start,
            next_route: start,
            stats: Default::default(),
            drift: Default::default(),
            previous_end: [None; 2],
            pending_discontinuity: [false; 2],
            snapshot,
            telemetry,
            recordings,
            last_cpu: None,
            stopped: false,
        };
        backend.event(json!({"type":"session_start","platform":"windows","clock":timeline,"preflight":p,
            "timestamp_basis":"WASAPI GetBuffer QPC position in 100 ns units",
            "normalization":"Windows audio engine AUTOCONVERTPCM + SRC_DEFAULT_QUALITY",
            "format":{"rate":SAMPLE_RATE,"channels":CHANNELS,"encoding":"PCM_S16LE"},"speaker_mode_allowed":true}))?;
        backend.event(json!({"type":"capture_started","route":backend.snapshot.route,"input":backend.snapshot.input}))?;
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
    fn process(&mut self, s: usize, packet: Packet) -> Result<(), String> {
        let source = if s == 0 { "USER" } else { "REMOTE" };
        let n = packet.pcm.len() / 2;
        let sequence = self.stats[s].native_sequence;
        self.stats[s].native_sequence += 1;
        if packet.flags & AUDCLNT_BUFFERFLAGS_TIMESTAMP_ERROR.0 as u32 != 0 {
            self.stats[s].timestamp_errors += 1;
            self.stats[s].dropped_frames += 1;
            self.stats[s].missing_samples += n as u64;
            self.pending_discontinuity[s] = true;
            return self.event(json!({"type":"timestamp_error","source":source,"native_sequence":sequence,"discarded_samples":n}));
        }
        let native_ns = self.timeline.native_ns(packet.ticks);
        let time = self.timeline.timeline_ns(packet.ticks)?;
        let d = self.drift[s].observe(native_ns, n, SAMPLE_RATE as f64);
        if d.backwards || self.stats[s].last_timeline_ns.is_some_and(|t| time < t) {
            self.stats[s].backwards_timestamps += 1;
            self.event(json!({"type":"timestamp_backwards","source":source,"timeline_ns":time}))?;
            return Err(format!("{source} capture timestamp went backwards"));
        }
        let missing = self.previous_end[s].map_or(0, |end| packet.position.saturating_sub(end));
        let gap = self.previous_end[s].is_some_and(|end| end != packet.position);
        let discontinuity = self.pending_discontinuity[s]
            || gap
            || d.step_ns.abs() > 2_000_000.0
            || packet.flags & AUDCLNT_BUFFERFLAGS_DATA_DISCONTINUITY.0 as u32 != 0;
        if gap
            || (sequence > 0 && packet.flags & AUDCLNT_BUFFERFLAGS_DATA_DISCONTINUITY.0 as u32 != 0)
        {
            self.stats[s].dropped_frames += 1;
        }
        self.stats[s].missing_samples += missing;
        self.previous_end[s] = Some(packet.position + n as u64);
        if discontinuity {
            self.stats[s].discontinuities += 1;
        }
        let original = &self.streams[s].endpoint;
        self.stats[s].original_sample_rate = original.sample_rate;
        self.stats[s].original_channels = original.channels;
        let metadata = FrameMetadata {
            source: if s == 0 { Source::User } else { Source::Remote },
            original_format: SourceFormat {
                sample_rate: original.sample_rate,
                channels: original.channels,
            },
            capture_timestamp_ns: native_ns,
            capture_clock: CaptureClock::WindowsQpc,
            timeline_timestamp_ns: time,
            sequence: self.stats[s].sequence,
            discontinuity,
        };
        self.event(json!({"type":"native_frame","source":source,"native_host_ticks":packet.ticks,
            "native_capture_ns":native_ns,"timeline_ns":time,"native_sequence":sequence,"generation":self.stats[s].generation,
            "native_samples":n,"native_rate":SAMPLE_RATE,"native_channels":CHANNELS,
            "original_sample_rate":metadata.original_format.sample_rate,"original_channels":metadata.original_format.channels,
            "device_position":packet.position,"wasapi_flags":packet.flags,"drift":d,"discontinuity":discontinuity}))?;
        let frame = NormalizedFrame::new(metadata, packet.pcm)
            .map_err(|e| format!("Invalid normalized frame: {e:?}"))?;
        let samples: Vec<f32> = frame
            .pcm_le()
            .chunks_exact(2)
            .map(|b| i16::from_le_bytes([b[0], b[1]]) as f32 / 32768.0)
            .collect();
        let (rms, peak) = levels(&samples);
        let offset = self.recordings[s]
            .as_mut()
            .map(|w| w.write(frame.pcm_le()))
            .transpose()
            .map_err(|e| e.to_string())?;
        self.stats[s].rms = Some(rms);
        self.stats[s].peak = Some(peak);
        self.stats[s].drift_ns = Some(d.drift_ns);
        self.stats[s].first_timeline_ns.get_or_insert(time);
        self.stats[s].last_timeline_ns = Some(time);
        self.stats[s].last_packet_elapsed_seconds = Some(self.start.elapsed().as_secs_f64());
        self.stats[s].samples += n as u64;
        self.event(json!({"type":"normalized_frame","source":source,"sequence":metadata.sequence,"native_sequence":sequence,
            "generation":self.stats[s].generation,"timeline_ns":time,"capture_ns":native_ns,
            "timestamp_basis":"WASAPI GetBuffer QPC position (100 ns)","sample_count":n,"rms":rms,"peak":peak,
            "drift_ns":d.drift_ns,"discontinuity":discontinuity,"recording_sample_offset":offset}))?;
        self.stats[s].sequence += 1;
        self.pending_discontinuity[s] = false;
        Ok(())
    }
    fn drain(&mut self) -> Result<(), String> {
        // Bounded reads keep stop/route monitoring responsive even with a continuously busy endpoint.
        for s in 0..2 {
            for _ in 0..64 {
                match self.streams[s].read()? {
                    Some(packet) => self.process(s, packet)?,
                    None => break,
                }
            }
        }
        Ok(())
    }
    pub fn poll(&mut self, timeout: Duration) -> Result<(), String> {
        if self.stopped {
            return Err("Capture already stopped".into());
        }
        let result = self.poll_inner(timeout);
        if let Err(e) = &result {
            self.snapshot.capture_state = CaptureState::ActualCaptureFailure;
            let _ = self.event(json!({"type":"capture_error","error":e}));
        }
        result
    }
    fn poll_inner(&mut self, timeout: Duration) -> Result<(), String> {
        self.drain()?;
        let now = Instant::now();
        if now >= self.next_route {
            self.next_route = now + Duration::from_millis(500);
            let p = inspect(
                self.options.output_endpoint.as_deref(),
                self.options.input_endpoint.as_deref(),
            )?;
            for (s, endpoint) in [p.input, p.route].into_iter().enumerate() {
                let old = &self.streams[s].endpoint;
                if old.uid != endpoint.uid
                    || old.sample_rate != endpoint.sample_rate
                    || old.channels != endpoint.channels
                {
                    self.event(json!({"type":"route_recovery","event":"StopRebuildResume","source":if s==0 {"USER"} else {"REMOTE"},"old":self.streams[s].endpoint,"new":endpoint}))?;
                    self.streams[s].stop()?;
                    self.streams[s] = Stream::start(endpoint, s == 1)?;
                    self.snapshot.restarts += 1;
                    self.snapshot.route_changes += 1;
                    self.stats[s].generation += 1;
                    self.stats[s].native_sequence = 0;
                    self.previous_end[s] = None;
                    self.pending_discontinuity[s] = true;
                    // Keep the common epoch; drift explicitly includes the route gap.
                }
            }
        }
        if now >= self.next_sample {
            self.next_sample = now + Duration::from_secs(1);
            self.resources()?;
            self.update_snapshot();
            self.event(json!({"type":"telemetry","snapshot":self.snapshot}))?;
            self.telemetry.flush().map_err(|e| e.to_string())?;
            if self.snapshot.elapsed_seconds > 5.0
                && self.stats[0]
                    .last_packet_elapsed_seconds
                    .is_none_or(|t| self.snapshot.elapsed_seconds - t > 5.0)
            {
                return Err("USER capture stalled for more than five seconds".into());
            }
        }
        std::thread::sleep(timeout.min(Duration::from_millis(5)));
        Ok(())
    }
    fn resources(&mut self) -> Result<(), String> {
        unsafe {
            let process = GetCurrentProcess();
            let mut memory = PROCESS_MEMORY_COUNTERS_EX::default();
            memory.cb = std::mem::size_of_val(&memory) as u32;
            win(GetProcessMemoryInfo(
                process,
                (&mut memory as *mut PROCESS_MEMORY_COUNTERS_EX).cast(),
                memory.cb,
            ))?;
            let (mut created, mut exited, mut kernel, mut user) = (
                FILETIME::default(),
                FILETIME::default(),
                FILETIME::default(),
                FILETIME::default(),
            );
            win(GetProcessTimes(
                process,
                &mut created,
                &mut exited,
                &mut kernel,
                &mut user,
            ))?;
            let seconds = |t: FILETIME| {
                ((t.dwHighDateTime as u64) << 32 | t.dwLowDateTime as u64) as f64 / 1e7
            };
            let cpu = seconds(kernel) + seconds(user);
            let now = Instant::now();
            self.snapshot.cpu_percent = self
                .last_cpu
                .map(|(t, c)| 100.0 * (cpu - c) / now.duration_since(t).as_secs_f64());
            self.last_cpu = Some((now, cpu));
            self.snapshot.memory_bytes = Some(memory.PrivateUsage as u64);
            self.snapshot.working_set_bytes = Some(memory.WorkingSetSize as u64);
            self.event(json!({"type":"resources","memory_bytes":memory.PrivateUsage,"memory_kind":"private_bytes",
                "working_set_bytes":memory.WorkingSetSize,"cpu_percent":self.snapshot.cpu_percent}))
        }
    }
    fn update_snapshot(&mut self) {
        let elapsed = self.start.elapsed().as_secs_f64();
        self.snapshot.elapsed_seconds = elapsed;
        self.snapshot.route = self.streams[1].endpoint.clone();
        self.snapshot.input = self.streams[0].endpoint.clone();
        self.snapshot.user = self.stats[0].clone();
        self.snapshot.remote = self.stats[1].clone();
        self.snapshot.relative_drift_ns = self.stats[0]
            .drift_ns
            .zip(self.stats[1].drift_ns)
            .map(|(u, r)| u - r);
        self.snapshot.initial_source_offset_ns = self.stats[0]
            .first_timeline_ns
            .zip(self.stats[1].first_timeline_ns)
            .map(|(u, r)| u as i128 - r as i128);
        if self.snapshot.capture_state != CaptureState::ActualCaptureFailure {
            self.snapshot.capture_state = if self.stats[1]
                .last_packet_elapsed_seconds
                .is_none_or(|t| elapsed - t > 2.0)
            {
                CaptureState::IdleNoRenderAudio
            } else {
                CaptureState::ActiveAudio
            };
        }
        for s in [&mut self.snapshot.user, &mut self.snapshot.remote] {
            if s.last_packet_elapsed_seconds
                .is_none_or(|t| elapsed - t > 2.0)
            {
                s.rms = None;
                s.peak = None;
            }
        }
    }
    pub fn stop(&mut self) -> Result<Snapshot, String> {
        if self.stopped {
            return Ok(self.snapshot());
        }
        let mut errors = vec![];
        for stream in &mut self.streams {
            if let Err(e) = stream.stop() {
                self.snapshot.teardown_failures += 1;
                errors.push(e);
            }
        }
        if let Err(e) = self.drain() {
            errors.push(e);
        }
        for wav in self.recordings.iter_mut().flatten() {
            if let Err(e) = wav.finish() {
                errors.push(e.to_string());
            }
        }
        self.update_snapshot();
        if !errors.is_empty() {
            self.snapshot.capture_state = CaptureState::ActualCaptureFailure;
        } else if self.snapshot.capture_state != CaptureState::ActualCaptureFailure {
            self.snapshot.capture_state = CaptureState::Stopped;
        }
        self.stopped = true;
        self.event(json!({"type":"session_stop","snapshot":self.snapshot,"errors":errors}))?;
        self.telemetry.flush().map_err(|e| e.to_string())?;
        if errors.is_empty() {
            Ok(self.snapshot())
        } else {
            Err(errors.join("; "))
        }
    }
}
impl Drop for WindowsAudioBackend {
    fn drop(&mut self) {
        let _ = self.stop();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn selects_actual_active_output_and_requires_explicit_choice_when_ambiguous() {
        let mut endpoints = vec![
            Endpoint {
                uid: "default".into(),
                name: "Speakers".into(),
                sample_rate: 48000,
                channels: 2,
                active_sessions: 0,
            },
            Endpoint {
                uid: "meeting".into(),
                name: "Monitor".into(),
                sample_rate: 44100,
                channels: 2,
                active_sessions: 1,
            },
        ];
        assert_eq!(
            choose_output(&endpoints, None, Some("default"), Some("meeting")).unwrap(),
            "meeting"
        );
        endpoints[0].active_sessions = 1;
        assert!(choose_output(&endpoints, None, Some("default"), None).is_err());
        assert_eq!(
            choose_output(&endpoints, Some("meeting"), None, None).unwrap(),
            "meeting"
        );
        assert!(choose_output(&endpoints, Some("missing"), None, None).is_err());
        endpoints.iter_mut().for_each(|e| e.active_sessions = 0);
        assert_eq!(
            choose_output(&endpoints, None, Some("default"), None).unwrap(),
            "default"
        );
        let timeline = HostTimeline::new(10_000_000, 100, 1).unwrap();
        assert_eq!(timeline.timeline_ns(10_100_000).unwrap(), 10_000_000);
        assert_eq!(timeline.native_ns(10_100_000), 1_010_000_000);
    }
}
