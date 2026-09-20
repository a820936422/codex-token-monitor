# Model metadata probe

The standalone probe verifies whether a configured Codex provider exposes model metadata that can be linked to a per-call token record. It is an experimental diagnostic, not continuous monitoring of desktop conversations.

## Verified result

One real probe completed on 2026-09-20 using the Codex executable bundled with ChatGPT (`0.155.0-alpha.9`) and the existing custom Responses provider:

| Observation | Result |
| --- | --- |
| Outbound request model | `gpt-6-astra` |
| Response body `model` | `gpt-6-astra` |
| HTTP `openai-model` / `x-openai-model` | Absent |
| Stream event model headers | Absent |
| Completed responses | 1 |
| Matching `token_usage_record` by exact `response_id` | 1 |
| Client exit code | 0 |
| Elapsed time | 24.18 seconds |
| Classification at the time | `unverified_body_only` (earlier evidence rules; the current declaration-only rules classify this as `match`) |

This proves that response metadata can be collected and joined to the existing token record format. It does **not** independently establish which backend model performed inference, and it does not establish a downgrade. The names reported by the request and response body matched in this single probe.

The local report is kept at `.model-probes/latest.json`, outside version control. It includes the synthetic probe's response/session/turn IDs and token usage, so do not publish it without reviewing it. The request used 14,203 input tokens and 8 output tokens, including Codex's built-in instructions; a short user prompt is not necessarily a small Codex request.

## Run a probe

Requirements: Python 3.11+, a Codex executable, and an explicitly configured HTTPS Responses provider with a bearer credential in the Codex config or its configured environment variable. The tool does not read `auth.json` or borrow credentials from another provider.

```bash
python3 scripts/model_probe.py --output .model-probes/latest.json
```

The script finds `codex` on `PATH`, then checks `/usr/lib/chatgpt/resources/codex`. To specify another executable or config:

```bash
python3 scripts/model_probe.py \
  --codex /path/to/codex \
  --config /path/to/config.toml \
  --output .model-probes/latest.json \
  --timeout 90
```

Running this command makes a real model request through the configured provider and consumes its normal usage allowance. It runs one isolated Codex task with the prompt `Reply with exactly MODEL_PROBE_OK. Do not use any tools.` Retries are disabled in the temporary provider configuration; every observed response is included in the report.

For a visible, independent terminal on KDE:

```bash
konsole --separate --nofork -e python3 scripts/model_probe.py \
  --output .model-probes/latest.json --timeout 90
```

The tool prints elapsed time, relay RSS, and captured request count every five seconds. The terminal closes on completion. Exit status 0 means the client finished and a response was captured; it does **not** mean model identity was verified. Inspect `records[].verdict`, `completed`, and `matchingTokenRecords`.

## Data flow and isolation

```text
isolated Codex exec → authenticated loopback relay → configured HTTPS provider
                              ↓
                       model-only metadata
                              ↓ exact response_id join
                    temporary Codex token records
```

The script creates temporary Codex configuration and an empty workspace. It does not modify the running desktop application's configuration, attach to its conversations, or send existing conversation logs. A child-process `CODEX_HOME` points to the temporary directory only.

The relay listens on `127.0.0.1` using a random port and requires a random local bearer token. Real provider credentials are read from the selected configuration and retained in memory; the temporary config contains only the local relay token. The relay forwards requests only to the selected provider's `/responses` and `/models` endpoints. HTTP redirects are rejected so credentials cannot be forwarded to a redirect destination.

HTTP bodies pass through memory to Codex. The report stores only model identifiers, response IDs, event type counts, token-record metadata, and sanitized error categories. Temporary Codex logs, configuration, workspace, and child output are removed after inspection. The report is written with mode 0600.

## Evidence rules

| Verdict | Meaning |
| --- | --- |
| `match` | One response-declared model equals the outbound request model, with no capture issues. |
| `different` | The request and response-declared identifiers differ. The reason is not inferred. |
| `conflict` | Different model identifiers were observed in the response body, HTTP headers or event headers. |
| `unknown` | Required model metadata is absent or capture is incomplete, malformed or oversized. |

The probe and continuous collector share the same parser. Response-body `model` is valid evidence of a **provider declaration** even without model headers. Terminal declarations take precedence for display; earlier differing values remain conflicts. Model aliases, casing and dated versions are compared exactly. All verdicts concern exposed identifiers, not independent authentication of backend model identity.

Codex also defines a [model reroute notification](https://github.com/openai/codex/blob/main/codex-rs/app-server-protocol/schema/typescript/v2/ModelReroutedNotification.ts). The collector does not attach to that event channel or infer a per-response reroute from a thread/turn event.

## Integration boundary

The probe's isolated session is deleted after its token-record association is checked. Its report is diagnostic output, not an input file for the desktop table.

The separate [continuous collector](model-audit.md) observes ordinary requests, writes bounded metadata files, and integrates with the Rust monitor and React model-audit column through exact `response_id` joins. Historical calls without such metadata remain uncaptured. Neither collector labels a provider declaration as an authenticated model identity.

## Tests

```bash
python3 -m unittest discover -s scripts -p 'test_model_probe.py' -v
```

These offline tests cover header casing, missing and conflicting evidence, exact identifier comparisons, metadata deduplication, SSE forwarding, header preservation, local authorization, redirect rejection, and exclusion of text/credentials from the report. Tests use synthetic data and local HTTP servers only.
