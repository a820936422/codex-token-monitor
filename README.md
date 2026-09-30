# Work Token Monitor

Work Token Monitor is a local-first Tauri 2 desktop app for inspecting per-call token telemetry written by ChatGPT Work / Codex. It shows input, cached input, output, and cache-hit rate in real time without requiring an API key or intercepting model requests.

## Features

- Live per-model-call token monitoring with 750 ms incremental polling
- Input, cached input, output, and cache-hit rate
- Project filtering using the Git root when available, otherwise the session working directory
- Conversation filtering using the local conversation title and ID
- Sub-agent calls grouped under their parent Work conversation
- Date-range filtering and latest/oldest sorting
- Right-click a conversation title to copy its conversation ID
- Follow-latest mode for newly recorded calls
- Persistent, keyboard-accessible resizable columns
- Filtered total-token summary and logged reasoning effort
- Native Tauri desktop window; no browser or localhost server required in a release build

## Privacy model

The app is local-only. It does not contain analytics, outbound telemetry, remote API calls, or upload code.

The Rust backend reads local Codex data from:

- `~/.codex/sessions/**/*.jsonl`
- `~/.codex/archived_sessions/**/*.jsonl`
- `~/.codex/session_index.jsonl`

It parses JSONL records locally and retains only the metadata needed for the UI, such as token usage, timestamps, model/turn identifiers, conversation IDs, conversation titles, and working-directory/project metadata. Session files may contain conversation bodies; the parser reads JSONL locally but retains only the listed metadata, not conversation content, in its data model. It does not read `auth.json` or `config.toml`, modify Codex configuration, start a relay, or make model requests. No session data is sent over the network. UI preferences remain in local webview storage.

Conversation titles, IDs, and local project paths are visible in the app UI. Treat screenshots or screen recordings of the app as potentially sensitive.

The only non-default Tauri capability granted to the frontend is clipboard **write-text**, used by the "Copy conversation ID" action. Clipboard read access is not granted.

Model names are read from Codex log context; they are not independent verification of the backend model. This application does not detect account restrictions, model substitutions, or model quality.

For upgrades from the removed collection feature, read [the migration note](docs/upgrade-token-only.md).

## Requirements

- Node.js 20.19+ within the 20.x line, or Node.js 22.12+ (`^20.19.0 || >=22.12.0`)
- Rust 1.89+ toolchain
- Tauri 2 Linux build dependencies for your distribution

The current implementation is primarily developed and tested on Linux/KDE Wayland.

## Development

```bash
npm install
npm run tauri:dev
```

## Tests and build

```bash
npx playwright install chromium --only-shell
npm test
npm run build
npm run test:rust
cargo fmt --manifest-path src-tauri/Cargo.toml --check
cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets -- -D warnings
npm run tauri:build
```

Rust tests use temporary synthetic logs. Browser tests run the actual React UI with mocked Tauri IPC, not a native webview or a live account. No test makes a model request. Playwright and its browser are development-only dependencies.

Bundling is currently disabled, so the Linux release executable is produced at:

```text
src-tauri/target/release/work-token-monitor
```

## Configuration

By default the application reads from `~/.codex`. The monitor also supports these environment overrides:

- `CODEX_HOME` — alternate Codex data directory
- `CODEX_SESSION_ROOT` — alternate session root(s)
- `CODEX_SESSION_INDEX` — alternate session index path

## Supported log format and current limits

Per-call usage comes from `token_usage_record`; model and effort come from local `turn_context`. Legacy cumulative `event_msg.token_count` entries are not treated as individual calls. A log without supported per-call telemetry may therefore show no calls.

Snapshots retain at most 50,000 calls. Date filters summarize the records currently loaded, not a guaranteed full-history total. The UI and deduplication cache still need a unified retention policy for long-running sessions. Files are polled every 750 ms; very large log collections may increase scan latency.

See [the token-only review and development priorities](docs/project-review-2026-09-23.md) for the removal scope, verified checks, and remaining limitations.

## Security

Please do not include real conversation logs, session IDs, local paths, tokens, or credentials in public bug reports. See [SECURITY.md](SECURITY.md) for vulnerability reporting guidance.

## License

MIT. See [LICENSE](LICENSE).
