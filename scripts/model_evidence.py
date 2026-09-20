"""Bounded, body-free model observations shared by the collector and probe.

All results compare provider-declared identifiers; none authenticate model weights.
"""
from __future__ import annotations

import collections
import json
import re

from collections.abc import Iterable, Mapping

HeaderValues = Mapping[str, str] | Iterable[tuple[str, str]]
MODEL_HEADERS = ("openai-model", "x-openai-model")
MAX_EVENT_BYTES = 2 * 1024 * 1024
MAX_VALUES = 32
IDENTIFIER = re.compile(r"[A-Za-z0-9][A-Za-z0-9._:/+@-]{0,255}\Z")
TERMINAL_EVENTS = {"response.completed", "response.done", "response.failed", "response.incomplete"}


def identifier(value: object) -> str | None:
    """Accept identifiers, never arbitrary text, nested objects or unbounded strings."""
    if isinstance(value, str) and IDENTIFIER.fullmatch(value):
        return value
    return None


def model_headers(headers: HeaderValues) -> list[str]:
    pairs = headers.items() if isinstance(headers, Mapping) else headers
    return list(dict.fromkeys(
        model for key, value in pairs
        if str(key).lower() in MODEL_HEADERS and (model := identifier(value))
    ))


class Evidence:
    def __init__(self, requested: object, headers: HeaderValues):
        self.requested = identifier(requested)
        self.http_models: list[str] = []
        self.event_models: list[str] = []
        self.body_models: list[str] = []
        self.response_id: str | None = None
        self.event_types: collections.Counter = collections.Counter()
        self.completed = False
        self.terminal_model: str | None = None
        self.capture_issues: list[str] = []
        self.id_conflict = False

        if requested is not None and self.requested is None:
            self.issue("invalid_identifier")
        for model in model_headers(headers):
            self._add(self.http_models, model)

    def issue(self, issue: str) -> None:
        if issue not in self.capture_issues:
            self.capture_issues.append(issue)

    def _add(self, values: list[str], value: str) -> None:
        if value in values:
            return
        if len(values) >= MAX_VALUES:
            self.issue("too_many_models")
        else:
            values.append(value)

    def consume(self, event: dict) -> None:
        kind = identifier(event.get("type"))
        if kind and (kind in self.event_types or len(self.event_types) < MAX_VALUES):
            self.event_types[kind] += 1
        response = event.get("response")
        if not isinstance(response, dict):
            # A non-streaming Responses response has its fields at the top level.
            response = event if event.get("object") == "response" or not kind else {}
        response_id = identifier(response.get("id"))
        if response.get("id") is not None and response_id is None:
            self.issue("invalid_identifier")
        if response_id:
            if self.response_id and self.response_id != response_id:
                self.id_conflict = True
                self.issue("response_id_conflict")
            elif not self.id_conflict:
                self.response_id = response_id
        for headers in (event.get("headers"), response.get("headers")):
            if isinstance(headers, dict):
                for model in model_headers(headers):
                    self._add(self.event_models, model)
        model = identifier(response.get("model"))
        if response.get("model") is not None and model is None:
            self.issue("invalid_identifier")
        if model:
            self._add(self.body_models, model)
            if kind in TERMINAL_EVENTS or not kind:
                self.terminal_model = model
        if kind in {"response.completed", "response.done"} or response.get("status") == "completed":
            self.completed = True

    def report(self) -> dict:
        all_models = list(dict.fromkeys(self.body_models + self.http_models + self.event_models))
        reported = self.terminal_model or next(iter(self.body_models + self.http_models + self.event_models), None)
        sources = []
        for source, models in (("response_body", self.body_models), ("http_header", self.http_models),
                               ("event_header", self.event_models)):
            if models:
                sources.append(source)
        if len(all_models) > 1:
            verdict = "conflict"
        elif self.capture_issues or not reported or not self.requested:
            verdict = "unknown"
        elif reported == self.requested:
            verdict = "match"
        else:
            verdict = "different"
        return {
            "requestedModel": self.requested,
            "responseId": None if self.id_conflict else self.response_id,
            "reportedModel": reported,
            "httpHeaderModels": self.http_models.copy(),
            "eventHeaderModels": self.event_models.copy(),
            "responseBodyModels": self.body_models.copy(),
            "evidenceSources": sources,
            "completed": self.completed,
            "verdict": verdict,
            "captureIssues": self.capture_issues.copy(),
            "eventTypes": dict(self.event_types),
        }


class ResponseObserver:
    """Incrementally inspect SSE/JSON without retaining whole response streams.

    The relay forwards original bytes independently of this observer. Oversized or
    malformed data degrades the audit to unknown rather than interrupting a call.
    """
    def __init__(self, evidence: Evidence, content_type: str):
        self.evidence = evidence
        self.sse = "text/event-stream" in content_type.lower()
        self.buffer = bytearray()
        self.data: list[bytes] = []
        self.data_size = 0
        self.discard_line = False
        self.discard_event = False
        self.event_type = ""
        self.disabled = False

    def _consume(self, payload: bytes) -> None:
        if payload.strip() == b"[DONE]":
            return
        try:
            value = json.loads(payload)
            if isinstance(value, dict):
                if self.event_type and "type" not in value:
                    value["type"] = self.event_type
                self.evidence.consume(value)
            else:
                self.evidence.issue("invalid_json")
        except (ValueError, UnicodeError, RecursionError):
            self.evidence.issue("invalid_json")

    def _dispatch(self) -> None:
        if self.data and not self.discard_event:
            self._consume(b"\n".join(self.data))
        self.data.clear()
        self.data_size = 0
        self.discard_event = False
        self.event_type = ""

    def _line(self, line: bytes) -> None:
        line = line.rstrip(b"\r")
        if not line:
            self._dispatch()
        elif line.startswith(b"event:"):
            self.event_type = identifier(line[6:].strip().decode("ascii", errors="replace")) or ""
        elif line.startswith(b"data:"):
            part = line[5:].removeprefix(b" ")
            self.data_size += len(part) + 1
            if self.data_size > MAX_EVENT_BYTES:
                self.evidence.issue("event_too_large")
                self.data.clear()
                self.discard_event = True
            elif not self.discard_event:
                self.data.append(part)

    def feed(self, chunk: bytes) -> None:
        if self.disabled:
            return
        if not self.sse:
            if len(self.buffer) + len(chunk) > MAX_EVENT_BYTES:
                self.evidence.issue("event_too_large")
                self.buffer.clear()
                self.disabled = True
            else:
                self.buffer.extend(chunk)
            return
        parts = chunk.split(b"\n")
        for index, part in enumerate(parts):
            ended = index < len(parts) - 1
            if not self.discard_line:
                if len(self.buffer) + len(part) > MAX_EVENT_BYTES:
                    self.buffer.clear()
                    self.discard_line = True
                    self.discard_event = True
                    self.evidence.issue("event_too_large")
                else:
                    self.buffer.extend(part)
            if ended:
                if not self.discard_line:
                    self._line(bytes(self.buffer))
                self.buffer.clear()
                self.discard_line = False

    def finish(self) -> None:
        if self.disabled:
            return
        if self.sse:
            if self.buffer and not self.discard_line:
                self._line(bytes(self.buffer))
            self._dispatch()
        elif self.buffer:
            self._consume(bytes(self.buffer))
        else:
            self.evidence.issue("empty_response")
        self.buffer.clear()
        if not self.evidence.completed:
            self.evidence.issue("incomplete_response")
