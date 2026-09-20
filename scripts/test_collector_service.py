"""Offline capture-controller lifecycle and privacy regressions."""
import json
from pathlib import Path
import socket
import tempfile
import threading
import unittest
from unittest import mock

from model_audit import AuditRelay, MetadataWriter
from model_collector_service import CollectorService, LEASE_SECONDS, serve_control


class ControllerTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.relay = AuditRelay('https://fixture.example/v1', MetadataWriter(Path(self.temporary.name) / 'audit'))
        self.addCleanup(self.relay.server_close)
        self.service = CollectorService(self.relay)

    def test_lease_expiry_pauses_and_heartbeat_cannot_restart_capture(self):
        with mock.patch('model_collector_service.time.monotonic', return_value=100.0):
            self.assertFalse(self.service.command({'command': 'status'})['enabled'])
            self.assertTrue(self.service.command({'command': 'capture', 'enabled': True})['enabled'])
        with mock.patch('model_collector_service.time.monotonic', return_value=100.0 + LEASE_SECONDS - 1):
            self.assertTrue(self.service.command({'command': 'heartbeat'})['enabled'])
        with mock.patch('model_collector_service.time.monotonic', return_value=100.0 + 2 * LEASE_SECONDS):
            self.assertFalse(self.service.command({'command': 'status'})['enabled'])
            self.assertFalse(self.service.command({'command': 'heartbeat'})['enabled'])
            self.assertTrue(self.service.command({'command': 'capture', 'enabled': True})['enabled'])

    def test_stop_is_separate_and_refuses_active_requests(self):
        self.service.command({'command': 'capture', 'enabled': True})
        self.service.command({'command': 'capture', 'enabled': False})
        self.assertFalse(self.service.done)
        self.relay.active_requests = 1
        self.assertEqual(self.service.command({'command': 'stop'}), {'error': 'requests_active'})
        self.assertFalse(self.service.done)
        self.relay.active_requests = 0
        self.assertFalse(self.service.command({'command': 'stop'})['enabled'])
        self.assertTrue(self.service.done)

    def test_control_rejects_invalid_payload_and_reports_resource_counters(self):
        self.assertEqual(self.service.command({'command':'capture','enabled':'false'}), {'error':'invalid_command'})
        status = self.service.command({'command':'status'})
        self.assertGreater(status['rssBytes'], 0)
        self.assertGreaterEqual(status['cpuSeconds'], 0)
        self.assertNotIn('Authorization', json.dumps(status))
        self.assertEqual(status['observations'], 0)

    def test_unix_control_is_bounded_and_stays_available_after_malformed_message(self):
        path = str(Path(self.temporary.name) / 'control.sock')
        with socket.socket(socket.AF_UNIX, socket.SOCK_STREAM) as listener:
            listener.bind(path)
            listener.listen(8)
            thread = threading.Thread(target=serve_control, args=(self.service, listener), daemon=True)
            thread.start()
            def request(payload):
                with socket.socket(socket.AF_UNIX, socket.SOCK_STREAM) as client:
                    client.settimeout(2)
                    client.connect(path)
                    client.sendall(payload)
                    response = client.recv(16384)
                    return json.loads(response)
            try:
                self.assertEqual(request(b'[]\n'), {'error':'invalid_command'})
                self.assertEqual(request(b'x' * 5000 + b'\n'), {'error':'invalid_command'})
                self.assertFalse(request(b'{"command":"status"}\n')['enabled'])
                request(b'{"command":"stop"}\n')
                thread.join(timeout=2)
                self.assertFalse(thread.is_alive())
            finally:
                self.service.done = True
                thread.join(timeout=2)


if __name__ == '__main__':
    unittest.main()
