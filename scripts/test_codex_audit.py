"""Exercise the installed Codex client against a synthetic local upstream only.

Skipped on CI machines without Codex. No real credentials, conversation logs or
external model service are used. A temporary home is removed after the test.
"""
import http.server
import json
import os
from pathlib import Path
import subprocess
import tempfile
import threading
import unittest
import urllib.request

from model_audit import AuditRelay, MetadataWriter, codex_command
from model_probe import NoRedirect, find_codex, inspect_rollouts

try:
    CODEX = find_codex(None)
except ValueError:
    CODEX = None


@unittest.skipUnless(CODEX, "Codex executable is not installed")
class CodexIntegrationTests(unittest.TestCase):
    def test_codex_response_exactly_joins_audit_and_token_record(self):
        assert CODEX is not None
        response_id = "resp-offline-audit-integration"
        requested = "gpt-5.4"
        returned = "gpt-5.4-offline-fixture"
        response = {"id": response_id, "object": "response", "model": returned,
                    "status": "completed", "output": [],
                    "usage": {"input_tokens": 12, "output_tokens": 3, "total_tokens": 15,
                              "input_tokens_details": {"cached_tokens": 4},
                              "output_tokens_details": {"reasoning_tokens": 0}}}
        message = {"id": "msg-offline", "type": "message", "role": "assistant", "status": "completed",
                   "content": [{"type": "output_text", "text": "OFFLINE_AUDIT_OK", "annotations": []}]}
        received = []

        class Fixture(http.server.BaseHTTPRequestHandler):
            def log_message(self, format, *args):
                pass

            def do_POST(self):
                received.append(json.loads(self.rfile.read(int(self.headers["Content-Length"]))))
                events = [
                    {"type": "response.created", "response": {**response, "status": "in_progress"}},
                    {"type": "response.output_item.added", "output_index": 0, "item": message},
                    {"type": "response.output_item.done", "output_index": 0, "item": message},
                    {"type": "response.completed", "response": {**response, "output": [message]}},
                ]
                body = "".join("data: " + json.dumps(e) + "\n\n" for e in events).encode()
                self.send_response(200)
                self.send_header("Content-Type", "text/event-stream")
                self.send_header("Content-Length", str(len(body)))
                self.end_headers()
                self.wfile.write(body)

        with tempfile.TemporaryDirectory(prefix="wtm-offline-codex-") as temp:
            root = Path(temp)
            home = root / "codex"
            home.mkdir()
            workspace = root / "workspace"
            workspace.mkdir()
            config = ('model = "gpt-5.4"\nmodel_provider = "fixture"\nproject_doc_max_bytes = 0\n'
                      'approval_policy = "never"\nsandbox_mode = "read-only"\n'
                      '[model_providers.fixture]\nname = "Offline fixture"\nwire_api = "responses"\n'
                      'experimental_bearer_token = "offline-dummy-token"\nrequest_max_retries = 0\n'
                      'stream_max_retries = 0\nstream_idle_timeout_ms = 10000\n')
            (home / "config.toml").write_text(config)
            (home / "work.config.toml").write_text('model_provider = "not-chosen"\n[features]\nenable_request_compression = true\n')
            writer = MetadataWriter(home / "model-audit")
            fixture = http.server.ThreadingHTTPServer(("127.0.0.1", 0), Fixture)
            relay = AuditRelay(f"http://127.0.0.1:{fixture.server_port}/v1", writer, allow_http=True)
            relay.upstream = urllib.request.build_opener(urllib.request.ProxyHandler({}), NoRedirect())
            for server in (fixture, relay):
                threading.Thread(target=server.serve_forever, kwargs={"poll_interval": 0.02}, daemon=True).start()
            try:
                env = os.environ.copy()
                for key in ("OPENAI_API_KEY", "CODEX_API_KEY", "CODEX_SESSION_ROOT", "CODEX_SESSION_INDEX"):
                    env.pop(key, None)
                env.update(CODEX_HOME=str(home), NO_PROXY="127.0.0.1,localhost", no_proxy="127.0.0.1,localhost")
                command = codex_command(CODEX, "fixture", relay.server_port,
                                        ["exec", "--profile", "work", "--skip-git-repo-check", "--json", "--color", "never", "-C", str(workspace),
                                         "Reply with OFFLINE_AUDIT_OK. Do not use tools."], "configured")
                result = subprocess.run(command, env=env, capture_output=True, timeout=30)
                self.assertEqual(result.returncode, 0, result.stderr.decode(errors="replace")[-2000:])
                self.assertEqual(len(received), 1)
                self.assertEqual(received[0]["model"], requested)
                usage, _ = inspect_rollouts(home)
                self.assertEqual(len(usage), 1)
                records = [json.loads(line) for path in writer.directory.glob("*.jsonl") for line in path.read_text().splitlines()]
                self.assertEqual(len(records), 1)
                self.assertEqual(records[0]["responseId"], usage[0]["response_id"])
                self.assertEqual(records[0]["reportedModel"], returned)
                self.assertEqual(records[0]["verdict"], "different")
                self.assertNotIn("OFFLINE_AUDIT_OK", json.dumps(records))
                self.assertNotIn("offline-dummy-token", json.dumps(records))
                self.assertEqual((home / "config.toml").read_text(), config)
            finally:
                for server in (relay, fixture):
                    server.shutdown()
                    server.server_close()


if __name__ == "__main__":
    unittest.main()
