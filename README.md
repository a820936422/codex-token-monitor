# Work Token Monitor

Work Token Monitor is a local-first Tauri 2 desktop app for inspecting per-call token telemetry written by ChatGPT Work / Codex. It shows input, cached input, output, and cache-hit rate in real time. An app-managed local collector adds requested-versus-response model declaration auditing for ordinary API and official subscription calls.

## Features

- Live per-model-call token monitoring with 750 ms incremental polling
- Input, cached input, output, and cache-hit rate
- Per-call model declaration status: match, different, conflict, unknown, or not captured
- Automatic model collection on launch, a manual capture switch, and a saved auto-start preference
- Live collector memory/CPU readings and reversible Codex routing configuration
- Inspect requested/sent/returned models, evidence sources and upstream origin; filter by audit status
- Exact `response_id` association, including metadata that arrives after token telemetry
- Project filtering using the Git root when available, otherwise the session working directory
- Conversation filtering using the local conversation title and ID
- Sub-agent calls grouped under their parent Work conversation
- Date-range filtering and latest/oldest sorting
- Right-click a conversation title to copy its conversation ID
- Follow-latest mode for newly recorded calls
- Native Tauri desktop window; no browser required (model collection uses a loopback relay)

## Privacy model

The app has no analytics and does not upload monitored session history. Model collection starts by default and can be disabled in the UI. Its Python helper forwards Codex's normal requests through a loopback relay to the original HTTPS provider, retaining only model metadata. It does not make additional model requests.

The Rust backend reads local Codex data from:

- `~/.codex/sessions/**/*.jsonl`
- `~/.codex/archived_sessions/**/*.jsonl`
- `~/.codex/session_index.jsonl`
- `~/.codex/model-audit/capture-*.jsonl` (optional response-model metadata)

It parses JSONL records locally and retains only the metadata needed for the UI, such as token usage, timestamps, model/turn identifiers, conversation IDs, conversation titles, and working-directory/project metadata. It does not read `auth.json`, does not retain prompt/response bodies, and does not send session data over the network.

The collector controller edits only routing/transport fields in `config.toml`. It stores an auto-start preference and a recovery journal under `$CODEX_HOME/work-token-monitor/`, with private file permissions. The journal excludes credential settings. Normal app exit restores managed fields without overwriting changes made elsewhere and pauses collection; a forward-only helper remains available for Codex clients that cached the local URL.

Conversation titles, IDs, and local project paths are visible in the app UI. Treat screenshots or screen recordings of the app as potentially sensitive.

The only non-default Tauri capability granted to the frontend is clipboard **write-text**, used to copy IDs and setup commands. Clipboard read access is not granted.

## Requirements

- Node.js 20+
- Rust 1.89+ toolchain
- Tauri 2 Linux build dependencies for your distribution
- Python 3.11+ and a configured Codex client for model metadata collection

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

Start Work Token Monitor and use the **模型采集** switch. **随程序启动自动采集** is enabled by default and saved across launches. The application embeds its helper scripts, starts a private relay, and updates the active provider in the user's `config.toml`; the release executable does not need this source checkout.

**Restart Codex after the first connection** so it loads the local route. Existing requests and historical token logs cannot be captured retroactively. Model observations appear after new requests pass through the relay.

Turning the capture switch off pauses parsing/writing while preserving forwarding. Uncheck auto-start to keep capture off on the next launch. On app exit, configuration is restored and capture stops; after restarting Codex, use **恢复直连配置 → 已重启 Codex，停止转发** in the collector details to fully stop the helper. Its live CPU/RSS readings describe the helper, not the whole desktop application.

Automatic integration currently supports the main user config on Linux/Unix. Default profiles, project/managed route overrides, and provider settings rewritten by another application need explicit setup; see the guide below. Python startup or configuration failures appear in the panel instead of claiming capture is active.

The separate CLI workflow is also available:

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
