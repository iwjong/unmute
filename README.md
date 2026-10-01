# Unmute

## Local assistant prototype

The app now defaults to keyless, on-device English transcription and response suggestions. Requires macOS 26+ on Apple Silicon; Apple Intelligence is not required. The app bundles its response runtime and downloads the 1.84 GB Qwen model on first use. [Run instructions, limits and validation](docs/local-copilot.md). The previous mock UX is still available from Meeting options.

macOS Meeting Copilot with a separate engineering diagnostics window.

The desktop app opens a minimal, always-on-top overlay with recent conversation
and a larger bottom-pinned spoken suggestion. Persistent headings and status labels
are removed; hover reveals controls. **Settings → Background transparency** adjusts the background
without fading text; **Text only** makes it fully transparent. [Overlay details](docs/minimal-overlay.md). Press **듣기 시작**
for real on-device English transcription and suggestions. **Settings → More → Preview** retains the deterministic REMOTE/YOU mock experience. The top bar supports dragging, collapse/expand, and closing. The
options menu opens **Engineering diagnostics** in a separate native window.

```sh
npm run test:meeting
PATH="$HOME/.cargo/bin:$PATH" npm run tauri -- build --debug --bundles app
open src-tauri/target/debug/bundle/macos/Unmute.app
```

Mock events and states live in `src/ui/meeting/mock.ts`; the product view lives
in `src/ui/meeting/`, and the preserved audio UI lives in `src/ui/diagnostics/`.
Collapse retains the current meeting state and restores the previous expanded size.
The live view shows at most six recent turns; earlier fixture turns remain in
session state. Human assessment of readability and placement remains
`PENDING_HUMAN_VISUAL_VALIDATION`.

Earlier UX-shell checks: frontend and bundled macOS debug application build successfully;
the app was launched as a native desktop window. Floating level 5, 410×660
expanded / 410×52 collapsed dimensions, state-preserving expansion, all five
SAY THIS states, partial/final transcripts, six-turn context, pinned suggestions
while scrolling, separate diagnostics, and normal close were observed. Existing
Rust tests pass with and without desktop features (8 each); analysis and mock
checks pass. Screenshots are in `artifacts/meeting-ux/`.
Native drag and edge-resize remain pending manual confirmation: the automation
tool's synthetic drags produced no observable movement. Resize/drag are configured,
but those interactions are not claimed as verified. No real-meeting or speaker-
bleed acceptance is inferred from these UX checks.

The macOS backend uses Core Audio process taps for system-wide REMOTE capture
and Core Audio input callbacks for USER. Both the Tauri/React diagnostic app and
the headless `audio-diag` CLI use the same Rust backend. Audio remains native;
only text events and diagnostic state cross WebView IPC.

Speakers are supported under the user's updated requirement. USER can contain
remote speech through acoustic bleed; correlation is observational and there is
no AEC or automatic source-isolation acceptance gate.

## Development

Requires Node.js 22.12+, stable Rust and the
[Tauri platform prerequisites](https://v2.tauri.app/start/prerequisites/).
macOS minimum: 14.2. On this machine, load Rust with `source "$HOME/.cargo/env"`.

```sh
npm ci
npm run tauri -- dev
```

The app checks permission preconditions before capture. Explicitly enable
recording to preserve separate USER/REMOTE WAVs. Browser-only `npm run dev` cannot
capture audio.

```sh
npm run build
cargo test --locked --manifest-path src-tauri/Cargo.toml
cargo test --locked --manifest-path src-tauri/Cargo.toml --no-default-features
python3 scripts/test-analysis.py
```

Headless build, independent of React, WebView and Tauri lifecycle:

```sh
cargo build --locked --release --manifest-path src-tauri/Cargo.toml --no-default-features --bin audio-diag
src-tauri/target/release/audio-diag preflight
```

See [MACOS-M0.md](docs/MACOS-M0.md) for permission setup, speech fixture generation,
headless soak, telemetry, recordings, drift analysis, and signed-app validation.

## Status

Implementation and deterministic tests do not establish live-audio acceptance.
See [M0-ACCEPTANCE.md](docs/M0-ACCEPTANCE.md) and [the macOS evidence report](reports/macos-m0.md).
A 30-minute development speech soak completed: no runtime drops/discontinuities,
+7.322 ms relative drift, and observable speaker bleed in USER. Real meeting and
signed-app permission acceptance remain pending.
macOS M0 is accepted for development progression. Development now prioritizes
macOS; Windows is deferred at the user's request. A Windows WASAPI/CLI draft
passes cross-target `cargo check`, but has not been linked, run on Windows,
validated with live speech, or subjected to a 30-minute soak. The CI matrix is
build-only and has not run remotely.

Local working documents are listed in [docs/README.md](docs/README.md).
[M0-REQUIREMENTS.md](docs/M0-REQUIREMENTS.md) records the original platform requirements;
[MACOS-M0.md](docs/MACOS-M0.md) takes precedence for macOS execution and speaker support.
