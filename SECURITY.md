# Security Policy

## Reporting

Report vulnerabilities privately to the maintainer using GitHub private vulnerability reporting when available. Do not publish real session logs, `auth.json`, credentials, conversation bodies, IDs, or identifying local paths. Use a minimal synthetic reproduction.

## Boundaries

Work Token Monitor reads local JSONL telemetry and title metadata. It does not read authentication or Codex configuration files, intercept requests, start a model relay, make model calls, or upload monitored data. Session files may contain bodies; JSON is parsed transiently, but only bounded telemetry/context fields enter the application model. Diagnostic output uses fixed issue codes rather than raw response bodies or untrusted error content.

The retained-call/ID window is bounded, directory discovery and reads are budgeted, JSONL lines and partial buffers are limited, and turn/title caches have explicit limits. These reduce resource amplification; they do not make hostile files or all filesystem races harmless. Check runtime diagnostics for skipped/invalid input, resource limits, and incomplete indexing.

The webview has Tauri core permissions and clipboard write-text permission. Clipboard read and arbitrary frontend filesystem writes are not granted. The custom export command takes a format and bounded content, never a caller-provided filesystem path. A native dialog must select the output destination. Only CSV/JSON extensions are allowed; directories/symlinks are rejected. Unix output is written to a new 0600 temporary file beside the destination and atomically renamed, avoiding truncation of an existing output before the new file is ready.

Exports omit real project/conversation labels, IDs, and paths by default; users can explicitly opt in. Timestamps, model names and usage remain sensitive, so review exports before sharing. Default redaction is not an anonymity guarantee. CSV text is quoted and formula-like prefixes are escaped. JSON aggregate integers use decimal strings. Export scope describes the filtered retained window, not a full history or billing statement.

Local UI preferences are stored in webview storage. Screenshots can reveal titles and paths. Protect the OS account and filesystem. Private logs and old captures must remain outside Git; the legacy ignore rules remain as a safeguard even though collection code was removed.

## Testing and dependencies

Lockfiles and toolchain versions are versioned. Browser, Rust and native tests use synthetic temporary data. The external WebDriver bridge binds locally, is used only by the test runner, and is not embedded in the production executable. Keep test ports private and do not run acceptance tests against live account directories.

Passing tests, static checks, or a dependency audit is not a complete security certification. Platform-specific permissions, WebKit updates, save-dialog behavior and long-running workloads need ongoing review.
