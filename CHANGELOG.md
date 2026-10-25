# Changelog

Notable changes to Llegion are listed here. `scripts/bump-version.sh` moves the
entries under **Unreleased** into the new version section when a version is bumped.

## [Unreleased]


## [0.2.0]

### Added

- Save benchmark results to `benchmarks.csv` and reload them at startup, on refresh, and after a benchmark completes.
- Browse saved chats in tabs, with per-chat model settings, rename, delete, and collapsible groups.
- Show Ollama agent launch commands for Claude Code, Codex, OpenCode, Pi, OMP, and other integrations.
- Open the configuration and data folders from Application settings.

### Changed

- Use bundled SQLite for chat history by default, with RocksDB available as an opt-in Cargo feature.
- Warn users on first entry to Chat that saved conversations are unencrypted.
- Keep existing RocksDB chat data when SQLite is selected, so it remains available with the RocksDB feature.
