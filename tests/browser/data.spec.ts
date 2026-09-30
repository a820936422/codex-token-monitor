import { expect, test } from "@playwright/test";
import { cacheRate, dateBoundary, filterError, ordered, summarize } from "../../src/data";
import { buildExport, csvCell } from "../../src/export";
import { call, makeSnapshot, snapshot } from "./fixtures";

test("aggregate arithmetic remains exact beyond the safe floating point sum", () => {
  const record = call("a", "c1", Number.MAX_SAFE_INTEGER, 0, 0);
  const t = summarize([record, { ...record, id: "b" }]);
  expect(t.total.toString()).toBe("18014398509481982");
  expect(cacheRate(t)).toBe(0);
});
test("unknown counters are excluded rather than converted to zero", () => {
  const record = call("a", "c1", 10, 5, 2);
  record.usage.inputTokens = null;
  record.usage.cachedInputTokens = null;
  record.usage.totalTokens = null;
  const t = summarize([record]);
  expect(t.totalKnown).toBe(0);
  expect(t.cachePairs).toBe(0);
  expect(cacheRate(t)).toBeNull();
});
test("invalid dates cannot silently roll over into a different day", () => {
  expect(dateBoundary("2026-02-30", false)).toBeNull();
  expect(dateBoundary("not-date", false)).toBeNull();
  expect(
    filterError({ projectId: "", conversationId: "", from: "2026-09-25", to: "2026-09-23" }),
  ).not.toBeNull();
});
test("sorting compares instants rather than displayed timestamp text", () => {
  const a = {
    ...snapshot.calls[0],
    timestamp: "2026-09-23T11:00:00+08:00",
    timestampMs: Date.parse("2026-09-23T11:00:00+08:00"),
  };
  const b = {
    ...snapshot.calls[1],
    timestamp: "2026-09-23T04:00:00Z",
    timestampMs: Date.parse("2026-09-23T04:00:00Z"),
  };
  expect(ordered([a, b])[0].id).toBe(b.id);
});
test("CSV text cannot become a spreadsheet formula and quoting preserves delimiters", () => {
  for (const value of ["=SUM(1,1)", " +1", "-2", "@SUM(1)", "\t=1"])
    expect(csvCell(value)).toContain("'");
  expect(csvCell('a,"b"\nc')).toBe('"a,""b""\nc"');
  expect(csvCell(null)).toBe('""');
});
test("JSON exports serialize large aggregates exactly and redact filter identities", () => {
  const item = {
    ...call("private-response", "c1", Number.MAX_SAFE_INTEGER, 0, 0),
    model: "logged-model",
  };
  const s = makeSnapshot([item, { ...item, id: "other-private-response" }]);
  const content = buildExport(
    s.calls,
    new Map(s.catalog.conversations.map((c) => [c.id, c])),
    s.status,
    { projectId: "private-path", conversationId: "private-id", from: "", to: "" },
    false,
    "json",
  );
  const report = JSON.parse(content);
  expect(report.summary.total).toBe("18014398509481982");
  expect(content).not.toContain("private-response");
  expect(content).not.toContain("private-path");
  expect(content).not.toContain("private-id");
  expect(report.scope.projectFilter).toBe("selected");
});
