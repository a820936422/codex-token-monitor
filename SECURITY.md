# Security Policy

## Reporting a vulnerability

Please avoid filing a public issue for a vulnerability that could expose local ChatGPT Work / Codex data. Report it privately to the repository maintainer using GitHub's private vulnerability reporting feature when available.

When reporting, do not attach raw `~/.codex` logs, `auth.json`, credentials, private conversation content, real conversation IDs, or identifying local filesystem paths. Prefer a minimal synthetic reproduction.

## Data-access boundary

Work Token Monitor reads local Codex session telemetry and model metadata. It has no analytics and does not upload monitored history. It does not read `auth.json`. Its app-managed collector, enabled by default and controllable in the UI, forwards the normal model requests of the configured Codex client as described below.

The frontend has Tauri core permissions plus clipboard write-text permission. Clipboard read permission is intentionally not enabled.

## Optional diagnostic script

`scripts/model_probe.py` is a separate, opt-in network diagnostic. It sends an isolated test task to the Responses provider selected in the supplied Codex configuration. It reads that provider's configured credentials for the request; it does not read `auth.json` or existing conversation logs. Credentials stay in memory, HTTP redirects are rejected, and the temporary loopback relay requires a random local token.

Probe reports contain model identifiers, response/session/turn IDs, and token usage. Reports exclude conversation bodies and credentials and are written with mode 0600. Keep reports in the git-ignored `.model-probes/` directory and review them before sharing.

## Continuous model collector

The app embeds `scripts/model_audit.py` and a private control service. Automatic collection starts a loopback HTTP relay and edits selected user-config routing/transport fields, with a recovery journal written before config replacement. A manual switch pauses capture; a saved preference disables startup collection. The standalone CLI remains available for explicit setups.

The relay forwards authentication provided by the client to one fixed HTTPS upstream. It does not read `auth.json`, cache credentials, follow redirects, or accept browser-origin requests. OAuth login and token refresh remain the Codex client's responsibility. HTTP/SSE transport is required; app-managed routing disables WebSockets and request compression in the user config and restores managed values on normal exit.

Requests and responses pass through relay memory. Only response IDs, model identifiers, evidence sources, completion/capture status, timestamps and provider origins are persisted. Records exclude bodies, account IDs, URL paths/queries and credentials. Capture files are created with mode 0600 and rotated; old files are not automatically deleted. The desktop backend reads them locally and associates calls only by exact response ID.

The private control socket is mode 0600 in a 0700 directory and has no HTTP administrative route. A controller lock prevents multiple app instances from taking ownership. The recovery journal excludes credentials and stores only the original/applied routing fields. Restoration preserves fields changed by the user or another application. Model collection stops on normal exit; after a crash the short controller lease expires. In both cases the helper can continue forwarding for already-open clients. Restart Codex after restoring direct routing before stopping the helper.

Use `CODEX_MODEL_AUDIT_DIR` to select a private directory and keep captures out of Git. These records are provider declarations and can be forged by the provider or someone who can edit your files; they do not authenticate backend model identity. See [the setup guide](docs/model-audit.md) for transport limits and desktop integration.
