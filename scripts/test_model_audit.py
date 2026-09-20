"""Offline tests for declaration capture, real HTTP forwarding and Codex routing."""
import concurrent.futures
import http.client
import http.server
import json
import os
from pathlib import Path
import tempfile
import threading
import tomllib
import unittest
from unittest import mock
import urllib.error
import urllib.request

import model_audit
from model_audit import AuditRelay, MetadataWriter, codex_command, configured_provider, validate_upstream
from model_evidence import Evidence, MAX_EVENT_BYTES, ResponseObserver


def event(model="requested", response_id="resp-test", kind="response.completed"):
    return {"type": kind, "response": {"id": response_id, "model": model,
            "output": [{"text": "private-response-body"}]}}


def sse(*events):
    return b"".join(("data: " + json.dumps(e) + "\r\n\r\n").encode() for e in events)


class ObservationTests(unittest.TestCase):
    def observe(self, payload, content_type="text/event-stream", chunk_size=7):
        evidence = Evidence("requested", {})
        observer = ResponseObserver(evidence, content_type)
        for i in range(0, len(payload), chunk_size):
            observer.feed(payload[i:i + chunk_size])
        observer.finish()
        return evidence.report()

    def test_body_is_sufficient_for_declaration_match(self):
        result = self.observe(sse(event()))
        self.assertEqual(result["verdict"], "match")
        self.assertEqual(result["evidenceSources"], ["response_body"])
        self.assertNotIn("private-response-body", json.dumps(result))

    def test_terminal_wins_without_hiding_conflict(self):
        result = self.observe(sse(event("first", kind="response.created"), event("final")))
        self.assertEqual(result["reportedModel"], "final")
        self.assertEqual(result["responseBodyModels"], ["first", "final"])
        self.assertEqual(result["verdict"], "conflict")

    def test_json_response_and_strict_version_difference(self):
        result = self.observe(json.dumps({"id": "resp-json", "model": "requested-2026-01-01",
                              "object": "response", "status": "completed"}).encode(), "application/json")
        self.assertEqual(result["verdict"], "different")
        self.assertEqual(result["responseId"], "resp-json")
        self.assertTrue(result["completed"])

    def test_sse_event_name_multiline_and_no_final_blank_line(self):
        payload = b'event: response.completed\ndata: {"response":\ndata: {"id":"resp-last", "model":"requested"}}'
        result = self.observe(payload)
        self.assertEqual(result["verdict"], "match")
        self.assertTrue(result["completed"])

    def test_response_id_conflict_cannot_join(self):
        result = self.observe(sse(event(response_id="resp-a"), event(response_id="resp-b")))
        self.assertIsNone(result["responseId"])
        self.assertEqual(result["verdict"], "unknown")
        self.assertIn("response_id_conflict", result["captureIssues"])

    def test_malformed_event_prevents_false_match(self):
        result = self.observe(b"data: not json\n\n" + sse(event()))
        self.assertEqual(result["verdict"], "unknown")
        self.assertIn("invalid_json", result["captureIssues"])

    def test_oversized_event_is_bounded_and_later_metadata_survives(self):
        evidence = Evidence("requested", {})
        observer = ResponseObserver(evidence, "text/event-stream")
        observer.feed(b"data: " + b"x" * (MAX_EVENT_BYTES + 10))
        self.assertLessEqual(len(observer.buffer), MAX_EVENT_BYTES)
        observer.feed(b"\n\n" + sse(event()))
        observer.finish()
        result = evidence.report()
        self.assertEqual(result["responseId"], "resp-test")
        self.assertEqual(result["verdict"], "unknown")
        self.assertIn("event_too_large", result["captureIssues"])

    def test_missing_model_is_unknown_and_arbitrary_text_is_not_stored(self):
        evidence = Evidence("requested", {"Authorization": "private-credential"})
        evidence.consume(event("some private text\nnot an identifier"))
        result = evidence.report()
        self.assertEqual(result["verdict"], "unknown")
        self.assertNotIn("private", json.dumps(result))

    def test_model_sets_remain_bounded(self):
        evidence = Evidence("requested", {})
        for i in range(100):
            evidence.consume(event(f"model-{i}"))
        result = evidence.report()
        self.assertEqual(len(result["responseBodyModels"]), 32)
        self.assertIn("too_many_models", result["captureIssues"])

    def test_eof_before_completion_remains_unknown(self):
        result = self.observe(sse(event(kind="response.created")))
        self.assertEqual(result["verdict"], "unknown")
        self.assertFalse(result["completed"])
        self.assertIn("incomplete_response", result["captureIssues"])

    def test_response_id_and_models_are_not_whitespace_normalized(self):
        result = self.observe(sse(event("requested ", "resp-test ")))
        self.assertIsNone(result["responseId"])
        self.assertEqual(result["verdict"], "unknown")
        self.assertIn("invalid_identifier", result["captureIssues"])



class ForwardingTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.writer = MetadataWriter(Path(self.temporary.name) / "audit")
        self.status = 200
        self.body = sse(event())
        self.content_type = "text/event-stream"
        self.encoding = None
        self.model_headers = []
        self.received = []
        case = self

        class Upstream(http.server.BaseHTTPRequestHandler):
            def log_message(self, format, *args):
                pass

            def do_POST(self):
                body = self.rfile.read(int(self.headers.get("Content-Length", "0")))
                case.received.append((self.path, body, dict(self.headers)))
                self.send_response(case.status)
                self.send_header("Content-Type", case.content_type)
                self.send_header("X-Request-Id", "req-test")
                self.send_header("Retry-After", "1")
                if case.encoding:
                    self.send_header("Content-Encoding", case.encoding)
                if case.status == 302:
                    self.send_header("Location", "/redirected")
                for value in case.model_headers:
                    self.send_header("OpenAI-Model", value)
                self.send_header("Content-Length", str(len(case.body)))
                self.end_headers()
                self.wfile.write(case.body)

        self.upstream = http.server.ThreadingHTTPServer(("127.0.0.1", 0), Upstream)
        self.relay = AuditRelay(f"http://127.0.0.1:{self.upstream.server_port}/backend-api/codex",
                                self.writer, allow_http=True)
        self.relay.upstream = urllib.request.build_opener(urllib.request.ProxyHandler({}), model_audit.NoRedirect())
        for server in (self.upstream, self.relay):
            threading.Thread(target=server.serve_forever, kwargs={"poll_interval": 0.02}, daemon=True).start()
            self.addCleanup(server.server_close)
            self.addCleanup(server.shutdown)
        self.client = urllib.request.build_opener(urllib.request.ProxyHandler({}))

    def request(self, extra=None, route="/v1/responses", token="client-secret"):
        headers = {"Content-Type": "application/json", "Chatgpt-Account-Id": "private-account"}
        if token is not None:
            headers["Authorization"] = "Bearer " + token
        headers.update(extra or {})
        request = urllib.request.Request(f"http://127.0.0.1:{self.relay.server_port}" + route,
                                         data=b'{"model":"requested","input":"private-prompt"}', headers=headers)
        try:
            response = self.client.open(request, timeout=3)
        except urllib.error.HTTPError as exc:
            response = exc
        with response:
            return response.status, response.read(), response.headers

    def records(self):
        return [json.loads(line) for p in self.writer.directory.glob("*.jsonl") for line in p.read_text().splitlines()]

    def test_forwards_original_bytes_and_client_oauth_without_persisting_secrets(self):
        status, body, headers = self.request()
        self.assertEqual((status, body), (200, self.body))
        self.assertEqual(headers["x-request-id"], "req-test")
        path, sent, sent_headers = self.received[0]
        self.assertEqual(path, "/backend-api/codex/responses")
        self.assertIn(b"private-prompt", sent)
        self.assertEqual(sent_headers["Authorization"], "Bearer client-secret")
        self.assertEqual(sent_headers["Chatgpt-Account-Id"], "private-account")
        records = self.records()
        self.assertEqual(len(records), 1)
        self.assertEqual(records[0]["verdict"], "match")
        self.assertNotIn("private", json.dumps(records))
        self.assertNotIn("client-secret", json.dumps(records))
        assert self.writer.path is not None
        self.assertEqual(self.writer.path.stat().st_mode & 0o777, 0o600)

    def test_nonstreaming_json_model_difference(self):
        self.content_type = "application/json"
        self.body = b'{"object":"response","id":"resp-json","model":"other","status":"completed"}'
        self.assertEqual(self.request()[0], 200)
        self.assertEqual(self.records()[0]["reportedModel"], "other")
        self.assertEqual(self.records()[0]["verdict"], "different")

    def test_repeated_model_headers_preserve_conflict(self):
        self.model_headers = ["requested", "other"]
        self.assertEqual(self.request()[0], 200)
        self.assertEqual(self.records()[0]["httpHeaderModels"], ["requested", "other"])
        self.assertEqual(self.records()[0]["verdict"], "conflict")

    def test_auth_recovery_and_rate_limit_responses_are_not_masked(self):
        for status in (401, 429, 500):
            self.status = status
            self.body = b'{"error":{"message":"synthetic-error"}}'
            got, body, headers = self.request()
            self.assertEqual((got, body), (status, self.body))
            self.assertEqual(headers["Retry-After"], "1")
        self.assertEqual([r["httpStatus"] for r in self.records()], [401, 429, 500])
        self.assertTrue(all(r["verdict"] == "unknown" for r in self.records()))
        self.assertNotIn("synthetic-error", json.dumps(self.records()))

    def test_failed_connection_records_only_fixed_failure_code(self):
        with mock.patch.object(self.relay.upstream, "open", side_effect=urllib.error.URLError("private-upstream")):
            self.assertEqual(self.request()[0], 502)
        record = self.records()[0]
        self.assertEqual(record["verdict"], "unknown")
        self.assertIsNone(record["responseId"])
        self.assertIsNone(record["httpStatus"])
        self.assertEqual(record["captureIssues"], ["upstream_connection_failed"])
        self.assertNotIn("private-upstream", json.dumps(record))

    def test_redirect_is_not_followed(self):
        self.status = 302
        self.assertEqual(self.request()[0], 502)
        self.assertEqual(len(self.received), 1)

    def test_browser_wrong_host_and_missing_auth_are_rejected(self):
        for headers, token, expected in [({"Origin": "https://example.com"}, "key", 403),
                                          ({"Host": "example.com"}, "key", 403), ({}, None, 401)]:
            self.assertEqual(self.request(headers, token=token)[0], expected)
        self.assertEqual(self.received, [])

    def test_invalid_route_and_compressed_request_are_explicit_errors(self):
        self.assertEqual(self.request(route="/v1/../admin")[0], 404)
        self.assertEqual(self.request({"Content-Encoding": "zstd"})[0], 415)
        self.assertEqual(self.received, [])

    def test_unknown_response_encoding_does_not_break_forwarding(self):
        self.encoding = "gzip"
        self.body = b"compressed-placeholder"
        self.assertEqual(self.request()[1], self.body)
        record = self.records()[0]
        self.assertEqual(record["verdict"], "unknown")
        self.assertIn("unsupported_encoding", record["captureIssues"])

    def test_concurrent_attempts_have_separate_observations(self):
        with concurrent.futures.ThreadPoolExecutor(max_workers=4) as executor:
            statuses = list(executor.map(lambda _: self.request()[0], range(8)))
        self.assertEqual(statuses, [200] * 8)
        self.assertEqual(len(self.records()), 8)

    def test_metadata_write_failure_preserves_response(self):
        with mock.patch("model_audit.os.open", side_effect=OSError("private-path")), mock.patch("sys.stderr") as stderr:
            self.assertEqual(self.request()[1], self.body)
        self.assertEqual(self.writer.errors, 1)
        self.assertNotIn("private-path", str(stderr.write.call_args_list))
        self.request()
        self.assertEqual(self.writer.records, 1)
        self.assertEqual(len(self.records()), 1)

    def test_new_files_on_rotation(self):
        with mock.patch("model_audit.MAX_FILE_BYTES", 1):
            self.request()
            self.request()
        self.assertEqual(len(list(self.writer.directory.glob("capture-*.jsonl"))), 2)


class ConfigurationTests(unittest.TestCase):
    def test_https_fixed_destination_validation(self):
        for url in ("http://example.com/v1", "https://user:pass@example.com", "https://example.com?key=secret",
                    "https://example.com/#fragment", "https://127.0.0.1/v1"):
            with self.assertRaises(ValueError):
                validate_upstream(url)
        self.assertEqual(validate_upstream("https://example.com/v1/"), "https://example.com/v1")

    def test_profile_provider_selection_and_no_credential_copy(self):
        config = {"model_provider": "first", "profiles": {"work": {"model_provider": "second"}},
                  "model_providers": {"second": {"base_url": "https://example.com/v1", "env_key": "PRIVATE_KEY"}}}
        self.assertEqual(configured_provider(config, ["--profile", "work"]), ("second", "https://example.com/v1"))
        command = codex_command("codex", "second", 1234, ["exec", "task"], "configured")
        self.assertNotIn("PRIVATE_KEY", " ".join(command))
        self.assertIn("features.enable_request_compression=false", command)
        self.assertIn("model_providers.second.supports_websockets=false", command)

    def test_profile_file_overrides_base_url_before_credentials_are_forwarded(self):
        config = {"model_provider": "custom", "model_providers": {"custom": {
            "base_url": "https://base.example/v1", "env_key": "PRIVATE_KEY"}}}
        with tempfile.TemporaryDirectory() as temp:
            home = Path(temp)
            (home / "work.config.toml").write_text(
                '[model_providers.custom]\nbase_url = "https://profile.example/v1"\n')
            for args in (["--profile", "work"], ["-pwork"], ["--profile=work"]):
                self.assertEqual(configured_provider(config, args, home), ("custom", "https://profile.example/v1"))
            with self.assertRaises(ValueError):
                configured_provider(config, ["--profile", "../escape"], home)

    def test_official_uses_fresh_oauth_provider_and_valid_toml_values(self):
        args = ["exec", "--sandbox", "read-only", "normal task"]
        command = codex_command("codex", "openai", 1234, args, "official")
        values = {}
        for i, part in enumerate(command):
            if part == "-c":
                key, raw = command[i + 1].split("=", 1)
                values[key] = tomllib.loads("value=" + raw)["value"]
        provider = values["model_provider"]
        self.assertTrue(provider.startswith("wtm_official_"))
        self.assertEqual(values[f"model_providers.{provider}.name"], "OpenAI")
        self.assertTrue(values[f"model_providers.{provider}.requires_openai_auth"])
        self.assertFalse(values[f"model_providers.{provider}.supports_websockets"])
        self.assertEqual(command[-len(args):], args)
        self.assertNotIn("CODEX_HOME", " ".join(command))

    def test_routing_cannot_be_accidentally_overridden(self):
        for args in (["-c", 'model_provider="other"'], ["--config=openai_base_url=bad"],
                     ["-c", 'model_providers={other={base_url="bad"}}'],
                     ["-c", '"model_provider"="other"'], ["-c", 'profiles.work.model_provider="other"'],
                     ["--enable=some_feature,enable_request_compression"],
                     ["-cmodel_providers.other.base_url=bad"], ["--enable", "enable_request_compression"]):
            with self.assertRaises(ValueError):
                codex_command("codex", "custom", 1234, args, "configured")
        # A prompt mentioning configuration must not be mistaken for a flag.
        codex_command("codex", "custom", 1234, ["exec", 'model_provider="example"'], "configured")


if __name__ == "__main__":
    unittest.main()
