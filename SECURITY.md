# Security Policy

## Reporting a vulnerability

Please avoid filing a public issue for a vulnerability that could expose local ChatGPT Work / Codex data. Report it privately to the repository maintainer using GitHub's private vulnerability reporting feature when available.

When reporting, do not attach raw `~/.codex` logs, `auth.json`, credentials, private conversation content, real conversation IDs, or identifying local filesystem paths. Prefer a minimal synthetic reproduction.

## Data-access boundary

Work Token Monitor is designed to read local Codex session telemetry and metadata only. It does not require an API key, does not read `auth.json`, and does not intentionally transmit monitored data over the network.

The frontend has Tauri core permissions plus clipboard write-text permission. Clipboard read permission is intentionally not enabled.
