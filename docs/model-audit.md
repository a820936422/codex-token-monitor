# Model declaration auditing / 模型声明核对

Work Token Monitor can compare each call's locally selected model with the model declared in its actual HTTP response. The optional collector observes ordinary requests; it does not issue extra model tests. The desktop app still reads local files only.

模型核对检查的是 **provider 返回的模型声明**。中转站返回的名称可能经过改写；官方直连提供更直接的声明，也不是对内部推理模型身份的独立认证。一致、不同或冲突都不能单独证明风控或降级原因。

## Quick start: your configured relay / 当前中转 provider

Python 3.11+ and Codex are required. From this repository:

```bash
python3 scripts/model_audit.py run --mode configured
```

This launches an ordinary interactive Codex session using the custom Responses provider in `$CODEX_HOME/config.toml` (default `~/.codex/config.toml`). It changes routing only for the launched process, preserves the provider's credential configuration, and leaves your config file and existing desktop conversations alone. Model requests go through a loopback relay to the original HTTPS base URL. Codex writes normal session telemetry, which the monitor joins to the captured metadata by exact `response_id`.

To run a normal task, select a model, resume a session, or use a config profile, pass Codex arguments after `--`:

```bash
python3 scripts/model_audit.py run --mode configured -- exec --sandbox read-only "Explain this project"
python3 scripts/model_audit.py run --mode configured -- --profile work
python3 scripts/model_audit.py run --mode configured -- resume
```

`--codex /path/to/codex` selects a binary. Otherwise the launcher uses `codex` on PATH, then `/usr/lib/chatgpt/resources/codex`. Stop the session normally or with Ctrl-C; the temporary relay closes with it. The regular task consumes normal usage; collecting metadata adds no model calls.

Configured mode requires a **named custom provider** with an explicit HTTPS `base_url` and `wire_api = "responses"`. Built-in `openai` cannot be overridden in the same way. For an OpenAI API key, create a named custom provider; for a subscription login, use official mode below. Route overrides passed through Codex `-c` are rejected to prevent accidentally bypassing collection.

The launcher reads provider selection and routing from the user config and selected profile: current `<profile>.config.toml` files, or the legacy `[profiles.<profile>]` format for older clients. Codex `0.155.0-alpha.9` requires the separate profile file. Project/managed configuration layers are not resolved by this helper; use an explicitly configured `serve` route when routing comes from those layers.

## Official subscription / 官方订阅直连

First sign in with your subscription using the official Codex client (`codex login`, or the bundled binary's `login` command), then:

```bash
python3 scripts/model_audit.py run --mode official
```

This creates a fresh, process-local provider with `requires_openai_auth = true`, display name `OpenAI`, and a fixed upstream of `https://chatgpt.com/backend-api/codex`. Codex manages login and token refresh. The collector forwards credentials supplied by the client and never reads `auth.json`. An API-key login is not a ChatGPT subscription login; ensure the active Codex login is the intended subscription. Choose a model available to that account using the normal Codex model selector or `-- -m MODEL`.

The launcher disables WebSockets and request compression for that process, keeping ordinary HTTP/SSE requests inspectable by the standard-library collector. Those settings affect transport, not the selected model or reasoning effort. Other ChatGPT functions such as login/refresh are not proxied. Workspace-specific/enterprise routing may need a different upstream and is not guaranteed by this fixed public endpoint.

The official configuration path was checked against Codex `0.155.0-alpha.9`, source commit [`434535bddfaf`](https://github.com/openai/codex/tree/434535bddfaf405a032f57be3c1096dd25ff6312). In that version, overriding `model_providers.openai` does not replace the built-in provider; the old `responses_websockets` feature flags are removed. A fresh custom provider and `features.enable_request_compression = false` are intentional. Newer client versions may require updated configuration.

## Existing desktop conversations / 桌面客户端持续采集

The launcher above does **not** attach to a desktop client that is already running. To capture its future calls, keep a relay running and explicitly configure that client's provider to use it. Close active calls before changing routing. Preserve a copy of your original configuration so you can restore the original base URL when stopping the relay.

Start the relay with the provider's **complete original base URL**, including its existing `/v1` suffix if applicable:

```bash
python3 scripts/model_audit.py serve --upstream https://provider.example/v1 --port 4319
```

In the existing **custom** provider section of the desktop client's Codex configuration, change only routing/capabilities. Keep existing credential settings in the client:

```toml
# Top-level feature table; merge into an existing [features] table if present.
[features]
enable_request_compression = false

# Use your existing provider ID here. Leave its name/authentication settings intact.
[model_providers.your_existing_provider]
base_url = "http://127.0.0.1:4319/v1"
supports_websockets = false
```

Do not duplicate existing TOML table headers; edit the corresponding fields. Restart the desktop client so it loads the change. If its UI manages the provider URL separately or rewrites config, set the URL through that UI and confirm new observations appear. The monitor's **模型采集说明** panel shows the watched directory and last observation; those indicate captured data, not whether a relay is currently alive.

For an official subscription desktop configuration, use a **new provider ID**, with no `env_key`, bearer token, or custom auth configured:

```bash
python3 scripts/model_audit.py serve --upstream https://chatgpt.com/backend-api/codex --port 4319
```

```toml
# Top-level setting, before any [table] headers:
model_provider = "wtm_official_audit"

[features]
enable_request_compression = false

[model_providers.wtm_official_audit]
name = "OpenAI"
base_url = "http://127.0.0.1:4319/v1"
wire_api = "responses"
requires_openai_auth = true
supports_websockets = false
```

The client must already be signed in with the official subscription. Do not overwrite the rest of your configuration with these fragments. When disabling collection, restore the original provider/base URL and transport settings, restart the client, then stop the relay.

## Display and evidence rules

| Display | Meaning |
| --- | --- |
| 声明一致 / match | One observed model identifier exactly equals the locally selected model, with no capture issues. |
| 声明不同 / different | The selected and response-declared identifiers differ. The reason is not inferred. |
| 声明冲突 / conflict | Headers or response events declare different model identifiers, or an ID has incompatible observations. |
| 未知 / unknown | A response was captured but a required model field is absent, or capture was incomplete/unreadable. |
| 未采集 / unobserved | No metadata could be joined to this call's `response_id`. Historical logs alone cannot fill this gap. |

Click a row's audit button to inspect the local model, actual outgoing model, returned model, all observed declarations, evidence sources, upstream origin, completion status and response ID. The main status compares the local selection to the response declaration. The actual outgoing model is displayed separately so request mapping is visible. Token totals are unaffected by late metadata updates.

The observer recognizes Responses JSON and SSE `response.model`, top-level JSON `model`, HTTP `openai-model` / `x-openai-model`, and the same headers in stream metadata. Terminal event declarations take precedence for display; earlier differing declarations remain a conflict. A body model is valid declaration evidence without model headers. Values are compared exactly: aliases, casing and dated versions are not silently equated.

Only exact response IDs are used for association. Calls lacking IDs stay unobserved. The collector does not currently attach to app-server reroute notifications; a thread/turn notification is not automatically attributed to a particular response.

## Storage, privacy and limits

Metadata goes to `$CODEX_HOME/model-audit/capture-*.jsonl`, or `CODEX_MODEL_AUDIT_DIR`. Start both the monitor and collector with the same directory configuration. `run --output-dir PATH` and `serve --output-dir PATH` override only the collector; use `CODEX_MODEL_AUDIT_DIR=PATH` for the monitor as well.

Records contain schema version, response ID, model declarations, evidence sources, timestamp, provider **origin** (no URL path/query), transport, HTTP status, completion and fixed capture issue codes. Failed outbound attempts are recorded as unknown; without a response ID they cannot join a token record and are excluded from the UI's cached-ID count. No request/response bodies, auth headers, account IDs, or API keys are persisted. Credentials and bodies necessarily pass through relay memory. Files are created with mode `0600`, new directories with `0700`; protect your account and filesystem accordingly. These identifiers can still be sensitive, so review metadata before sharing it.

Each collector owns unique filenames and rotates at 8 MiB. The monitor retains a bounded metadata cache. Files are not automatically deleted; remove old `capture-*.jsonl` files when you no longer need to reload their audit history. Never put your actual capture directory under version control.

The relay binds only to `127.0.0.1`. It forwards client-supplied authentication to one fixed HTTPS destination with normal certificate verification, rejects redirects and browser-origin requests, and accepts only `/responses`, `/responses/compact`, and `/models` routes below its local `/v1` prefix. Compaction/models requests are forwarded but are not treated as ordinary call evidence. Upstream HTTP errors, including 401 and 429, are returned to Codex so authentication recovery and retries can proceed.

Supported transport is HTTP JSON/SSE, with Content-Length requests up to 32 MiB and up to 16 concurrent connections. WebSockets, chunked or compressed request bodies are rejected explicitly. Upstream response bytes are forwarded unchanged; individual observed JSON/SSE events are bounded at 2 MiB. Oversized, malformed or unsupported compressed responses degrade audit evidence instead of claiming a match. Oversized events can occur in image-heavy responses. A metadata write failure is reported to the terminal and does not intentionally stop response forwarding.

## Verification

```bash
python3 -m unittest discover -s scripts -p 'test_*.py' -v
cargo test --manifest-path src-tauri/Cargo.toml
npm run build
```

Python tests use synthetic local HTTP services, including byte-preserving streaming, JSON, conflicts, malformed/oversized evidence, concurrency, credential exclusion, redirects, error status preservation and config generation. When a Codex executable is installed, an additional test runs the real client against a local synthetic upstream and checks the exact join to its token record. It uses a temporary Codex home and makes no live model request. Rust tests cover ingestion and both orders of call/observation arrival.

Official subscription routing is source-verified and its authentication forwarding is exercised with synthetic headers. These tests do not claim a live official-account test or independent model identity verification. The earlier one-call provider experiment is documented in [model-verification-probe.md](model-verification-probe.md).
