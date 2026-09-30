# Work Token Monitor

A local-first Tauri 2 desktop application for inspecting per-call token telemetry written by ChatGPT Work / Codex. Version 0.4 focuses on a well-defined recent-record window, bounded indexing, useful diagnostics, and explicit local exports. It does not intercept model requests or detect account restrictions, model identity, or model quality.

## What the numbers mean

The monitor retains the **latest 50,000 observed calls by UTC timestamp and stable ID**, independent of file traversal order. The limit is configurable. The table, summaries, group views, and exports use the same retained window; the frontend applies additions and evictions in one revisioned batch. Missing batches trigger a new authoritative snapshot.

The coverage notice displays the loaded count, retention limit, oldest/newest timestamps, indexing progress, and truncation. Date filters cover **currently loaded records**, not an unlimited history or billing statement. Reopening the application rescans available logs; previously observed records whose source files were deleted are not guaranteed to reappear. No private persistent history database is created.

Unknown or invalid counters are shown as `—`, not zero. A `*` next to a subtotal means some records lack that measurement. Cache hit is input-weighted over records with both valid input and cached-input counters; zero total input has no percentage. Aggregate integer arithmetic uses `BigInt`; JSON aggregate totals are decimal strings to preserve precision.

## Features

- Real-time local monitoring with file notifications and polling fallback
- Incremental directory discovery, bounded reads and bounded record/ID storage
- Input, cached input, output, total-token summaries and logged reasoning effort
- Project/conversation filtering and grouped summaries over the same filtered calls
- Today, last 7/30 local-calendar days, and custom inclusive date ranges
- Virtualized tables, searchable project/conversation pickers and adaptive compact columns
- Persistent filters, sort, follow-latest preference and accessible resizable columns
- Heartbeat age, indexing, unreadable inputs, malformed records and compatibility diagnostics
- User-initiated CSV/JSON exports through a native save dialog, with identifiers redacted by default
- Conversation ID copy with clipboard write-only access

## Privacy and exports

The backend reads `sessions/**/*.jsonl`, `archived_sessions/**/*.jsonl`, and `session_index.jsonl` under `~/.codex` by default. Session files can contain conversation bodies: the parser reads JSONL locally but retains only bounded telemetry/context metadata, never prompt or response bodies in the application data model. It does not read `auth.json` or `config.toml`, modify Codex configuration, start a relay, or make model requests.

Local UI preferences live in webview storage. Screenshots can reveal titles, IDs, and paths. The export menu replaces project/conversation names and removes raw IDs/paths by default; opt in explicitly to include them. Redaction is **not anonymity**: timestamps, model names, and usage remain visible. CSV string cells are escaped against spreadsheet-formula interpretation. Exports include their scope and are limited to 64 MiB. On Unix the backend writes a private temporary file and atomically replaces only the destination selected in the native dialog.

No network upload or analytics is implemented. The frontend receives no filesystem path-writing permission; `save_export` receives content and a format, then the backend asks the user where to save it. The only extra frontend plugin permission remains clipboard **write-text**, not clipboard read. See [SECURITY.md](SECURITY.md).

## Requirements and development

The tested toolchain is pinned in `.nvmrc` (Node 26.8.2), `package.json` (npm 12.1.0), and `rust-toolchain.toml` (Rust 1.98.1). Both dependency lockfiles are versioned. The application manifest retains Rust 1.89 as its declared minimum, but CI validates the pinned toolchain, not every older compiler. Linux requires GTK 3, WebKitGTK 4.1, and the usual Tauri 2 build dependencies.

```bash
npm ci --ignore-scripts
npm run tauri:dev
```

The development server binds to loopback. Release builds do not need a Vite server, browser, Python collector, API key, or local model relay.

## Configuration

| Variable | Meaning |
| --- | --- |
| `CODEX_HOME` | Alternate local Codex data directory |
| `CODEX_SESSION_ROOT` | Alternate session root(s), separated by the OS path-list separator; at most 32 |
| `CODEX_SESSION_INDEX` | Alternate title-index path |
| `WTM_RECORD_LIMIT` | Retained-call limit from 100 to 100,000; default 50,000 |

## Supported records and limits

Per-call usage comes from `token_usage_record`, with optional model/effort context from `turn_context`. Flat counters and nested cached/reasoning counter details are supported. Legacy cumulative `event_msg.token_count` events are counted as a compatibility diagnostic, not guessed into individual calls. Records without a valid timestamp or any usable primary counter are skipped and diagnosed. Values outside JavaScript's exact per-counter integer range are marked invalid. Missing response IDs use a session/offset/time fallback; unusual externally rewritten logs can defeat that fallback, so exact response IDs remain preferable.

Each scan reads at most 4 MiB, checks at most 128 file candidates, and targets a soft 50 ms reading budget. Directory discovery is incremental and normally rescans every 10 seconds; notifications prioritize newly changed files. Individual JSONL lines are limited to 1 MiB, with an 8 MiB shared partial-line pool. Discovery tracks up to 20,000 log files/directories, titles up to 50,000, and each file retains 64 turn contexts. Exceeding limits is visible in diagnostics. Disk operations, parsing, watcher registration, and catalogue rebuilding can exceed the soft time budget on slow or unusually large workloads.

The index assumes append-oriented immutable per-call telemetry. Truncation, file replacement, and changes near the read checkpoint are handled; arbitrary same-inode edits elsewhere in a previously read file are not a general-purpose synchronization protocol. Existing retained IDs are not counted again. Removing source files does not immediately erase their already observed calls from the current window; restart to rebuild solely from currently available files.

## Linux rendering

On Wayland, startup supplies a process-local `__NV_DISABLE_EXPLICIT_SYNC=1` default before GTK starts, unless explicitly set by the user. This addresses the reproduced NVIDIA/Wayland protocol failure without disabling WebKit's whole DMABUF renderer or changing desktop-wide settings. Other graphics stacks and compositor versions still require validation. Native CI uses X11 under Xvfb; a successful headless test is not a universal graphics-driver certification.

## Tests and release build

```bash
npx playwright install chromium --only-shell
npm run format:check
npm test
npm run test:rust
cargo fmt --manifest-path src-tauri/Cargo.toml --check
cargo clippy --locked --manifest-path src-tauri/Cargo.toml --all-targets -- -D warnings
npm run tauri:build -- --ci
```

The release executable is `src-tauri/target/release/work-token-monitor`; bundling is disabled. Browser tests exercise the real React UI with mocked IPC. Rust tests use temporary synthetic logs, including retention, parsing, file replacement, and export permissions.

Native Linux acceptance uses the real executable, WebKit, and IPC. Install `WebKitWebDriver` from your distribution and `cargo install tauri-driver --version 2.0.6 --locked`, then run `npm run test:native` in a graphical session (or `xvfb-run -a npm run test:native` in CI). Optional `TAURI_DRIVER`, `WEBKIT_WEBDRIVER`, and `WTM_TEST_BINARY` select test executables. Test input, preferences, and exports are isolated from real Codex data. The driver is external test tooling, not bundled application code.

A synthetic backend benchmark is available with `cargo test --release --locked --manifest-path src-tauri/Cargo.toml benchmark_100k_records -- --ignored --nocapture`. It does not issue model calls. See [the optimization report](docs/optimization-2026-09-23.md) for measured results and remaining limits, and [the upgrade note](docs/upgrade-token-only.md) before upgrading from the removed collection feature.

## License

MIT. See [LICENSE](LICENSE).
