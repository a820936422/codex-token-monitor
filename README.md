# Work Token Monitor

Work Token Monitor is a local-first Tauri 2 desktop app for inspecting per-call token telemetry written by ChatGPT Work / Codex. It shows input, cached input, output, and cache-hit rate in real time without requiring an API key or a local web server.

## Features

- Live per-model-call token monitoring with 750 ms incremental polling
- Input, cached input, output, and cache-hit rate
- Project filtering using the Git root when available, otherwise the session working directory
- Conversation filtering using the local conversation title and ID
- Sub-agent calls grouped under their parent Work conversation
- Date-range filtering and latest/oldest sorting
- Right-click a conversation title to copy its conversation ID
- Follow-latest mode for newly recorded calls
- Native Tauri desktop window; no browser or localhost server required

## Privacy model

The app is local-only. It does not contain analytics, telemetry, remote APIs, or upload code.

The Rust backend reads local Codex data from:

- `~/.codex/sessions/**/*.jsonl`
- `~/.codex/archived_sessions/**/*.jsonl`
- `~/.codex/session_index.jsonl`

It parses JSONL records locally and retains only the metadata needed for the UI, such as token usage, timestamps, model/turn identifiers, conversation IDs, conversation titles, and working-directory/project metadata. It does not read `auth.json`, does not retain prompt/response bodies, and does not send session data over the network.

Conversation titles, IDs, and local project paths are visible in the app UI. Treat screenshots or screen recordings of the app as potentially sensitive.

The only non-default Tauri capability granted to the frontend is clipboard **write-text**, used by the "Copy conversation ID" action. Clipboard read access is not granted.

## Requirements

- Node.js 20+
- Rust toolchain
- Tauri 2 Linux build dependencies for your distribution

The current implementation is primarily developed and tested on Linux/KDE Wayland.

## Development

```bash
npm ci
npm run tauri:dev
```

## Tests and build

```bash
npm run build
cd src-tauri
cargo test
cd ..
npm run tauri:build
```

Bundling is currently disabled, so the Linux release executable is produced at:

```text
src-tauri/target/release/work-token-monitor
```

## Configuration

By default the application reads from `~/.codex`. The monitor also supports these environment overrides:

- `CODEX_HOME` — alternate Codex data directory
- `CODEX_SESSION_ROOT` — alternate session root(s)
- `CODEX_SESSION_INDEX` — alternate session index path

## Security

Please do not include real conversation logs, session IDs, local paths, tokens, or credentials in public bug reports. See [SECURITY.md](SECURITY.md) for vulnerability reporting guidance.

## License

MIT. See [LICENSE](LICENSE).
