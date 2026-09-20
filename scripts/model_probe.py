#!/usr/bin/env python3
"""Probe model metadata through the configured Codex provider without changing it.

Uses a temporary, loopback-only relay and an isolated Codex home/workspace. Real
upstream credentials stay in memory; temporary config contains a random local
relay token only. The report contains model metadata and IDs, never HTTP bodies,
credentials, conversation text, or arbitrary response headers.
"""
from __future__ import annotations

import argparse
import collections
import datetime as dt
import hmac
import http.server
import json
import os
from typing import cast
from pathlib import Path
import resource
import secrets
import shutil
import subprocess
import tempfile
import threading
import time
import tomllib
import urllib.error
import urllib.parse
import urllib.request

MODEL_HEADERS = ("openai-model", "x-openai-model")
MAX_REQUEST_BYTES = 8 * 1024 * 1024


# Keep the isolated diagnostic and continuous collector on identical evidence rules.
from model_evidence import Evidence, ResponseObserver, model_headers

class NoRedirect(urllib.request.HTTPRedirectHandler):
    def redirect_request(self, req, fp, code, msg, headers, newurl):
        # Provider credentials must never be forwarded to a redirect destination.
        return None


class ProbeRelay(http.server.ThreadingHTTPServer):
    daemon_threads = True

    def __init__(self, base_url: str, headers: dict, local_token: str):
        super().__init__(("127.0.0.1", 0), RelayHandler)
        self.base_url = base_url.rstrip("/")
        self.upstream_headers = headers
        self.upstream = urllib.request.build_opener(NoRedirect())
        self.local_token = local_token
        self.evidence: list[Evidence] = []
        self.errors: list[dict] = []


class RelayHandler(http.server.BaseHTTPRequestHandler):
    @property
    def relay(self) -> ProbeRelay:
        return cast(ProbeRelay, self.server)

    def log_message(self, format: str, *args: object) -> None:
        pass

    def do_GET(self) -> None:
        self.forward()

    def do_POST(self) -> None:
        self.forward()

    def forward(self) -> None:
        if not hmac.compare_digest(self.headers.get("Authorization", ""),
                                   "Bearer " + self.relay.local_token):
            self.send_error(401)
            return
        route = urllib.parse.urlsplit(self.path)
        if route.path not in ("/v1/responses", "/v1/models") or route.query:
            self.send_error(404)
            return
        try:
            length = int(self.headers.get("Content-Length", "0"))
        except ValueError:
            self.send_error(400)
            return
        if length < 0 or length > MAX_REQUEST_BYTES:
            self.send_error(413)
            return
        body = self.rfile.read(length) if length else None
        requested = "unknown"
        if body:
            try:
                requested = json.loads(body).get("model", "unknown")
            except (ValueError, AttributeError):
                self.send_error(400)
                return
        headers = {
            key: value for key, value in self.headers.items()
            if key.lower() not in ("authorization", "host", "connection", "content-length",
                                   "transfer-encoding", "accept-encoding")
        }
        headers.update(self.relay.upstream_headers)
        headers["Accept-Encoding"] = "identity"
        url = self.relay.base_url + route.path.removeprefix("/v1")
        request = urllib.request.Request(url, data=body, headers=headers, method=self.command)
        try:
            upstream = self.relay.upstream.open(request, timeout=60)
        except urllib.error.HTTPError as exc:
            self.relay.errors.append({"kind": "upstream_http", "status": exc.code})
            exc.close()
            self.send_error(502, "Upstream rejected the probe")
            return
        except (urllib.error.URLError, TimeoutError, OSError) as exc:
            self.relay.errors.append({"kind": "upstream_connection", "errorType": type(exc).__name__})
            self.send_error(502, "Upstream connection failed")
            return
        with upstream:
            self.send_response(upstream.status)
            self.send_header("Content-Type", upstream.headers.get("Content-Type", "application/json"))
            for key in (*MODEL_HEADERS, "x-request-id"):
                if upstream.headers.get(key):
                    self.send_header(key, upstream.headers[key])
            self.send_header("Connection", "close")
            self.end_headers()
            evidence = None
            if route.path == "/v1/responses":
                evidence = Evidence(requested, upstream.headers.items())
                self.relay.evidence.append(evidence)
            observer = ResponseObserver(evidence, upstream.headers.get("Content-Type", "")) if evidence else None
            if observer and upstream.headers.get("Content-Encoding", "identity").lower() not in {"", "identity"}:
                observer.evidence.issue("unsupported_encoding")
                observer.disabled = True
            try:
                while chunk := upstream.read1(64 * 1024):
                    if observer:
                        observer.feed(chunk)
                    self.wfile.write(chunk)
                    self.wfile.flush()
            except (BrokenPipeError, ConnectionResetError, TimeoutError, OSError) as exc:
                self.relay.errors.append({"kind": "stream", "errorType": type(exc).__name__})
                if evidence:
                    evidence.issue("stream_interrupted")
            finally:
                if observer:
                    observer.finish()

def find_codex(explicit: str | None) -> str:
    candidates = [explicit, shutil.which("codex"), "/usr/lib/chatgpt/resources/codex"]
    for candidate in candidates:
        if candidate and Path(candidate).is_file() and os.access(candidate, os.X_OK):
            return candidate
    raise ValueError("Codex executable not found; pass --codex")


def load_provider(config_path: Path) -> tuple[str, str, dict]:
    config = tomllib.loads(config_path.read_text())
    model = config.get("model")
    name = config.get("model_provider")
    provider = config.get("model_providers", {}).get(name, {})
    base_url = provider.get("base_url", "")
    parsed = urllib.parse.urlsplit(base_url)
    if not isinstance(model, str) or not model:
        raise ValueError("A model must be explicitly configured")
    if parsed.scheme != "https" or not parsed.hostname or parsed.query or parsed.username:
        raise ValueError("Probe requires a configured HTTPS provider base_url")
    if provider.get("wire_api", "responses") != "responses":
        raise ValueError("Probe supports Responses providers only")
    headers = dict(provider.get("http_headers", {}))
    for key, env_name in provider.get("env_http_headers", {}).items():
        if os.environ.get(env_name):
            headers[key] = os.environ[env_name]
    bearer = provider.get("experimental_bearer_token") or os.environ.get(provider.get("env_key", ""))
    if bearer:
        headers["Authorization"] = "Bearer " + bearer
    if not bearer and not any(k.lower() == "authorization" for k in headers):
        raise ValueError("Configured provider has no explicit bearer credential; auth.json is not read")
    return model, base_url, headers


def inspect_rollouts(probe_home: Path) -> tuple[list[dict], list[dict]]:
    usage = []
    reroutes = []
    for path in (probe_home / "sessions").rglob("*.jsonl"):
        with path.open() as stream:
            for line in stream:
                try:
                    row = json.loads(line)
                except ValueError:
                    continue
                payload = row.get("payload", {})
                if not isinstance(payload, dict):
                    continue
                if row.get("type") == "token_usage_record":
                    usage.append({key: payload.get(key) for key in
                                  ("response_id", "session_id", "thread_id", "turn_id", "usage")})
                if row.get("type") == "event_msg" and payload.get("type") == "model_reroute":
                    reroutes.append({key: payload.get(key) for key in ("from_model", "to_model", "reason")})
    return usage, reroutes


def run_probe(args: argparse.Namespace) -> dict:
    model, base_url, headers = load_provider(args.config)
    binary = find_codex(args.codex)
    local_token = secrets.token_urlsafe(32)
    relay = ProbeRelay(base_url, headers, local_token)
    relay_thread = threading.Thread(target=relay.serve_forever, daemon=True)
    relay_thread.start()
    started = time.monotonic()
    print(f"Starting isolated model probe for {model}; upstream credentials remain in memory.", flush=True)
    try:
        with tempfile.TemporaryDirectory(prefix="wtm-model-probe-") as temporary:
            probe_home = Path(temporary) / "codex"
            workspace = Path(temporary) / "workspace"
            probe_home.mkdir(mode=0o700)
            workspace.mkdir(mode=0o700)
            config = (
                f"model = {json.dumps(model)}\nmodel_provider = \"model_probe\"\n"
                "approval_policy = \"never\"\nsandbox_mode = \"read-only\"\n"
                "project_doc_max_bytes = 0\n"
                "[model_providers.model_probe]\nname = \"Isolated model probe\"\n"
                f"base_url = \"http://127.0.0.1:{relay.server_port}/v1\"\n"
                f"experimental_bearer_token = {json.dumps(local_token)}\n"
                "wire_api = \"responses\"\nrequires_openai_auth = false\nsupports_websockets = false\n"
                "request_max_retries = 0\nstream_max_retries = 0\nstream_idle_timeout_ms = 60000\n"
            )
            config_path = probe_home / "config.toml"
            config_path.write_text(config)
            config_path.chmod(0o600)
            child_env = os.environ.copy()
            child_env["CODEX_HOME"] = str(probe_home)
            # Avoid unrelated credentials or sessions from the caller's environment.
            for key in ("OPENAI_API_KEY", "CODEX_API_KEY", "CODEX_SESSION_ROOT", "CODEX_SESSION_INDEX"):
                child_env.pop(key, None)
            command = [binary, "exec", "--sandbox", "read-only", "--skip-git-repo-check",
                       "--json", "--color", "never", "-C", str(workspace),
                       "Reply with exactly MODEL_PROBE_OK. Do not use any tools."]
            process = subprocess.Popen(command, env=child_env, stdout=subprocess.PIPE, stderr=subprocess.PIPE)
            timed_out = False
            while True:
                try:
                    stdout, _stderr = process.communicate(timeout=5)
                    break
                except subprocess.TimeoutExpired:
                    elapsed = time.monotonic() - started
                    rss = resource.getrusage(resource.RUSAGE_SELF).ru_maxrss / 1024
                    print(f"elapsed={elapsed:.0f}s relay_RSS={rss:.1f}MiB captured_requests={len(relay.evidence)}", flush=True)
                    if elapsed >= args.timeout:
                        timed_out = True
                        process.terminate()
                        try:
                            stdout, _stderr = process.communicate(timeout=5)
                        except subprocess.TimeoutExpired:
                            process.kill()
                            stdout, _stderr = process.communicate()
                        break
            usage, reroutes = inspect_rollouts(probe_home)
            records = [entry.report() for entry in relay.evidence]
            for record in records:
                matching = [row for row in usage if row["response_id"] and row["response_id"] == record["responseId"]]
                record["matchingTokenRecords"] = len(matching)
            client_events = collections.Counter()
            for line in stdout.splitlines():
                try:
                    event = json.loads(line)
                    if isinstance(event.get("type"), str):
                        client_events[event["type"]] += 1
                except (ValueError, AttributeError):
                    pass
            return {
                "schemaVersion": 1,
                "observedAt": dt.datetime.now(dt.timezone.utc).isoformat(),
                "source": "isolated_codex_provider_probe",
                "requestedModel": model,
                "clientExitCode": process.returncode,
                "timedOut": timed_out,
                "elapsedSeconds": round(time.monotonic() - started, 2),
                "records": records,
                "tokenRecords": usage,
                "reroutes": reroutes,
                "clientEventTypes": dict(client_events),
                "errors": relay.errors,
                "limitations": ["Server-declared identifiers cannot independently prove backend model identity.",
                                "Body and header models are provider declarations, not identity attestation.",
                                "This probe exercises the configured provider, not an already-running desktop conversation."],
            }
    finally:
        relay.shutdown()
        relay.server_close()


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--config", type=Path,
                        default=Path(os.environ.get("CODEX_HOME", str(Path.home() / ".codex"))) / "config.toml")
    parser.add_argument("--codex")
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--timeout", type=float, default=90)
    args = parser.parse_args()
    try:
        report = run_probe(args)
    except (ValueError, OSError, tomllib.TOMLDecodeError) as exc:
        # Never print exceptions which might contain a credential or upstream body.
        report = {"schemaVersion": 1, "probeError": type(exc).__name__, "records": []}
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(report, indent=2, ensure_ascii=False) + "\n")
    args.output.chmod(0o600)
    print(f"Report saved to {args.output}", flush=True)
    for record in report.get("records", []):
        print(json.dumps({key: record[key] for key in ("requestedModel", "httpHeaderModels", "eventHeaderModels",
                                                       "responseBodyModels", "verdict", "matchingTokenRecords")}), flush=True)
    return 0 if report.get("clientExitCode") == 0 and report.get("records") else 1


if __name__ == "__main__":
    raise SystemExit(main())
