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
