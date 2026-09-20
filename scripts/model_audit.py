#!/usr/bin/env python3
"""Collect model declarations from ordinary Codex HTTP Responses calls.

The optional relay binds to loopback, forwards client-supplied authentication to
one fixed HTTPS provider, and persists only allowlisted model metadata. It never
reads auth.json. Use `run` for process-local Codex overrides or `serve` for a
client configured to use http://127.0.0.1:4319/v1. See docs/model-audit.md.
"""
from __future__ import annotations

import argparse
import datetime as dt
import http.client
import http.server
import json
import os
from pathlib import Path
import signal
import socket
import subprocess
import sys
import threading
import time
import tomllib
from typing import cast
import urllib.error
import urllib.parse
import urllib.request
import uuid

from model_evidence import Evidence, ResponseObserver
from model_probe import NoRedirect, find_codex

MAX_REQUEST_BYTES = 32 * 1024 * 1024
MAX_FILE_BYTES = 8 * 1024 * 1024
HOP_HEADERS = {"host", "connection", "proxy-connection", "keep-alive", "transfer-encoding",
               "te", "trailer", "upgrade", "proxy-authorization", "proxy-authenticate"}
ROUTES = {"/v1/responses": {"POST"}, "/v1/responses/compact": {"POST"}, "/v1/models": {"GET"}}
OFFICIAL_UPSTREAM = "https://chatgpt.com/backend-api/codex"


def codex_home() -> Path:
    return Path(os.environ.get("CODEX_HOME", str(Path.home() / ".codex")))


def audit_directory() -> Path:
    return Path(os.environ.get("CODEX_MODEL_AUDIT_DIR", str(codex_home() / "model-audit")))


def validate_upstream(value: str, *, allow_http: bool = False) -> str:
    url = urllib.parse.urlsplit(value)
    if (url.scheme != "https" and not (allow_http and url.scheme == "http" and url.hostname == "127.0.0.1")):
        raise ValueError("Upstream must use HTTPS")
    if (not url.hostname or url.username is not None or url.password is not None or url.query
            or url.fragment or any(c.isspace() for c in value)):
        raise ValueError("Upstream must be a base URL without credentials, query or fragment")
    if url.scheme == "https" and url.hostname in {"localhost", "127.0.0.1", "::1"}:
        raise ValueError("Upstream must not point back to the local relay")
    _ = url.port  # Validate malformed ports before any socket is opened.
    return value.rstrip("/")


class MetadataWriter:
    """Single-process append/rotation; unique filenames avoid cross-process races."""
    def __init__(self, directory: Path):
        directory.mkdir(parents=True, exist_ok=True, mode=0o700)
        self.directory = directory
        self.lock = threading.Lock()
        self.path: Path | None = None
        self.bytes_written = 0
        self.records = 0
        self.errors = 0

    def append(self, record: dict) -> None:
        data = (json.dumps(record, separators=(",", ":"), ensure_ascii=True) + "\n").encode()
        with self.lock:
            try:
                if self.path is None or self.bytes_written + len(data) > MAX_FILE_BYTES:
                    stamp = dt.datetime.now(dt.timezone.utc).strftime("%Y%m%dT%H%M%S")
                    self.path = self.directory / f"capture-{stamp}-{uuid.uuid4().hex}.jsonl"
                    fd = os.open(self.path, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
                    self.bytes_written = 0
                else:
                    fd = os.open(self.path, os.O_WRONLY | os.O_APPEND | getattr(os, "O_NOFOLLOW", 0))
                with os.fdopen(fd, "wb") as stream:
                    stream.write(data)
                self.bytes_written += len(data)
                self.records += 1
            except OSError:
                self.errors += 1
                self.path = None  # A future observation may recover after storage becomes writable.
                # Never include exception text (paths, provider content or tokens).
                print("Model audit: metadata write failed; request forwarding continues.", file=sys.stderr, flush=True)


class AuditRelay(http.server.ThreadingHTTPServer):
    daemon_threads = True
    request_queue_size = 32

    def __init__(self, upstream: str, writer: MetadataWriter, port: int = 0, *, allow_http: bool = False):
        self.base_url = validate_upstream(upstream, allow_http=allow_http)
        parsed = urllib.parse.urlsplit(self.base_url)
        self.origin = f"{parsed.scheme}://{parsed.netloc}"
        self.writer = writer
        self.upstream = urllib.request.build_opener(NoRedirect())
        self.slots = threading.BoundedSemaphore(16)
        self.requests = 0
        self.forward_errors = 0
        self.capture_lock = threading.Lock()
        self.capture_enabled = True
        self.capture_epoch = 0
        self.active_requests = 0
        super().__init__(("127.0.0.1", port), AuditHandler)

    def set_capture(self, enabled: bool) -> None:
        # An acknowledged pause excludes even responses that were already streaming.
        with self.capture_lock:
            if enabled != self.capture_enabled:
                self.capture_epoch += 1
            self.capture_enabled = enabled

    def capture_token(self) -> int | None:
        with self.capture_lock:
            return self.capture_epoch if self.capture_enabled else None

    def record(self, evidence: Evidence, transport: str, status: int | None, token: int | None) -> None:
        record = evidence.report()
        record.update(schemaVersion=1, observedAt=dt.datetime.now(dt.timezone.utc).isoformat(),
                      upstream=self.origin, transport=transport, httpStatus=status)
        with self.capture_lock:
            if token is not None and self.capture_enabled and token == self.capture_epoch:
                self.writer.append(record)

    def record_failure(self, requested: object, issue: str, token: int | None, status: int | None = None) -> None:
        if token is None:
            return
        evidence = Evidence(requested, {})
        evidence.issue(issue)
        self.record(evidence, "http_json", status, token)

    def process_request(self, request, client_address):
        # Give a just-finished connection time to release its slot before rejecting
        # the next request from the same client. The active-connection cap stays 16.
        if not self.slots.acquire(timeout=0.05):
            try:
                cast(socket.socket, request).sendall(b"HTTP/1.1 503 Service Unavailable\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
            finally:
                self.shutdown_request(request)
            return
        try:
            super().process_request(request, client_address)
        except BaseException:
            self.slots.release()
            raise

    def process_request_thread(self, request, client_address):
        try:
            with self.capture_lock:
                self.active_requests += 1
            super().process_request_thread(request, client_address)
        finally:
            with self.capture_lock:
                self.active_requests -= 1
            self.slots.release()

    def handle_error(self, request, client_address):
        # Base implementation prints exceptions; those may contain private data.
        self.forward_errors += 1


class AuditHandler(http.server.BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"

    @property
    def relay(self) -> AuditRelay:
        return cast(AuditRelay, self.server)

    def setup(self):
        super().setup()
        self.connection.settimeout(30)

    def log_message(self, format, *args):
        pass

    def do_GET(self):
        self.forward()

    def do_POST(self):
        self.forward()

    def fail(self, status: int, message: str):
        data = json.dumps({"error": {"message": message, "type": "model_audit_relay"}}).encode()
        self.send_response(status)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(data)))
        self.send_header("Connection", "close")
        self.end_headers()
        self.wfile.write(data)
        self.close_connection = True

    def forward(self):
        expected_hosts = {f"127.0.0.1:{self.relay.server_port}", f"localhost:{self.relay.server_port}"}
        if self.headers.get("Host", "").lower() not in expected_hosts or self.headers.get("Origin") is not None:
            self.fail(403, "Only local non-browser clients are supported")
            return
        route = urllib.parse.urlsplit(self.path)
        if route.scheme or route.netloc or self.command not in ROUTES.get(route.path, set()):
            self.fail(404, "Unsupported Responses route")
            return
        if self.headers.get("Upgrade"):
            self.fail(426, "Disable WebSocket transport for HTTP model auditing")
            return
        if not any(self.headers.get(key) for key in ("Authorization", "x-api-key", "api-key")):
            self.fail(401, "Client-supplied upstream authentication is required")
            return
        if self.headers.get("Transfer-Encoding") or len(self.headers.get_all("Content-Length", [])) > 1:
            self.fail(400, "A single Content-Length is required")
            return
        try:
            length = int(self.headers.get("Content-Length", "0"))
        except ValueError:
            self.fail(400, "Invalid Content-Length")
            return
        if not 0 <= length <= MAX_REQUEST_BYTES:
            self.fail(413, "Request exceeds the 32 MiB relay limit")
            return
        try:
            body = self.rfile.read(length) if length else None
        except (OSError, TimeoutError):
            self.fail(408, "Request body timed out")
            return
        if body is not None and len(body) != length:
            self.fail(400, "Incomplete request body")
            return
        if self.headers.get("Content-Encoding", "identity").lower() not in {"", "identity"}:
            self.fail(415, "Disable features.enable_request_compression for HTTP model auditing")
            return
        requested = None
        capture_token = self.relay.capture_token()
        if route.path == "/v1/responses" and capture_token is not None:
            try:
                payload = json.loads(body or b"")
                if not isinstance(payload, dict):
                    raise ValueError("Request must be an object")
                requested = payload.get("model")
                del payload
            except (ValueError, UnicodeError, RecursionError):
                self.fail(400, "Invalid Responses JSON")
                return
        connection_headers = {v.strip().lower() for v in self.headers.get("Connection", "").split(",")}
        excluded = HOP_HEADERS | connection_headers | {"content-length", "accept-encoding"}
        headers = {k: v for k, v in self.headers.items() if k.lower() not in excluded}
        headers["Accept-Encoding"] = "identity"
        suffix = route.path.removeprefix("/v1")
        url = self.relay.base_url + suffix + ("?" + route.query if route.query else "")
        request = urllib.request.Request(url, data=body, headers=headers, method=self.command)
        self.relay.requests += 1
        try:
            upstream = self.relay.upstream.open(request, timeout=300)
        except urllib.error.HTTPError as exc:
            # Preserve 401/429/status bodies so Codex can refresh auth/retry normally.
            if 300 <= exc.code < 400:
                exc.close()
                if route.path == "/v1/responses":
                    self.relay.record_failure(requested, "upstream_redirect", capture_token, exc.code)
                self.fail(502, "Upstream redirects are not followed")
                return
            upstream = exc
        except (urllib.error.URLError, OSError, TimeoutError, http.client.HTTPException):
            self.relay.forward_errors += 1
            if route.path == "/v1/responses":
                self.relay.record_failure(requested, "upstream_connection_failed", capture_token)
            self.fail(502, "Upstream connection failed")
            return
        evidence = None
        observer = None
        with upstream:
            content_type = upstream.headers.get("Content-Type", "application/json")
            status_code = cast(int, upstream.getcode())
            if route.path == "/v1/responses" and capture_token is not None:
                evidence = Evidence(requested, upstream.headers.items())
                if not 200 <= status_code < 300:
                    evidence.issue("upstream_http_error")
                observer = ResponseObserver(evidence, content_type)
                if upstream.headers.get("Content-Encoding", "identity").lower() not in {"", "identity"}:
                    evidence.issue("unsupported_encoding")
                    observer.disabled = True
            self.send_response(status_code)
            excluded_response = HOP_HEADERS | {"content-length", "access-control-allow-origin"}
            excluded_response |= {v.strip().lower() for v in upstream.headers.get("Connection", "").split(",")}
            for key, value in upstream.headers.items():
                if key.lower() not in excluded_response:
                    self.send_header(key, value)
            self.send_header("Connection", "close")
            self.end_headers()
            self.close_connection = True
            try:
                while chunk := upstream.read1(64 * 1024):
                    if observer and capture_token != self.relay.capture_token():
                        observer = None
                        evidence = None
                    if observer:
                        observer.feed(chunk)
                    self.wfile.write(chunk)
                    self.wfile.flush()
            except (OSError, TimeoutError, http.client.HTTPException):
                self.relay.forward_errors += 1
                if evidence:
                    evidence.issue("stream_interrupted")
            finally:
                if observer and evidence:
                    observer.finish()
                    self.relay.record(evidence, "http_sse" if observer.sse else "http_json", status_code, capture_token)


def selected_profile(config: dict, args: list[str]) -> str | None:
    profile = config.get("profile")
    for i, value in enumerate(args):
        if value in {"-p", "--profile"} and i + 1 < len(args):
            profile = args[i + 1]
        elif value.startswith("--profile="):
            profile = value.split("=", 1)[1]
        elif value.startswith("-p") and not value.startswith("--"):
            profile = value[2:]
    if profile is not None and (not isinstance(profile, str) or not profile or
            any(c not in "abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789_-" for c in profile)):
        raise ValueError("Unsupported profile name")
    return profile


def merge_config(base: dict, overlay: dict) -> dict:
    merged = dict(base)
    for key, value in overlay.items():
        merged[key] = merge_config(merged[key], value) if isinstance(merged.get(key), dict) and isinstance(value, dict) else value
    return merged


def configured_provider(config: dict, args: list[str], home: Path | None = None) -> tuple[str, str]:
    profile = selected_profile(config, args)
    effective = merge_config(config, config.get("profiles", {}).get(profile, {}))
    if profile and home is not None:
        profile_file = home / f"{profile}.config.toml"
        if profile_file.exists():
            effective = merge_config(effective, tomllib.loads(profile_file.read_text()))
    name = effective.get("model_provider", "openai")
    provider = effective.get("model_providers", {}).get(name, {})
    if name == "openai":
        raise ValueError("Configured mode requires a named custom provider; use official mode for subscription login")
    if provider.get("wire_api", "responses") != "responses":
        raise ValueError("Only Responses providers are supported")
    base = provider.get("base_url")
    if not isinstance(base, str):
        raise ValueError("Configured mode needs an explicit provider base_url; use --mode official for subscription login")
    if not isinstance(name, str) or not name or any(c not in "abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789_-" for c in name):
        raise ValueError("Unsupported provider identifier")
    return name, validate_upstream(base)


def check_overrides(args: list[str]) -> None:
    # A caller can still use -m, sandbox and normal Codex options. Routing overrides
    # would bypass the collector or accidentally keep a different provider active.
    for i, value in enumerate(args):
        if value in {"-c", "--config"} and i + 1 < len(args):
            candidate = args[i + 1]
        elif value.startswith("--config="):
            candidate = value.split("=", 1)[1]
        elif value.startswith("-c") and not value.startswith("--"):
            candidate = value[2:]
        else:
            continue
        key = candidate.split("=", 1)[0].strip().replace('"', "").replace("'", "")
        if (key in {"model_provider", "model_providers", "chatgpt_base_url", "openai_base_url",
                    "profile", "profiles", "features", "features.enable_request_compression"}
                or key.startswith(("model_providers.", "profiles."))):
            raise ValueError("Pass model routing through --mode, not Codex -c overrides")
    for i, value in enumerate(args):
        features = value.split("=", 1)[1] if value.startswith("--enable=") else (
            args[i + 1] if value == "--enable" and i + 1 < len(args) else "")
        if "enable_request_compression" in features.split(","):
            raise ValueError("Request compression is incompatible with HTTP auditing")


def codex_command(binary: str, provider: str, port: int, args: list[str], mode: str) -> list[str]:
    check_overrides(args)
    # Built-in OpenAI capabilities cannot be replaced by model_providers.openai.
    # A fresh custom provider preserves OAuth auth management while forcing SSE.
    provider = f"wtm_official_{uuid.uuid4().hex}" if mode == "official" else provider
    prefix = f"model_providers.{provider}"
    overrides = {
        "model_provider": provider,
        f"{prefix}.base_url": f"http://127.0.0.1:{port}/v1",
        f"{prefix}.supports_websockets": False,
        "features.enable_request_compression": False,
    }
    if mode == "official":
        overrides.update({f"{prefix}.name": "OpenAI", f"{prefix}.wire_api": "responses",
                          f"{prefix}.requires_openai_auth": True})
    command = [binary]
    for key, value in overrides.items():
        command.extend(["-c", f"{key}={json.dumps(value)}"])
    return command + args


def run(args: argparse.Namespace) -> int:
    child_args = args.codex_args
    if child_args and child_args[0] == "--":
        child_args = child_args[1:]
    check_overrides(child_args)
    if args.mode == "official":
        provider, upstream = "openai", OFFICIAL_UPSTREAM
    else:
        config_file = codex_home() / "config.toml"
        config = tomllib.loads(config_file.read_text())
        provider, upstream = configured_provider(config, child_args, codex_home())
    binary = find_codex(args.codex)
    writer = MetadataWriter(args.output_dir)
    relay = AuditRelay(upstream, writer)
    thread = threading.Thread(target=relay.serve_forever, daemon=True)
    thread.start()
    process = None
    print(f"Model audit: forwarding to {relay.origin}; metadata directory: {writer.directory}", file=sys.stderr, flush=True)
    try:
        command = codex_command(binary, provider, relay.server_port, child_args, args.mode)
        child_env = os.environ.copy()
        no_proxy = ",".join(filter(None, [child_env.get("NO_PROXY"), child_env.get("no_proxy"), "127.0.0.1,localhost"]))
        child_env.update(NO_PROXY=no_proxy, no_proxy=no_proxy)
        process = subprocess.Popen(command, env=child_env)
        try:
            return process.wait()
        except KeyboardInterrupt:
            # Ctrl-C also reaches the client in the terminal process group.
            try:
                return process.wait(timeout=5)
            except subprocess.TimeoutExpired:
                process.terminate()
                try:
                    return process.wait(timeout=5)
                except subprocess.TimeoutExpired:
                    process.kill()
                    return process.wait()
    finally:
        relay.shutdown()
        relay.server_close()
        print(f"Model audit: {writer.records} observations, {writer.errors} write errors.", file=sys.stderr, flush=True)


def serve(args: argparse.Namespace) -> int:
    writer = MetadataWriter(args.output_dir)
    relay = AuditRelay(args.upstream, writer, args.port)
    done = threading.Event()
    for sig in (signal.SIGINT, signal.SIGTERM):
        signal.signal(sig, lambda *_: done.set())
    thread = threading.Thread(target=relay.serve_forever, daemon=True)
    thread.start()
    print(f"Model audit listening: http://127.0.0.1:{relay.server_port}/v1", flush=True)
    print(f"Fixed upstream: {relay.origin}; metadata: {writer.directory}", flush=True)
    started = time.monotonic()
    try:
        while not done.wait(30):
            rss = ""
            try:
                import resource
                rss = f" peak_RSS={resource.getrusage(resource.RUSAGE_SELF).ru_maxrss / 1024:.1f}MiB"
            except ImportError:
                pass
            print(f"elapsed={time.monotonic() - started:.0f}s requests={relay.requests} "
                  f"observations={writer.records} write_errors={writer.errors}{rss}", flush=True)
    finally:
        relay.shutdown()
        relay.server_close()
    return 0


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest="command", required=True)
    run_parser = commands.add_parser("run", help="Launch Codex with temporary process-only routing")
    run_parser.add_argument("--mode", choices=("configured", "official"), default="configured")
    run_parser.add_argument("--codex", help="Codex executable (otherwise PATH or desktop bundled Codex)")
    run_parser.add_argument("--output-dir", type=Path, default=audit_directory())
    run_parser.add_argument("codex_args", nargs=argparse.REMAINDER)
    serve_parser = commands.add_parser("serve", help="Run a relay for an explicitly configured desktop client")
    serve_parser.add_argument("--upstream", required=True, help="Complete HTTPS provider base URL, including /v1 if needed")
    serve_parser.add_argument("--port", type=int, default=4319)
    serve_parser.add_argument("--output-dir", type=Path, default=audit_directory())
    args = parser.parse_args()
    try:
        return run(args) if args.command == "run" else serve(args)
    except (ValueError, OSError, tomllib.TOMLDecodeError) as exc:
        # Deliberately don't print configuration/transport exception details.
        print(f"Model audit could not start ({type(exc).__name__}). Check the command and docs/model-audit.md.", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
