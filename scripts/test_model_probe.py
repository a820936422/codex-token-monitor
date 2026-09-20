"""Model probe evidence tests; no network or real credentials are used."""
import http.server
import threading
import urllib.error
import urllib.request
import json
import unittest

from model_probe import Evidence, NoRedirect, ProbeRelay, model_headers


class EvidenceTests(unittest.TestCase):
    def test_header_names_are_case_insensitive(self):
        self.assertEqual(model_headers({"OpenAI-Model": "served", "X-OpenAI-Model": "served",
                                       "Authorization": "secret"}), ["served"])

    def test_response_body_detects_declared_model_difference(self):
        evidence = Evidence("requested", {})
        evidence.consume({"type": "response.completed", "response": {
            "id": "synthetic-response", "model": "different", "output": [{"text": "private text"}]}})
        result = evidence.report()
        self.assertEqual(result["verdict"], "different")
        self.assertEqual(result["responseBodyModels"], ["different"])
        self.assertEqual(result["responseId"], "synthetic-response")
        self.assertTrue(result["completed"])
        self.assertNotIn("private text", json.dumps(result))

    def test_missing_evidence_is_not_a_match(self):
        self.assertEqual(Evidence("requested", {}).report()["verdict"], "unknown")

    def test_http_response_header_can_detect_difference(self):
        evidence = Evidence("requested", {"openai-model": "served"})
        self.assertEqual(evidence.report()["verdict"], "different")

    def test_stream_metadata_can_confirm_identifier(self):
        evidence = Evidence("requested", {})
        evidence.consume({"type": "response.created", "response": {
            "id": "synthetic-response", "headers": {"x-openai-model": "requested"}}})
        evidence.consume({"type": "response.completed", "response": {"id": "synthetic-response"}})
        result = evidence.report()
        self.assertEqual(result["verdict"], "match")
        self.assertEqual(result["eventHeaderModels"], ["requested"])

    def test_disagreeing_header_sources_remain_a_conflict(self):
        evidence = Evidence("requested", {"openai-model": "requested"})
        evidence.consume({"type": "codex.response.metadata", "headers": {"openai-model": "different"}})
        self.assertEqual(evidence.report()["verdict"], "conflict")

    def test_repeated_metadata_is_deduplicated(self):
        evidence = Evidence("requested", {})
        event = {"type": "response.created", "response": {"headers": {"openai-model": "requested"},
                                                             "model": "body-alias"}}
        evidence.consume(event)
        evidence.consume(event)
        result = evidence.report()
        self.assertEqual(result["eventHeaderModels"], ["requested"])
        self.assertEqual(result["responseBodyModels"], ["body-alias"])
        self.assertEqual(result["verdict"], "conflict")


class RelayTests(unittest.TestCase):
    def setUp(self):
        self.received = []
        self.redirect = False
        self.redirect_followed = False
        case = self

        class Upstream(http.server.BaseHTTPRequestHandler):
            def log_message(self, format, *args):
                pass

            def do_GET(self):
                case.redirect_followed = True
                self.send_response(200)
                self.end_headers()

            def do_POST(self):
                payload = json.loads(self.rfile.read(int(self.headers["Content-Length"])))
                case.received.append((payload["model"], self.headers.get("Authorization")))
                if case.redirect:
                    self.send_response(302)
                    self.send_header("Location", "/redirect-target")
                    self.end_headers()
                    return
                events = [
                    {"type": "response.created", "response": {"id": "test-response", "model": "body-alias",
                        "headers": {"x-openai-model": "served"}}},
                    {"type": "response.completed", "response": {"id": "test-response", "model": "body-alias",
                        "output": [{"text": "do-not-store"}]}},
                ]
                data = "".join("event: " + e["type"] + "\ndata: " + json.dumps(e) + "\n\n" for e in events).encode()
                self.send_response(200)
                self.send_header("Content-Type", "text/event-stream")
                self.send_header("OpenAI-Model", "served")
                self.send_header("Content-Length", str(len(data)))
                self.end_headers()
                self.wfile.write(data)

        self.upstream = http.server.ThreadingHTTPServer(("127.0.0.1", 0), Upstream)
        threading.Thread(target=self.upstream.serve_forever, daemon=True).start()
        self.relay = ProbeRelay(f"http://127.0.0.1:{self.upstream.server_port}/v1",
                                {"Authorization": "Bearer upstream-test"}, "local-test")
        self.relay.upstream = urllib.request.build_opener(urllib.request.ProxyHandler({}),
                                                        NoRedirect())
        threading.Thread(target=self.relay.serve_forever, daemon=True).start()
        self.client = urllib.request.build_opener(urllib.request.ProxyHandler({}))

    def tearDown(self):
        for server in (self.relay, self.upstream):
            server.shutdown()
            server.server_close()

    def request(self, token="local-test"):
        return self.client.open(urllib.request.Request(
            f"http://127.0.0.1:{self.relay.server_port}/v1/responses",
            data=json.dumps({"model": "requested"}).encode(),
            headers={"Authorization": "Bearer " + token, "Content-Type": "application/json"},
        ), timeout=3)

    def test_stream_and_header_forwarding_preserve_metadata(self):
        with self.request() as response:
            self.assertEqual(response.headers.get("openai-model"), "served")
            self.assertIn(b"response.completed", response.read())
        self.assertEqual(self.received, [("requested", "Bearer upstream-test")])
        record = self.relay.evidence[0].report()
        self.assertEqual(record["responseId"], "test-response")
        self.assertEqual(record["verdict"], "conflict")
        self.assertTrue(record["completed"])
        self.assertNotIn("do-not-store", json.dumps(record))
        self.assertNotIn("upstream-test", json.dumps(record))

    def test_unknown_local_caller_never_reaches_upstream(self):
        with self.assertRaises(urllib.error.HTTPError) as caught:
            self.request("wrong-token")
        self.assertEqual(caught.exception.code, 401)
        caught.exception.close()
        self.assertEqual(self.received, [])

    def test_redirect_does_not_forward_credentials(self):
        self.redirect = True
        with self.assertRaises(urllib.error.HTTPError) as caught:
            self.request()
        self.assertEqual(caught.exception.code, 502)
        caught.exception.close()
        self.assertFalse(self.redirect_followed)
        self.assertEqual(self.relay.errors, [{"kind": "upstream_http", "status": 302}])


if __name__ == "__main__":
    unittest.main()
