# Changelog

All notable changes to Work Token Monitor will be documented here.

## Unreleased

- Added synthetic Rust and browser regressions for token monitoring, filters, initialization, privacy boundaries, and file changes
- Fixed startup preference loss, failed-initialization listener cleanup, same-size log replacement, and overflow-safe token totals

- Removed model declaration auditing, automatic collection, relay/configuration management, and the standalone network probe
- Returned the desktop app to read-only local token monitoring; model names are explicitly labeled as log metadata
- Removed the Python runtime requirement and direct configuration-editing dependency
- Preserved existing token summaries, filters, column resizing, and snapshot/event merging

- Added persistent drag-resizable table columns with keyboard resizing and double-click reset
- Added filtered total-token usage summary with K/M/B compact units
- Added reasoning-effort display beneath each model name

## 0.3.0 - 2026-09-14

Initial public release candidate.

- Migrated to Tauri 2 + Rust + React + TypeScript
- Added live per-call token monitoring
- Added project and conversation filters
- Added date-range filtering
- Added conversation ID copy action
- Added local-only privacy boundaries and restricted clipboard permissions
