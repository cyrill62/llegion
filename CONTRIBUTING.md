# Contributing

Thanks for your interest in llegion. Bug reports, ideas and pull requests are welcome.

## Reporting a bug

Open an issue with:

- your OS (and whether it's WSL), and the app version or commit
- your Ollama version, and whether the server is external, managed or remote
- what you did, what you expected, and what happened
- the relevant lines from the Logs tab, or the terminal output if the app crashed

**Remove API tokens** before pasting settings or logs. `settings.toml` contains them.

## Development setup

Install a stable Rust toolchain from [rustup.rs](https://rustup.rs). On Linux you also need the desktop libraries:

```sh
sudo apt-get install libxcb-render0-dev libxcb-shape0-dev libxcb-xfixes0-dev libxkbcommon-dev
```

Then:

```sh
cargo run
```

You can develop without a real Ollama by running the mock server used by the end-to-end test, then setting Settings → Connection to `http://127.0.0.1:11500`:

```sh
cargo run --example mock_ollama -- 11500
```

## Before opening a pull request

CI runs these on Linux, Windows and macOS, and all of them must pass:

```sh
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
```

If you touched the managed server, the process supervisor or the Ferron proxy, also run the end-to-end test (needs `ferron`; the mock server is built by `cargo test`):

```sh
cargo test e2e -- --ignored
```

## Guidelines

- Keep pull requests focused: one fix or feature per PR, with a short description of what changed and why. Screenshots help for UI changes.
- Never do blocking work (HTTP calls, process spawning, file tailing) on the UI thread. Use the poller or a one-shot task in `src/backend.rs`.
- Each tab lives in `src/views/<tab>.rs`. Reuse the widgets in `src/views/kit.rs` so the UI stays consistent.
- The app must build on Linux, Windows and macOS. Put platform-specific code behind `cfg`.
- Add unit tests next to the code (`#[cfg(test)] mod tests`), especially for parsers and for settings → environment mapping.
- Never log or print the settings file or tokens.

## Screenshots

The README pictures in `docs/screenshots/` come from a capture mode that exists only in debug builds. After a visible UI change, regenerate them:

```sh
scripts/screenshots.sh
```

```powershell
.\scripts\screenshots.ps1        # Windows
```

It starts the mock server, runs the app with `LLEGION_SCREENSHOTS=docs/screenshots` (default settings, never your own), fills the Chat and Performance tabs, saves one PNG per tab and quits. Set `LLEGION_URL` to capture against a real server instead. On WSLg the window is sometimes dropped at startup ("Connection reset by peer"); run it again, or use `screenshots.ps1` natively on Windows.

## License

By contributing, you agree that your contributions are licensed under the [MIT License](LICENSE).
