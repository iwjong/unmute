# Unmute

M0 desktop audio diagnostics. The repository root is `/Users/iwjong/unmute`;
there is no nested project directory.

## Development

Install Node.js 22.12+ and stable Rust, plus the
[Tauri platform prerequisites](https://v2.tauri.app/start/prerequisites/).
On this development machine Rust was installed in `~/.cargo/bin`; load it with
`source "$HOME/.cargo/env"` if it is not on your PATH.

```sh
npm ci
npm run tauri -- dev
```

`npm run dev` opens a browser-only preview; it cannot capture audio.

```sh
npm run build
cargo test --locked --manifest-path src-tauri/Cargo.toml
npm run tauri -- build --no-bundle
```

## Current implementation

The initial implementation contains a Tauri/React diagnostic shell, a typed
normalized PCM16 LE frame contract with source/timing metadata and validation,
and a build-only Windows/macOS CI matrix. IPC exposes diagnostic state only;
normalized frames deliberately do not implement serialization.

Native capture, conversion, permissions queries, recording, meters, device
monitoring, correlation, and soak tooling are **not implemented yet**. The UI
reports this state explicitly. The CI workflow currently validates the shell and
shared contract; it cannot validate native backends until they are implemented.

The macOS target is configured for 14.2 with stable bundle identifier
`com.iwjong.unmute` and separate microphone/audio-capture usage descriptions.
This configuration alone does not implement or validate permissions.

Native implementation must follow [M0-REQUIREMENTS.md](M0-REQUIREMENTS.md).
macOS uses [Core Audio process taps](https://developer.apple.com/documentation/coreaudio/capturing-system-audio-with-core-audio-taps),
with an isolated native bridge if needed; no ScreenCaptureKit fallback has been
introduced. Windows requires WASAPI system-output loopback with Communications
endpoint preference and explicit idle/failure distinction.

See [M0-ACCEPTANCE.md](M0-ACCEPTANCE.md) for separate platform evidence and exact
human validation steps. Neither platform has completed M0 acceptance.
