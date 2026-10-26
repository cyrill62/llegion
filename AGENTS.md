# CLAUDE.md

llegion: a cross-platform control panel for Ollama, in Rust (edition 2024), with two front-ends over one core: the eframe/egui desktop app (feature `gui`, default, binary `llegion`) and a ratatui terminal UI (feature `tui`, default, binary `llegion-tui`). See README.md for user-facing features.

## Commands

```sh
cargo run                                   # debug build + launch the desktop app
cargo run --bin llegion-tui                 # launch the terminal UI
cargo fmt --check                           # CI gate
cargo clippy --all-targets --all-features -- -D warnings                          # CI gate: warnings are errors
cargo clippy --all-targets --no-default-features --features tui,rustls,sqlite -- -D warnings  # CI gate: TUI without egui
cargo test --all-features                   # unit tests
cargo test e2e -- --ignored                 # end-to-end: needs ferron (or FERRON=/path)
cargo test detect_and_sample -- --nocapture # real GPU probe on this machine
cargo check --no-default-features --features sqlite  # no rustls; a chat storage feature is always required
./scripts/screenshots.sh                    # regenerate docs/screenshots/*.png (debug-only capture mode, mock server; screenshots.ps1 on Windows)
./scripts/build.sh [targets]                # release archives into dist/ (build.ps1 on Windows)
```

Run fmt, clippy and tests before considering a change done; CI runs them on Linux, Windows and macOS.

## Architecture

- `lib.rs`: module tree; the core is always built, `app`/`views`/`icons`/`screenshots` only with `gui`, `tui` only with `tui`.
- `main.rs` (bin `llegion`): eframe entry point; creates `app::App`. `bin/llegion-tui.rs`: calls `tui::run()`.
- `control.rs`: `Control`, the UI-independent core both front-ends own: settings in effect, `Backend`, `LogBuffer`, the `ollama serve` / `ferron` supervisors, autostart, crash restarts, applying settings. Front-ends call `tick()` every frame and pass a `util::Repaint` callback so background threads can request a redraw.
- `app.rs`: `App` state (wraps `ctl: Control` plus the settings draft and per-tab UI state), `Tab` enum, top-level frame. Each tab lives in `views/<tab>.rs` as additional `impl App` methods; shared widgets (panels, stat tiles, badges, form rows) are in `views/kit.rs`.
- `tui/`: the terminal UI. `tui/mod.rs` has `Tui` (event loop, top bar, tabs, footer hints, toasts, line editing); each tab is `tui/<tab>.rs` with `draw_*`, `*_key` and `*_hints` methods on `Tui`; `tui/kit.rs` has shared widgets, `tui/input.rs` the line editor.
- `backend.rs`: background poller thread plus one-shot `std::thread::spawn` tasks (pull, load, benchmark...). Results go into `Arc<Mutex<Shared>>` that the UI reads each frame. Never do blocking I/O on the UI thread.
- `ollama.rs`: minimal blocking REST client (ureq) for `/api/*`.
- `config.rs`: settings (TOML in the platform config dir, owner-only perms on Unix) and settings → environment variables for `ollama serve` (GPU selection, proxy forces loopback, etc.).
- `process.rs`: supervisor for child processes (`ollama serve`, `ferron`) with log capture and restart.
- `ferron.rs`: Ferron config (`ferron.kdl`) generation (TLS modes, bearer-token auth, CORS) and the self-signed certificate.
- `gpu.rs`: GPU detection/sampling (nvidia-smi, /sys/class/drm + lspci, Windows registry, system_profiler).
- `logs.rs`: log sources (process pipes, file tailing, journalctl).
- `icons.rs`: Material icon codepoints as macros (so they can be `concat!`ed into labels); declared with `#[macro_use]` first in `lib.rs`.
- `screenshots.rs`: `#[cfg(debug_assertions)]` capture mode (`LLEGION_SCREENSHOTS=<dir>`), stepping through every tab; never compiled into release builds.
- `e2e.rs`: `#[cfg(test)]`, `#[ignore]`d end-to-end test using the mock server `examples/mock_ollama.rs` and real Ferron.
- `examples/mock_ollama.rs`: Ollama REST stand-in (std + serde_json) for the e2e test and the screenshot scripts.

## Conventions

- Unit tests sit in `#[cfg(test)] mod tests` at the bottom of each module; GPU parsers are tested against captured sample output.
- Modules start with a `//!` doc comment describing their role.
- Keep platform-specific code behind `cfg` and make sure all three OSes still compile.
- Logic both front-ends need (formatting, filters, summaries) goes in the core modules, not in `views/` or `tui/`.
- The TUI must not print to stdout/stderr while running: everything goes through `LogBuffer`.
- The settings file contains API tokens: never log or print it.

## Environment notes

- On WSL, `libEGL`/`MESA ZINK` warnings at startup are graphics-stack noise, not app errors. WSLg may also drop eframe windows; running natively on Windows avoids it.
