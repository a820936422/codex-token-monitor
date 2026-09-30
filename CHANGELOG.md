# Changelog

All notable changes to Work Token Monitor will be documented here.

## 0.4.0 - 2026-09-23

- Unified chronological record retention and bounded deduplication; revisioned batches synchronize UI evictions and recover missed updates
- Added bounded incremental discovery/reading, native file notifications, polling fallback, and virtualized tables
- Preserved missing/invalid counters as unknown, normalized timestamps, and added log compatibility/data-quality diagnostics
- Added heartbeat expiry, retry, loaded-window coverage and explicit truncation indicators
- Added local-calendar presets, project/conversation summaries, searchable selectors and adaptive column presets
- Added user-selected CSV/JSON exports with default identifier redaction, exact aggregate totals and CSV formula escaping
- Versioned lockfiles, pinned development tools, expanded browser/Rust/native tests and formatting checks
- Added process-local Wayland rendering compatibility without changing desktop configuration


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
