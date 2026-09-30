# Security Policy

## Reporting a vulnerability

Please avoid filing a public issue for a vulnerability that could expose local ChatGPT Work / Codex data. Report it privately to the repository maintainer using GitHub's private vulnerability reporting feature when available.

When reporting, do not attach raw `~/.codex` logs, `auth.json`, credentials, private conversation content, real conversation IDs, or identifying local filesystem paths. Prefer a minimal synthetic reproduction.

## Data-access boundary

Work Token Monitor is designed to read local Codex session telemetry and metadata only. It does not require an API key, does not read `auth.json`, and does not intentionally transmit monitored data over the network.

The frontend has Tauri core permissions plus clipboard write-text permission. Clipboard read permission is intentionally not enabled.

The backend opens session JSONL and the session index read-only. JSONL may contain conversation bodies, but the application data model retains only usage and display metadata. It does not read or edit Codex configuration, load authentication files, spawn a collector, or issue model requests. A Vite development server is used only during frontend development.

Old collection metadata is not read, migrated, or deleted by this version. See [the upgrade note](docs/upgrade-token-only.md) for restoring an older installation before upgrading.
