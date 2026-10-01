//! Pure timing, recovery and observational analysis, shared by CLI and desktop.
use rustfft::{num_complex::Complex, FftPlanner};
use serde::{Deserialize, Serialize};
use std::{
    collections::VecDeque,
    fs::File,
    io::{self, Seek, SeekFrom, Write},
    path::Path,
};

#[derive(Clone, Copy, Debug, Serialize)]
pub struct HostTimeline {
    pub origin_ticks: u64,
    pub numer: u32,
    pub denom: u32,
}
impl HostTimeline {
    pub fn new(origin_ticks: u64, numer: u32, denom: u32) -> Result<Self, String> {
        if numer == 0 || denom == 0 {
            return Err("invalid host clock timebase".into());
        }
        Ok(Self {
            origin_ticks,
            numer,
            denom,
        })
    }
    pub fn native_ns(&self, ticks: u64) -> u64 {
        ((ticks as u128 * self.numer as u128) / self.denom as u128) as u64
    }
    pub fn timeline_ns(&self, ticks: u64) -> Result<i64, String> {
        let delta = ticks as i128 - self.origin_ticks as i128;
        i64::try_from(delta * self.numer as i128 / self.denom as i128)
            .map_err(|_| "timeline overflow".into())
    }
    pub fn native_from_timeline_ns(&self, timestamp: i64) -> Result<u64, String> {
        u64::try_from(self.native_ns(self.origin_ticks) as i128 + timestamp as i128)
            .map_err(|_| "native timestamp overflow".into())
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Route {
    pub id: u32,
    pub uid: String,
    pub name: String,
    pub kind: String,
    pub transport: u32,
    pub data_source: u32,
    pub terminal: u32,
    pub sample_rate: f64,
    pub channels: u32,
}
#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum CaptureState {
    Stopped,
    Starting,
    ActiveAudio,
    IdleNoRenderAudio,
    Recovering,
    Warning,
    ActualCaptureFailure,
    PermissionRequired,
}

#[derive(Debug)]
pub struct Recovery {
    pub state: CaptureState,
    pub generation: u64,
    pub restarts: u64,
    pub discontinuities: u64,
    pub warning: bool,
}
impl Default for Recovery {
    fn default() -> Self {
        Self {
            state: CaptureState::Stopped,
            generation: 0,
            restarts: 0,
            discontinuities: 0,
            warning: false,
        }
    }
}
impl Recovery {
    /// The production controller performs Stop -> Rebuild -> Resume inside this operation.
    /// Returning the transition list makes the exact sequence observable and testable.
    pub fn rebuild<T>(
        &mut self,
        old: &mut Option<T>,
        route: &Route,
        start: impl FnOnce() -> Result<T, String>,
    ) -> Vec<String> {
        self.state = CaptureState::Recovering;
        self.warning = route.kind != "HEADPHONES";
        self.generation += 1;
        self.discontinuities += 1;
        let mut events = vec!["RouteChanged".into(), "RemoteInvalidated".into()];
        drop(old.take()); // Native Drop stops IO before destroying tap/aggregate.
        if self.warning {
            events.push("AcousticBleedWarning".into());
        }
        events.push("RemoteRebuilding".into());
        match start() {
            Ok(stream) => {
                *old = Some(stream);
                self.restarts += 1;
                self.state = if self.warning {
                    CaptureState::Warning
                } else {
                    CaptureState::Starting
                };
                events.push("RemoteResumedDiscontinuous".into());
            }
            Err(e) => {
                self.state = CaptureState::ActualCaptureFailure;
                events.push(format!("RemoteRecoveryFailed: {e}"));
            }
        }
        events
    }
}

#[derive(Default, Debug)]
pub struct Drift {
    first_ns: Option<u64>,
    expected_ns: f64,
    previous_ns: Option<u64>,
    previous_duration_ns: f64,
}
#[derive(Clone, Debug, Serialize)]
pub struct DriftPoint {
    pub drift_ns: f64,
    pub step_ns: f64,
    pub backwards: bool,
}
impl Drift {
    pub fn observe(&mut self, native_ns: u64, frames: usize, rate: f64) -> DriftPoint {
        let first = *self.first_ns.get_or_insert(native_ns);
        let drift = native_ns as f64 - first as f64 - self.expected_ns;
        let step = self
            .previous_ns
            .map(|p| native_ns as f64 - p as f64 - self.previous_duration_ns)
            .unwrap_or(0.0);
        let backwards = self.previous_ns.is_some_and(|p| native_ns < p);
        let duration = frames as f64 * 1e9 / rate;
        self.expected_ns += duration;
        self.previous_ns = Some(native_ns);
        self.previous_duration_ns = duration;
        DriftPoint {
            drift_ns: drift,
            step_ns: step,
            backwards,
        }
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct Correlation {
    pub maximum: f64,
    pub lag_ms: f64,
    pub window_start_ns: i64,
    pub window_duration_ms: u64,
}
/// REMOTE precedes USER by 0..300 ms. Exact 16 kHz lag grid, mean-centered NCC.
/// FFT computes all dot products; prefix sums normalize each candidate independently.
pub fn correlate(remote: &[f32], user: &[f32], start_ns: i64) -> Option<Correlation> {
    let n = remote.len();
    let max_lag = 4_800;
    if n < 16 || user.len() < n + max_lag {
        return None;
    }
    let size = (n + user.len() - 1).next_power_of_two();
    let mut a = vec![Complex::new(0.0f64, 0.0); size];
    let mut b = a.clone();
    for (i, v) in remote.iter().rev().enumerate() {
        a[i].re = *v as f64;
    }
    for (i, v) in user.iter().enumerate() {
        b[i].re = *v as f64;
    }
    let mut planner = FftPlanner::new();
    let fft = planner.plan_fft_forward(size);
    fft.process(&mut a);
    fft.process(&mut b);
    for (a, b) in a.iter_mut().zip(b) {
        *a *= b;
    }
    planner.plan_fft_inverse(size).process(&mut a);
    let sum_r: f64 = remote.iter().map(|v| *v as f64).sum();
    let energy_r =
        remote.iter().map(|v| (*v as f64).powi(2)).sum::<f64>() - sum_r.powi(2) / n as f64;
    if energy_r <= 1e-12 {
        return None;
    }
    let mut sums = vec![0.0; user.len() + 1];
    let mut squares = sums.clone();
    for (i, v) in user.iter().enumerate() {
        sums[i + 1] = sums[i] + *v as f64;
        squares[i + 1] = squares[i] + (*v as f64).powi(2);
    }
    let mut best: Option<(f64, usize)> = None;
    for lag in 0..=max_lag {
        let sum = sums[lag + n] - sums[lag];
        let energy = squares[lag + n] - squares[lag] - sum.powi(2) / n as f64;
        if energy <= 1e-12 {
            continue;
        }
        let value = ((a[n - 1 + lag].re / size as f64 - sum_r * sum / n as f64)
            / (energy_r * energy).sqrt())
        .clamp(-1.0, 1.0);
        if best.is_none_or(|(v, _)| value > v) {
            best = Some((value, lag));
        }
    }
    best.map(|(maximum, lag)| Correlation {
        maximum,
        lag_ms: lag as f64 / 16.0,
        window_start_ns: start_ns,
        window_duration_ms: n as u64 / 16,
    })
}

#[derive(Default)]
pub struct RollingAudio {
    blocks: VecDeque<(i64, Vec<f32>)>,
}
impl RollingAudio {
    pub fn clear(&mut self) {
        self.blocks.clear();
    }
    pub fn push(&mut self, start: i64, samples: Vec<f32>) {
        self.blocks.push_back((start, samples));
        while self
            .blocks
            .front()
            .is_some_and(|(t, _)| start.saturating_sub(*t) > 3_000_000_000)
        {
            self.blocks.pop_front();
        }
    }
    pub fn end(&self) -> Option<i64> {
        self.blocks.back().map(|(t, s)| t + s.len() as i64 * 62_500)
    }
    pub fn window(&self, start: i64, len: usize) -> Option<Vec<f32>> {
        let mut result = vec![None; len];
        for (time, samples) in &self.blocks {
            let offset = ((*time as i128 - start as i128) as f64 / 62_500.0).round() as i64;
            for (i, value) in samples.iter().enumerate() {
                let index = offset + i as i64;
                if index >= 0 && index < len as i64 {
                    result[index as usize] = Some(*value);
                }
            }
        }
        result.into_iter().collect() // Missing data invalidates the measurement, never fill with silence.
    }
}

pub fn levels(samples: &[f32]) -> (f64, f64) {
    if samples.is_empty() {
        return (0.0, 0.0);
    }
    (
        (samples.iter().map(|s| (*s as f64).powi(2)).sum::<f64>() / samples.len() as f64).sqrt(),
        samples
            .iter()
            .map(|s| (*s as f64).abs())
            .fold(0.0, f64::max),
    )
}

pub struct WavWriter {
    file: File,
    bytes: u32,
}
impl WavWriter {
    pub fn create(path: &Path) -> io::Result<Self> {
        let mut this = Self {
            file: File::options().write(true).create_new(true).open(path)?,
            bytes: 0,
        };
        this.header()?;
        Ok(this)
    }
    fn header(&mut self) -> io::Result<()> {
        self.file.seek(SeekFrom::Start(0))?;
        self.file.write_all(b"RIFF")?;
        self.file.write_all(&(36 + self.bytes).to_le_bytes())?;
        self.file.write_all(b"WAVEfmt \x10\0\0\0\x01\0\x01\0")?;
        self.file.write_all(&16_000u32.to_le_bytes())?;
        self.file.write_all(&32_000u32.to_le_bytes())?;
        self.file.write_all(b"\x02\0\x10\0data")?;
        self.file.write_all(&self.bytes.to_le_bytes())?;
        self.file.seek(SeekFrom::End(0))?;
        Ok(())
    }
    pub fn write(&mut self, pcm: &[u8]) -> io::Result<u64> {
        let offset = self.bytes as u64 / 2;
        let next = self
            .bytes
            .checked_add(u32::try_from(pcm.len()).map_err(io::Error::other)?)
            .filter(|v| *v <= u32::MAX - 36)
            .ok_or_else(|| io::Error::other("WAV exceeds RIFF limit"))?;
        self.file.write_all(pcm)?;
        self.bytes = next;
        Ok(offset)
    }
    pub fn finish(&mut self) -> io::Result<()> {
        self.header()?;
        self.file.sync_all()
    }
}
impl Drop for WavWriter {
    fn drop(&mut self) {
        let _ = self.finish();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn native_time_and_drift_preserve_step_and_backwards_jump() {
        let t = HostTimeline::new(240, 125, 3).unwrap();
        assert_eq!(t.native_ns(480), 20_000);
        assert_eq!(t.timeline_ns(480).unwrap(), 10_000);
        assert_eq!(t.timeline_ns(120).unwrap(), -5_000);
        assert_eq!(t.native_from_timeline_ns(-5_000).unwrap(), 5_000);
        let mut drift = Drift::default();
        assert_eq!(drift.observe(1_000_000_000, 480, 48000.0).drift_ns, 0.0);
        assert_eq!(
            drift.observe(1_010_100_000, 480, 48000.0).drift_ns,
            100_000.0
        );
        assert_eq!(
            drift.observe(1_025_100_000, 480, 48000.0).step_ns,
            5_000_000.0
        );
        assert!(drift.observe(1_000_000_000, 480, 48000.0).backwards);
    }
    #[test]
    fn route_recovery_stops_before_rebuild_preserves_warning_and_handles_failure() {
        use std::{cell::RefCell, rc::Rc};
        struct Stream(Rc<RefCell<Vec<&'static str>>>);
        impl Drop for Stream {
            fn drop(&mut self) {
                self.0.borrow_mut().push("stop");
            }
        }
        let calls = Rc::new(RefCell::new(vec![]));
        let mut stream = Some(Stream(calls.clone()));
        let route = Route {
            id: 1,
            uid: "speaker".into(),
            name: "Speakers".into(),
            kind: "SPEAKERS".into(),
            transport: 0,
            data_source: 0,
            terminal: 0,
            sample_rate: 48000.0,
            channels: 2,
        };
        let mut recovery = Recovery::default();
        let events = recovery.rebuild(&mut stream, &route, || {
            calls.borrow_mut().push("start");
            Ok(Stream(calls.clone()))
        });
        assert_eq!(&*calls.borrow(), &["stop", "start"]);
        assert_eq!(recovery.state, CaptureState::Warning);
        assert_eq!(recovery.discontinuities, 1);
        assert_eq!(events.last().unwrap(), "RemoteResumedDiscontinuous");
        recovery.rebuild(&mut stream, &route, || Err("device unavailable".into()));
        assert!(stream.is_none());
        assert_eq!(recovery.state, CaptureState::ActualCaptureFailure);
        let mut headphones = route;
        headphones.kind = "HEADPHONES".into();
        recovery.rebuild(&mut stream, &headphones, || Ok(Stream(calls.clone())));
        assert!(!recovery.warning);
        assert_eq!(recovery.state, CaptureState::Starting);
        assert_eq!(recovery.generation, 3);
        assert_eq!(recovery.restarts, 2);
    }
    #[test]
    fn correlation_finds_delayed_scaled_speech_like_signal_and_rejects_silence() {
        let mut seed = 7u32;
        let remote: Vec<f32> = (0..8000)
            .map(|_| {
                seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
                (seed as i32 as f32) / i32::MAX as f32
            })
            .collect();
        // Synthetic data is exclusively an algorithm unit test, never live acceptance evidence.
        for lag in [0, 1600, 4800] {
            let mut user = vec![0.03; 12800];
            for (i, v) in remote.iter().enumerate() {
                user[i + lag] = *v * 0.2 + 0.03;
            }
            let c = correlate(&remote, &user, 123).unwrap();
            assert!((c.maximum - 1.0).abs() < 1e-5);
            assert_eq!(c.lag_ms, lag as f64 / 16.0);
        }
        assert!(correlate(&vec![0.0; 8000], &vec![0.0; 12800], 0).is_none());
    }
    #[test]
    fn rolling_windows_do_not_hide_gaps() {
        let mut r = RollingAudio::default();
        r.push(0, vec![1.0; 16]);
        r.push(2_000_000, vec![1.0; 16]);
        assert!(r.window(0, 48).is_none());
        assert_eq!(r.window(0, 16).unwrap().len(), 16);
    }
    #[test]
    fn wav_header_and_samples_are_little_endian() {
        let path = std::env::temp_dir().join(format!("unmute-wav-{}.wav", std::process::id()));
        let mut wav = WavWriter::create(&path).unwrap();
        assert_eq!(wav.write(&[0, 128, 255, 127]).unwrap(), 0);
        wav.finish().unwrap();
        let data = std::fs::read(&path).unwrap();
        assert_eq!(&data[..4], b"RIFF");
        assert_eq!(&data[24..28], &16000u32.to_le_bytes());
        assert_eq!(&data[40..44], &4u32.to_le_bytes());
        assert_eq!(&data[44..], &[0, 128, 255, 127]);
        drop(wav);
        std::fs::remove_file(path).unwrap();
    }
}
