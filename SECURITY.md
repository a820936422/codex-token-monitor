# Security Policy

## Reporting a vulnerability

Please avoid filing a public issue for a vulnerability that could expose local ChatGPT Work / Codex data. Report it privately to the repository maintainer using GitHub's private vulnerability reporting feature when available.

When reporting, do not attach raw `~/.codex` logs, `auth.json`, credentials, private conversation content, real conversation IDs, or identifying local filesystem paths. Prefer a minimal synthetic reproduction.

## Data-access boundary

Work Token Monitor is designed to read local Codex session telemetry and metadata only. It does not require an API key, does not read `auth.json`, and does not intentionally transmit monitored data over the network.

The frontend has Tauri core permissions plus clipboard write-text permission. Clipboard read permission is intentionally not enabled.

## Optional diagnostic script

`scripts/model_probe.py` is a separate, opt-in network diagnostic. It sends an isolated test task to the Responses provider selected in the supplied Codex configuration. It reads that provider's configured credentials for the request; it does not read `auth.json` or existing conversation logs. Credentials stay in memory, HTTP redirects are rejected, and the temporary loopback relay requires a random local token.

Probe reports contain model identifiers, response/session/turn IDs, and token usage. Reports exclude conversation bodies and credentials and are written with mode 0600. Keep reports in the git-ignored `.model-probes/` directory and review them before sharing.

## Optional continuous model collector

`scripts/model_audit.py` is a separate, opt-in loopback HTTP relay for normal Responses calls. It forwards the authentication provided by the configured client to one fixed HTTPS upstream. It does not read `auth.json`, cache credentials, follow redirects, or accept browser-origin requests. OAuth login and token refresh remain the Codex client's responsibility. HTTP/SSE transport is required; the launcher disables WebSockets and request compression only for the process it starts.

Requests and responses pass through relay memory. Only response IDs, model identifiers, evidence sources, completion/capture status, timestamps and provider origins are persisted. Records exclude bodies, account IDs, URL paths/queries and credentials. Capture files are created with mode 0600 and rotated; old files are not automatically deleted. The desktop backend reads them locally and associates calls only by exact response ID.

Use `CODEX_MODEL_AUDIT_DIR` to select a private directory and keep captures out of Git. These records are provider declarations and can be forged by the provider or someone who can edit your files; they do not authenticate backend model identity. See [the setup guide](docs/model-audit.md) for transport limits and desktop integration.
