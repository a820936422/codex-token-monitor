#!/usr/bin/env python3
"""Private Unix-socket control for the desktop collector (no HTTP admin routes).

A short controller lease disables capture after a monitor crash. The forwarding
bridge remains available for desktop clients that cached its URL. Explicit stop
is separate from pausing collection and drains accepted HTTP requests.
"""
from __future__ import annotations

import argparse
import fcntl
import json
import os
from pathlib import Path
import resource
import signal
import socket
import threading
import time

from model_audit import AuditRelay, MetadataWriter

PROTOCOL = 1
LEASE_SECONDS = 8
MAX_CONTROL_BYTES = 4096


class CollectorService:
    def __init__(self, relay: AuditRelay):
        self.relay = relay
        self.relay.set_capture(False)
        self.lease_until = 0.0
        self.done = False

    def expire_lease(self) -> None:
        if self.relay.capture_enabled and time.monotonic() >= self.lease_until:
            self.relay.set_capture(False)

    def status(self) -> dict:
        self.expire_lease()
        usage = resource.getrusage(resource.RUSAGE_SELF)
        rss = usage.ru_maxrss * 1024
        try:
            rss = int(Path('/proc/self/statm').read_text().split()[1]) * os.sysconf('SC_PAGE_SIZE')
        except (OSError, ValueError, IndexError):
            pass
        return {"protocolVersion": PROTOCOL, "pid": os.getpid(), "port": self.relay.server_port,
                "upstream": self.relay.base_url, "outputDir": str(self.relay.writer.directory),
                "enabled": self.relay.capture_enabled, "requests": self.relay.requests,
                "activeRequests": self.relay.active_requests, "observations": self.relay.writer.records,
                "writeErrors": self.relay.writer.errors, "forwardErrors": self.relay.forward_errors,
                "rssBytes": rss, "cpuSeconds": usage.ru_utime + usage.ru_stime}

    def command(self, request: dict) -> dict:
        self.expire_lease()
        command = request.get("command")
        if command == "capture":
            enabled = request.get("enabled")
            if not isinstance(enabled, bool):
                return {"error": "invalid_command"}
            self.lease_until = time.monotonic() + LEASE_SECONDS if enabled else 0.0
            self.relay.set_capture(enabled)
        elif command == "heartbeat":
            if self.relay.capture_enabled:
                self.lease_until = time.monotonic() + LEASE_SECONDS
        elif command == "stop":
            if self.relay.active_requests:
                return {"error": "requests_active"}
            self.relay.set_capture(False)
            self.done = True
        elif command != "status":
            return {"error": "invalid_command"}
        return self.status()


def serve_control(service: CollectorService, listener: socket.socket) -> None:
    listener.settimeout(0.5)
    while not service.done:
        service.expire_lease()
        try:
            connection, _ = listener.accept()
        except socket.timeout:
            continue
        with connection:
            connection.settimeout(1)
            try:
                data = bytearray()
                while b'\n' not in data and len(data) <= MAX_CONTROL_BYTES:
                    chunk = connection.recv(min(1024, MAX_CONTROL_BYTES + 1 - len(data)))
                    if not chunk:
                        break
                    data.extend(chunk)
                if len(data) > MAX_CONTROL_BYTES or not data.endswith(b'\n'):
                    response = {"error": "invalid_command"}
                else:
                    request = json.loads(data)
                    response = service.command(request) if isinstance(request, dict) else {"error": "invalid_command"}
                connection.sendall(json.dumps(response).encode() + b'\n')
            except (ValueError, OSError, RecursionError):
                pass  # Never print control payloads or exception details.


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--socket', type=Path, required=True)
    parser.add_argument('--upstream', required=True)
    parser.add_argument('--output-dir', type=Path, required=True)
    parser.add_argument('--port', type=int, default=0)
    args = parser.parse_args()
    os.umask(0o077)
    args.socket.parent.mkdir(parents=True, exist_ok=True, mode=0o700)
    lock = open(args.socket.with_suffix('.lock'), 'a')
    try:
        fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
    except OSError:
        return 2
    relay = None
    serving = False
    try:
        relay = AuditRelay(args.upstream, MetadataWriter(args.output_dir), args.port)
        relay.daemon_threads = False
        service = CollectorService(relay)
        args.socket.unlink(missing_ok=True)
        with socket.socket(socket.AF_UNIX, socket.SOCK_STREAM) as listener:
            listener.bind(str(args.socket))
            args.socket.chmod(0o600)
            listener.listen(8)
            for sig in (signal.SIGINT, signal.SIGTERM):
                signal.signal(sig, lambda *_: setattr(service, 'done', True))
            threading.Thread(target=relay.serve_forever, daemon=True).start()
            serving = True
            serve_control(service, listener)
    except (OSError, ValueError):
        return 1
    finally:
        if relay:
            relay.set_capture(False)
            if serving:
                relay.shutdown()
            relay.server_close()
        args.socket.unlink(missing_ok=True)
        lock.close()
    return 0


if __name__ == '__main__':
    raise SystemExit(main())
