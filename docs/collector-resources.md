# Collector resource measurements

Measured on Linux with 16 logical CPU threads on 2026-09-20, using synthetic data only. No credentials, conversation logs, real model calls or production upstreams were used. CPU percentages below use **one logical core as 100%**.

## Actual managed helper, idle

The packaged helper entry point (`model_collector_service.py`) was launched with a temporary control socket and output directory. Its controller was polled every two seconds, matching the desktop cadence. The upstream was never contacted.

| State | Sampling time | RSS | Measured CPU time |
| --- | ---: | ---: | ---: |
| Capture enabled, no requests | 10.00 s | 34.27 MiB | 0.00 s |
| Capture paused, no requests | 10.00 s | 34.27 MiB | 0.00 s |

CPU accounting has approximately 10 ms resolution, so these readings mean idle usage was below the measurement resolution (roughly 0.1% of one core over ten seconds), not that the process can consume literally no CPU. Pausing capture retains the forwarding service and its baseline memory. Full stop releases the helper; restore direct routing and restart Codex first.

## Synthetic streaming load

A localhost upstream generated 256 KiB of text per response as 128 SSE deltas, paced one millisecond apart, followed by terminal response-ID/model metadata. Upstream, relay and clients ran in separate processes. Every successful response was checked byte-for-byte by SHA-256. Each route/concurrency combination ran for about three seconds; in-flight requests were allowed to finish.

| Concurrent requests | Mean direct latency | Mean relay latency | Relay throughput | Relay CPU | Sampled relay RSS peak |
| ---: | ---: | ---: | ---: | ---: | ---: |
| 1 | 140.41 ms | 140.96 ms | 7.09 req/s | 5.16% | 37.26 MiB |
| 4 | 138.61 ms | 138.93 ms | 28.78 req/s | 17.66% | 37.98 MiB |
| 8 | 138.21 ms | 139.23 ms | 57.38 req/s | 37.50% | 38.73 MiB |
| 16 | 138.66 ms | 140.00 ms | 113.88 req/s | 69.88% | 41.44 MiB |

The load harness imports measurement libraries into the relay process, so its baseline RSS is about 3 MiB higher than the actual managed helper. RSS was sampled every 20 ms and can miss shorter peaks. These are short local measurements, not a production latency guarantee or real-model benchmark. The 16-request load used approximately 0.70 of one CPU core; the other cores were not saturated by the relay.

The final run had zero HTTP 503 rejections at all four tested concurrency levels. An initial run exposed a brief slot-release race at the 16-connection limit. The relay now waits up to 50 ms for a just-finished connection to release its slot; the active-connection cap remains 16. Requests beyond sustained capacity can still receive 503.

Each captured synthetic response produced exactly **500 bytes** of JSONL. At that record size, 10,000 responses require about 4.77 MiB. Real record sizes depend on identifier lengths, evidence and failure details. Files rotate at 8 MiB; rotation does not cap total disk usage and old files are not automatically removed.

## Scope and limits

These figures describe the collector helper. They exclude the Tauri/WebView window, Rust log scanner, Codex and model-provider computation. The UI exposes the helper's live RSS/CPU so users can inspect their own workload.

The collector creates no additional model requests or token usage. It adds local HTTP forwarding, metadata parsing and a small metadata write. Disabling collection skips response observation and metadata writes while preserving forwarding.

Small benchmark requests do not establish a worst-case memory bound. Up to 16 concurrent request bodies of 32 MiB already represent 512 MiB before JSON decoding and response buffers. Large contexts or images can therefore use substantially more memory than the table. Each observed JSON/SSE event is capped at 2 MiB, and the request queue/concurrency are bounded. See [transport and storage limits](model-audit.md#storage-privacy-and-limits).
