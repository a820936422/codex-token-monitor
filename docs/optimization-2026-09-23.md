# Token monitor optimization — 2026-09-23

This report supersedes the improvement backlog in the earlier [project review](project-review-2026-09-23.md). The model-audit/collector feature remains removed. Version 0.4 implements the agreed work in the order below; it does not claim unlimited history, independent model verification, or perfect log compatibility.

## 1. Consistent statistics and retention

`monitor/store.rs` retains the greatest `(timestamp_ms, id)` keys in a bounded ordered map, with a retained-ID index of the same size. File traversal order no longer determines which calls survive. `monitor/model.rs` defines revisioned update batches with additions and removals. `useMonitor.ts` checks count/revision consistency, ignores old updates, and reloads a snapshot after a gap. The initial snapshot and concurrently arriving batches cannot silently resurrect evicted calls.

Coverage and truncation are displayed. Table rows, filtered summaries, group views, and exports use the same current window. An in-memory observed window is intentionally not a durable full-history billing ledger.

## 2. Large-input and display performance

`reader.rs` reads measured byte ranges in 64 KiB chunks, with a 1 MiB line limit and shared 8 MiB partial-buffer pool. The engine limits a scan to 4 MiB and 128 candidates, targeting 50 ms of reading work. `discovery.rs` walks directories incrementally; `watch.rs` prioritizes native filesystem changes and falls back to periodic discovery/polling. Catalogue work is invalidated by meaningful changes instead of rebuilt for every heartbeat. Bounds and failures are diagnosed, not hidden.

`MonitorTable.tsx` renders a small overscanned window of fixed-height rows. Status-only updates preserve the frontend call-map identity, avoiding repeated sorting/aggregation. One batch clones the map once instead of once per call. Closed pickers do not render thousands of options; open pickers are searchable and limit results to 200.

A release-mode synthetic benchmark on the development workstation processed **100,000 records / 22,000,000 bytes**, retaining 50,000, in **619 ms across 9 explicit scans**. The maximum recorded scan was **57 ms**; an unchanged scan averaged **267 microseconds** over 20 iterations. This benchmark loops directly without UI pacing, uses warm synthetic local data, and excludes event transport and desktop rendering. It is not a cold-disk, many-directory, or cross-machine guarantee. Reproduce with the ignored `benchmark_100k_records` test.

The browser regression additionally loads 50,000 records, verifies fewer than 100 rendered call rows, scrolls to the oldest row, and exports all 50,000 records. Performance tests do not assert fragile machine-specific time thresholds.

## 3. Compatibility and data quality

The parser accepts per-call telemetry with flat or nested usage details. It normalizes RFC3339 timestamps to UTC instants, rejects missing/invalid times instead of inventing the current time, bounds turn contexts, and distinguishes known zero from unknown/invalid counters. Legacy cumulative events are diagnosed without being counted again. Invalid/unsafe integers, missing fields, cache/input inconsistencies and explicit total mismatches remain visible. BigInt aggregation preserves integer totals above the per-number safe range.

## 4. Runtime observability

The frontend distinguishes starting, indexing, live, stale, partial, unsupported and failed states. Heartbeat expiry is based on receipt time rather than the timestamp of the last model call. Retry performs a new synchronization. The diagnostics panel reports progress, bytes, timing, unreadable roots/files, malformed/invalid/legacy records, partial lines, watcher failures and resource limits.

## 5. Reproducibility and native acceptance

Node 26.8.2, npm 12.1.0, Rust 1.98.1, Playwright 1.63.0, Prettier 3.9.9 and the external tauri-driver 2.0.6 are pinned. The npm and Cargo lockfiles are tracked. CI uses locked installs, formatting, TypeScript, browser tests, Rust tests/Clippy, release compilation and a native Xvfb/WebKit acceptance test. Native automation runs in external test processes; no HTTP test API is shipped in the application.

Native testing also exposed a reproducible Wayland `Error 71` startup failure on the development graphics stack. A process-local `__NV_DISABLE_EXPLICIT_SYNC=1` default, applied before GTK starts and preserving existing overrides, resolved it without disabling the entire DMABUF renderer. X11 and Wayland native tests subsequently passed. This is a tested compatibility default for the observed environment, not proof of support for every GPU/compositor. The upstream family of reports is tracked in Tauri issue #10702.

## 6. Summaries and exports

Local-calendar date shortcuts, project/conversation aggregation, adaptive compact columns and searchable pickers share the same filtered data. CSV and JSON export every filtered record, including off-screen virtual rows. Export scope travels with the data. Default export replaces project/conversation labels and omits real IDs/paths; including them requires an explicit checkbox. CSV formula-like text is escaped. Native backend export accepts only content/format, asks for a destination, limits size, and writes atomically with Unix mode 0600.

## Verification performed

| Check | Result |
| --- | --- |
| Locked npm install | Passed; npm audit reported zero known findings during this run |
| Frontend TypeScript and browser/data tests | 31 passed |
| Rust tests | 30 library tests plus 1 platform test passed; 1 benchmark ignored in the normal suite |
| Explicit release benchmark | Passed, measured above |
| Rust formatting and Clippy with warnings denied | Passed |
| Prettier formatting checks | Passed |
| Frontend and Tauri release builds | Passed |
| Native real-IPC smoke test | Passed on local X11 and Wayland after compatibility fix |

Native acceptance verifies initial loading, real append events, reaching and retaining exactly 100 of 104 synthetic calls, a bounded rendered table, project summary, and unchanged synthetic Codex configuration. Save-file dialog interaction itself is not automated end to end; export content/privacy/error behavior is covered in browser tests and atomic writing/permissions in Rust tests. WebDriver screenshot capture timed out on the local stack; screenshot capture is optional, while all native functional assertions are required. Browser dark/light screenshots were inspected separately.

No live model request, actual account restriction test, Windows/macOS runtime test, all-version Codex compatibility test, or multi-day real-world soak test was performed. The GitHub workflow is updated; its remote execution status is separate from these local results.

## Remaining engineering limits

The product now deliberately exposes a bounded loaded window. Persistent full-history storage, arbitrary mutation tracking, and a universal Codex log adapter are not implemented. Long or image-heavy JSONL records exceeding limits are skipped with diagnostics. Native watcher limits, slow disks and large catalogues can still exceed the soft scan target. Benchmark numbers above are single-workstation observations, not regression promises. Future work should be driven by concrete unsupported log fixtures, native platform coverage and sustained-load measurements rather than another speculative model/account detector.
