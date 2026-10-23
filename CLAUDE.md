# CLAUDE.md

Ollama Control (`ollama-egui`): a cross-platform desktop control panel for Ollama, in Rust (edition 2024) with eframe/egui. See README.md for user-facing features.

## Commands

```sh
cargo run                                   # debug build + launch
cargo fmt --check                           # CI gate
cargo clippy --all-targets -- -D warnings   # CI gate: warnings are errors
cargo test                                  # unit tests
cargo test e2e -- --ignored                 # end-to-end: needs python3 + caddy (or CADDY=/path)
cargo test detect_and_sample -- --nocapture # real GPU probe on this machine
cargo check --no-default-features           # no rustls, for cross-target checks without a C toolchain
./scripts/screenshots.sh                    # regenerate docs/screenshots/*.png (debug-only capture mode, mock server)
./scripts/build.sh [targets]                # release archives into dist/ (build.ps1 on Windows)
```

Run fmt, clippy and tests before considering a change done; CI runs them on Linux, Windows and macOS.

## Architecture

- `main.rs`: eframe entry point; creates `app::App`.
- `app.rs`: `App` state, `Tab` enum, top-level frame. Each tab lives in `views/<tab>.rs` as additional `impl App` methods; shared widgets (panels, stat tiles, badges, form rows) are in `views/kit.rs`.
- `backend.rs`: background poller thread plus one-shot `std::thread::spawn` tasks (pull, load, benchmark...). Results go into `Arc<Mutex<Shared>>` that the UI reads each frame. Never do blocking I/O on the UI thread.
- `ollama.rs`: minimal blocking REST client (ureq) for `/api/*`.
- `config.rs`: settings (TOML in the platform config dir, owner-only perms on Unix) and settings → environment variables for `ollama serve` (GPU selection, proxy forces loopback, etc.).
- `process.rs`: supervisor for child processes (`ollama serve`, `caddy run`) with log capture and restart.
- `caddy.rs`: Caddyfile generation (TLS modes, bearer-token auth, CORS).
- `gpu.rs`: GPU detection/sampling (nvidia-smi, /sys/class/drm + lspci, Windows registry, system_profiler).
- `logs.rs`: log sources (process pipes, file tailing, journalctl).
- `icons.rs`: Material icon codepoints as macros (so they can be `concat!`ed into labels); declared with `#[macro_use]` first in `main.rs`.
- `screenshots.rs`: `#[cfg(debug_assertions)]` capture mode (`OLLAMA_EGUI_SCREENSHOTS=<dir>`), stepping through every tab; never compiled into release builds.
- `e2e.rs`: `#[cfg(test)]`, `#[ignore]`d end-to-end test using `tests/fixtures/mock_ollama.py` and real Caddy.

## Conventions

- Unit tests sit in `#[cfg(test)] mod tests` at the bottom of each module; GPU parsers are tested against captured sample output.
- Modules start with a `//!` doc comment describing their role.
- Keep platform-specific code behind `cfg` and make sure all three OSes still compile.
- The settings file contains API tokens: never log or print it.

## Environment notes

- On WSL, `libEGL`/`MESA ZINK` warnings at startup are graphics-stack noise, not app errors. WSLg may also drop eframe windows; running natively on Windows avoids it.
