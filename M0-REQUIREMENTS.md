# M0 platform requirements

These requirements override conflicting general M0 requirements. This document
records the required behavior; it is not evidence of implementation or acceptance.

## Shared audio boundary

- Every normalized frame contains 16,000 Hz, mono, signed 16-bit little-endian PCM.
- Preserve original sample rate, original channel count, USER or REMOTE source
  identity, source/capture timestamp, normalized application timeline timestamp,
  and sequence/discontinuity information where applicable.
- Resampling and downmixing happen inside the platform backend. Shared
  conversation/STT code does not interpret native device formats.
- Never send raw frames, waveform buffers, or PCM samples through Tauri
  Rust-to-WebView IPC. High-frequency audio processing stays outside React.
- IPC carries only low-rate diagnostics: RMS/peak, capture/device/permission state,
  drops, discontinuities, drift, correlation, and timeline/health summaries.

## Windows

- REMOTE captures entire-system output, independently of meeting application.
  No Zoom-, Teams-, or WebEx-specific capture.
- Select the Default Communications render endpoint first; fall back to Default
  Multimedia. Do not assume Multimedia is the meeting application's endpoint.
- Record selected role, endpoint ID, readable name, and native format. Show the
  REMOTE endpoint name in the diagnostic UI. Log structurally when Communications
  and Multimedia endpoints differ.
- Distinguish `ACTIVE_AUDIO`, `IDLE_NO_RENDER_AUDIO`, and
  `ACTUAL_CAPTURE_FAILURE`. Absence of loopback packets alone is not failure,
  disconnect, or interruption.
- Implement and document a reliable health mechanism during output silence.
  Introduce a silent render keep-alive only if needed for reliable behavior.
- Physical acceptance requires a real Teams or Zoom call with headphones/headset:
  other participant speech reaches REMOTE, local microphone reaches USER, and
  the tapped endpoint is visible in logs/UI. Media playback alone is insufficient.

## macOS

- Minimum supported version: macOS 14.2.
- Use Core Audio process taps configured for system-wide output capture, targeting
  System Audio Recording permission instead of Screen Recording solely for audio.
- Do not default to ScreenCaptureKit for API/binding convenience. It is only a
  documented fallback for a concrete technical or distribution constraint.
  Stop and document that constraint before changing architecture.
- A narrow Swift or Objective-C bridge is permitted:
  native audio bridge → Rust MacAudioBackend → normalized audio contract.
  Session, STT, conversation, response, persistence, and UI logic stay outside it.
- Show microphone and system-audio recording permission states separately.
  Denial must not crash or silently fail; expose a clear recovery state.
- Report DEV MODE RESULT and SIGNED APP RESULT separately. Only a properly
  bundled, signed `.app` with a stable bundle identifier counts toward production
  permission acceptance; `tauri dev` does not.

## Headphone guard and bleed diagnostics

- V1 requires headphones/headset. Monitor output-route changes during capture.
- A route change likely to enable speakers, especially built-in speakers, emits
  `HeadphonesDisconnected` or an equivalent explicit safety event, sets diagnostic
  state to WARNING, and explains possible acoustic contamination of USER/REMOTE.
  Do not continue reporting healthy separation. Full AEC is outside M0.
- Measure rolling USER/REMOTE correlation or equivalent similarity over aligned
  windows, accounting for reasonable relative delay.
- This metric is observational, never an automatic single-threshold PASS/FAIL gate.
  Capture delay, resampling, mic processing, AGC, noise suppression, and acoustics
  affect it. Evaluate it with separate recordings, level behavior, listening, and
  timeline information. Include useful summary statistics in the test report.

## Soak and real-call acceptance

- Capture continuously for at least 30 minutes while a real speech recording loops
  through system output throughout the run. Silent capture is insufficient.
- Measure dropped frames, discontinuities, timeline drift, recovery events,
  memory growth, CPU utilization, and unexpected backend restarts.
- USER may be mostly silent during automation; that does not validate mic quality.
- Each platform requires a real meeting call before full acceptance. Confirm remote
  speech → REMOTE, local speech → USER, no source reversal, no significant remote
  contamination of USER with headphones, and stability during conversational pauses.

## Validation ownership and CI

- Agent-automatable: system-output speech playback, REMOTE meters, diagnostic file
  analysis, speech soak, timelines/drift/drops, memory/CPU sampling, unit/integration
  tests, and builds.
- Human-required: speaking and listening for mic quality, physical headset/device
  changes, OS permission denial/recovery, and real second-party calls.
- Unperformed human tests must read `PENDING HUMAN VALIDATION` with exact steps.
- Add a build-only Windows/macOS CI matrix that builds/tests shared Rust, compiles
  each native backend and platform integration, and builds the frontend. CI is not
  hardware validation; hardware tests remain in the acceptance checklist.
- Report platform statuses separately. M0 is fully accepted only after every
  required item actually completes on both platforms.
