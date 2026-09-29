# M0 acceptance evidence

Implementation state: the Tauri/React shell, normalized frame contract and its
unit test, and build-only CI workflow exist. Native audio backends and capture
test tooling are not implemented. `NOT RUN` is not PASS. No platform is accepted.

| Required item | Owner | Windows | macOS |
| --- | --- | --- | --- |
| Platform build and frontend build | Agent | NOT RUN | PASS: debug shell only; native backend pending |
| Shared Rust automated tests | Agent | NOT RUN | PASS: initial contract test only |
| CI native backend/integration compilation | Agent | NOT RUN | NOT RUN |
| System-output speech playback and REMOTE meters | Agent | NOT RUN | NOT RUN |
| Output silence health and resumed capture | Agent | NOT RUN | NOT RUN |
| Speech playback soak ≥30 minutes | Agent | NOT RUN | NOT RUN |
| Timeline, drift, drops, discontinuities | Agent | NOT RUN | NOT RUN |
| Bleed diagnostic analysis and summaries | Agent + human listening | NOT RUN | NOT RUN |
| Real meeting call | Human | PENDING HUMAN VALIDATION | PENDING HUMAN VALIDATION |
| Microphone speech quality | Human | PENDING HUMAN VALIDATION | PENDING HUMAN VALIDATION |
| Headset disconnect/reconnect and warning | Human | PENDING HUMAN VALIDATION | PENDING HUMAN VALIDATION |
| Permission denial/recovery | Human | PENDING HUMAN VALIDATION | PENDING HUMAN VALIDATION |
| DEV MODE RESULT: permissions | Human | N/A | PENDING HUMAN VALIDATION |
| SIGNED APP RESULT: permissions | Human | N/A | PENDING HUMAN VALIDATION |

## Evidence to attach to each run

Local foundation checks (2026-09-29, macOS arm64): `npm run build` passed;
`cargo test --locked --manifest-path src-tauri/Cargo.toml` passed (one frame
contract test). This does not test resampling, capture, or any hardware path.
The CI workflow has been added locally but has not run on GitHub.
`npm run tauri -- build --debug --no-bundle` also passed, producing
`src-tauri/target/debug/unmute`. This is an unbundled debug executable, not
signed-app permission evidence or a completed native audio backend build.

Record commit/build identity, OS/version, start/end times, app/bundle identity and
signing details where applicable, device/route names, native formats, normalized
format, permission states, logs, and separate USER/REMOTE diagnostic recordings.
Windows additionally records endpoint role/ID and whether default roles differ.

Document the implemented Windows silence-health mechanism and its evidence;
none has been selected or tested yet. If a keep-alive is needed, record why.

For each soak, record speech asset and looping method, capture duration, dropped
frames, discontinuities, drift with units and measurement method, recovery events,
unexpected backend restarts, CPU samples, and baseline/peak/final memory usage.
Attach the samples and explain any gaps in playback or capture. Do not infer
microphone quality from a silent USER track.

For bleed analysis, record window duration, alignment/delay search range, usable
window count, correlation summary statistics and observed delays, alongside level
and listening notes. Do not derive acceptance from one correlation threshold.

## Exact human validation steps

Perform these on each platform after a runnable build and diagnostic recording
path exist. Record observed results and evidence before changing any status.

1. Connect and wear headphones/headset. Start capture and note USER input and
   REMOTE output names. On Windows, verify endpoint role/ID in logs and the
   readable tapped endpoint name in the UI.
2. Speak a known phrase into the microphone while the remote participant is silent.
   Save and listen to the USER recording; document intelligibility, clipping,
   dropouts, and whether source labeling is correct.
3. Join a real second-party meeting (Teams or Zoom on Windows). Have the other
   participant speak alone, then speak locally alone, then alternate with ordinary
   pauses. Listen to separate USER and REMOTE recordings. Confirm remote speech
   reaches REMOTE, local speech reaches USER, no reversal, no significant remote
   bleed into USER with headphones, and capture stability through pauses.
4. During capture, disconnect the headset or explicitly switch the output route
   to built-in speakers. Verify an explicit safety event, WARNING, and the UI
   explanation of possible acoustic bleed. Reconnect/select the headset, verify
   the actual route and continuing capture, and record recovery/discontinuities.
5. Through OS settings, deny microphone access, launch/relaunch as required, and
   attempt capture. Verify a clear denial/recovery state without a crash or silent
   failure. Restore access and verify capture recovers; record any restart needed.
6. On macOS, separately deny and restore system-audio recording access through
   OS settings. Verify its state is distinct from microphone permission and that
   failure/recovery is clearly shown. Record whether Screen Recording was required.
7. On macOS, record development-mode permission observations as DEV MODE RESULT.
   Repeat permission tests using a properly bundled and signed `.app` with a
   stable bundle identifier; record identity/signature and results as SIGNED APP
   RESULT. Only the latter counts toward final permission acceptance.

## Completion rule

Publish the table with evidence for both platforms. Leave unperformed human
items as PENDING HUMAN VALIDATION and automated items as NOT RUN. Neither local
playback, CI, development permissions, nor one platform's success establishes
complete cross-platform M0 acceptance.
