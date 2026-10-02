# Unmute

macOS desktop app for meeting audio. Native code captures system output (REMOTE) and the microphone (USER). The window receives transcripts and diagnostic state only.

## Requirements

- Node.js 22.12+
- Stable Rust
- macOS 14.2+
- [Tauri prerequisites](https://v2.tauri.app/start/prerequisites/)

## Run

```sh
npm ci
npm run tauri -- dev
```

`npm run dev` opens a browser preview and cannot capture audio.

## Test

```sh
npm run build
cargo test --locked --manifest-path src-tauri/Cargo.toml
cargo test --locked --manifest-path src-tauri/Cargo.toml --no-default-features
python3 scripts/test-analysis.py
```

Headless diagnostics, without React or Tauri:

```sh
cargo build --locked --release --manifest-path src-tauri/Cargo.toml --no-default-features --bin audio-diag
src-tauri/target/release/audio-diag preflight
```

## Documents

Local notes are listed in [docs/README.md](docs/README.md).

- [M0 requirements](docs/M0-REQUIREMENTS.md)
- [macOS runbook](docs/MACOS-M0.md) — permission setup, soak, and validation. This takes precedence on macOS.
- [Acceptance checklist](docs/M0-ACCEPTANCE.md)

## Meeting controls

The menu-bar icon provides audio permission setup, transparency presets, window controls,
listening controls, and **Suggest Now**. **Settings…** opens the fine-grained transparency slider.
macOS still requires its own permission confirmation; development builds may require approval again.

**Pause listening** stops capture, keeps the recognizer/model available, and requests a response
from the latest recognized text. **Resume listening** continues the same meeting.
**Suggest now** requests a response without waiting for the automatic quiet-period trigger.

Remote speech appears on the left; response alternatives accumulate on the right.
**Mark as said** is a manual annotation, not proof that microphone speech was recognized.
Both streams of displayed text remain scrollable; new messages do not force scrolling while
you are reading older history.

Text and said marks are saved locally in the WebView's application storage and restored on
relaunch. **New meeting…** archives the current history; **Saved meetings** restores it.
Audio is not recorded by the product. Storage quota failures are shown rather than silently
discarding history; clearing app website data removes these local records.

Regression checks: `npm run test:meeting`. Real local pause/replay smoke check:
`python3 scripts/test-local-copilot.py --helper /path/to/local-copilot --fixture fixtures/stt/c-long-turn.wav --pause --output reports/local-pause-resume.jsonl`.
