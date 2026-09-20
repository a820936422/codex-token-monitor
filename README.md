# Work Token Monitor

Work Token Monitor is a local-first Tauri 2 desktop app for inspecting per-call token telemetry written by ChatGPT Work / Codex. It shows input, cached input, output, and cache-hit rate in real time. An optional local collector adds requested-versus-response model declaration auditing for ordinary API and official subscription calls.

## Features

- Live per-model-call token monitoring with 750 ms incremental polling
- Input, cached input, output, and cache-hit rate
- Per-call model declaration status: match, different, conflict, unknown, or not captured
- Inspect requested/sent/returned models, evidence sources and upstream origin; filter by audit status
- Exact `response_id` association, including metadata that arrives after token telemetry
- Project filtering using the Git root when available, otherwise the session working directory
- Conversation filtering using the local conversation title and ID
- Sub-agent calls grouped under their parent Work conversation
- Date-range filtering and latest/oldest sorting
- Right-click a conversation title to copy its conversation ID
- Follow-latest mode for newly recorded calls
- Native Tauri desktop window; no browser or localhost server required

## Privacy model

The desktop app is local-only. It does not contain analytics or upload monitored session data. The optional Python collector explicitly forwards the requests of a client you configure through it; it is separate from the desktop monitor.

The Rust backend reads local Codex data from:

- `~/.codex/sessions/**/*.jsonl`
- `~/.codex/archived_sessions/**/*.jsonl`
- `~/.codex/session_index.jsonl`
- `~/.codex/model-audit/capture-*.jsonl` (optional response-model metadata)

It parses JSONL records locally and retains only the metadata needed for the UI, such as token usage, timestamps, model/turn identifiers, conversation IDs, conversation titles, and working-directory/project metadata. It does not read `auth.json`, does not retain prompt/response bodies, and does not send session data over the network.

Conversation titles, IDs, and local project paths are visible in the app UI. Treat screenshots or screen recordings of the app as potentially sensitive.

The only non-default Tauri capability granted to the frontend is clipboard **write-text**, used to copy IDs and setup commands. Clipboard read access is not granted.

## Requirements

- Node.js 20+
- Rust toolchain
- Tauri 2 Linux build dependencies for your distribution
- Python 3.11+ and Codex for optional model metadata collection

The current implementation is primarily developed and tested on Linux/KDE Wayland.

## Development

```bash
npm install
npm run tauri:dev
```

## Tests and build

```bash
python3 -m unittest discover -s scripts -p 'test_*.py' -v
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
- `CODEX_MODEL_AUDIT_DIR` — alternate model metadata directory (set for both monitor and collector)

## Model declaration auditing

For a normal Codex session using your configured custom Responses provider:

```bash
python3 scripts/model_audit.py run --mode configured
```

For an official subscription, sign in with the official Codex client first, then run:

```bash
python3 scripts/model_audit.py run --mode official
```

These commands start ordinary Codex sessions with process-local routing overrides. They do not change your saved configuration or attach to an already-running desktop conversation. The collector observes normal requests without making extra model calls; the UI gains model evidence after joining each response ID. Existing records without evidence remain uncaptured.

See [setup, desktop integration, evidence rules and limits / 配置与使用说明](docs/model-audit.md). Model declarations describe what your immediate provider returns. A relay can rewrite them; matching identifiers do not independently authenticate the backend model.

The separate [isolated diagnostic](docs/model-verification-probe.md) remains available with `python3 scripts/model_probe.py --output .model-probes/latest.json`. That diagnostic makes an extra real model call and consumes normal usage.

## Security

Please do not include real conversation logs, session IDs, local paths, tokens, or credentials in public bug reports. See [SECURITY.md](SECURITY.md) for vulnerability reporting guidance.

## License

MIT. See [LICENSE](LICENSE).
