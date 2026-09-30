import { expect, test, type Page } from "@playwright/test";
import { batch, call, makeSnapshot, snapshot, type HarnessOptions } from "./fixtures";

async function boot(page: Page, options: Partial<HarnessOptions> = {}) {
  await page.addInitScript(
    (opts) => {
      window.__wtmOptions = opts;
    },
    { snapshot, ...options },
  );
  await page.goto("/tests/browser/index.html");
  await expect(page.locator(".live-badge")).not.toHaveText("Starting");
}
const rows = (page: Page) => page.locator("tr[data-call-row]");
const summary = (page: Page, label: string) =>
  page
    .locator(".summary > div")
    .filter({ has: page.locator("span", { hasText: label }) })
    .locator("strong");

test("retention evictions preserve exact UI counts and do not resurrect old records", async ({
  page,
}) => {
  await boot(page);
  const newest = {
    ...call("new", "c1", 100, 50, 20),
    model: "new-model",
    timestamp: "2026-09-24T10:00:00Z",
    timestampMs: Date.parse("2026-09-24T10:00:00Z"),
  };
  const update = batch([newest], 1, ["A"], 3);
  update.status.truncated = true;
  await page.evaluate((value) => window.__wtm.emit("monitor-update", value), update);
  await expect(rows(page)).toHaveCount(3);
  await expect(rows(page).first().locator(".model-stack strong")).toHaveText("new-model");
  await expect(page.locator(".scope-note")).toContainText("已截断");
  await page.evaluate(
    (value) => window.__wtm.emit("monitor-update", value),
    batch([snapshot.calls[0]], 0, [], 4),
  );
  await expect(rows(page)).toHaveCount(3);
  await expect(summary(page, "Total tokens")).toHaveText("690");
});

test("a missing batch triggers a fresh authoritative snapshot", async ({ page }) => {
  const authoritative = makeSnapshot([{ ...call("only", "c2", 10, 5, 2), model: "resynced" }], 3);
  await boot(page, { resyncSnapshot: authoritative });
  await page.evaluate((value) => window.__wtm.emit("monitor-update", value), batch([], 2, [], 3));
  await expect(rows(page)).toHaveCount(1);
  await expect(rows(page).first().locator(".model-stack strong")).toHaveText("resynced");
  await expect.poll(() => page.evaluate(() => window.__wtm.snapshots)).toBe(2);
});
test("heartbeat stalls are visible and recover on a fresh update", async ({ page }) => {
  await page.clock.install();
  await boot(page);
  await page.clock.fastForward(6100);
  await expect(page.locator(".live-badge")).toHaveText("Stale");
  await expect(page.getByRole("alert")).toContainText("心跳");
  await page.evaluate((value) => window.__wtm.emit("monitor-update", value), batch([], 1, [], 3));
  await page.clock.runFor(1100);
  await expect(page.locator(".live-badge")).toHaveText("Live");
});
test("saved filters are not discarded while initial indexing is incomplete", async ({ page }) => {
  await boot(page, {
    snapshot: makeSnapshot([], 0, { initialLoadComplete: false, discovering: true }),
    saved: { "wtm.project": "p2", "wtm.conversation": "c2" },
  });
  await expect(page.locator(".live-badge")).toHaveText("Indexing");
  const ready = makeSnapshot([snapshot.calls[1]], 1);
  await page.evaluate(
    (s) =>
      window.__wtm.emit("monitor-update", {
        revision: s.revision,
        calls: s.calls,
        removedIds: [],
        catalog: s.catalog,
        status: s.status,
      }),
    ready,
  );
  await expect(rows(page)).toHaveCount(1);
  await expect(page.locator(".conversation-field summary")).toContainText("Beta conversation");
});
test("unknown usage remains unknown and incomplete totals are marked", async ({ page }) => {
  const incomplete = structuredClone(snapshot.calls[1]);
  incomplete.usage.inputTokens = null;
  incomplete.usage.cachedInputTokens = null;
  incomplete.usage.totalTokens = null;
  incomplete.cacheHitRate = null;
  incomplete.issues = ["missing_input_tokens"];
  await boot(page, { snapshot: makeSnapshot([snapshot.calls[0], incomplete]) });
  await expect(summary(page, "Total tokens")).toHaveText("1.1K*");
  await expect(summary(page, "Average cache hit")).toHaveText("60.0%*");
  await expect(rows(page).first().locator(".token-input")).toHaveText("—");
});
test("legacy-only logs are distinguished from an empty collection", async ({ page }) => {
  await boot(page, { snapshot: makeSnapshot([], 0, { legacyRecords: 3 }) });
  await expect(page.locator(".live-badge")).toHaveText("Unsupported");
  await expect(page.getByRole("status")).toContainText("旧累计日志");
});
test("project and conversation summaries share the same filtered data", async ({ page }) => {
  await boot(page);
  await page.getByLabel("视图", { exact: true }).selectOption("project");
  await expect(page.locator("tr[data-group-row]")).toHaveCount(2);
  await expect(page.locator("tr[data-group-row]").first()).toContainText("1.32K");
  await page.locator("tr[data-group-row]").first().getByRole("button").click();
  await expect(summary(page, "Visible calls")).toHaveText("2");
  await page.getByLabel("视图", { exact: true }).selectOption("conversation");
  await expect(page.locator("tr[data-group-row]")).toHaveCount(1);
});
test("quick dates use the local calendar and manual invalid ranges are explained", async ({
  page,
}) => {
  await page.clock.setFixedTime(new Date("2026-09-23T12:00:00+08:00"));
  await boot(page);
  await page.getByRole("button", { name: "今天", exact: true }).click();
  await expect(summary(page, "Visible calls")).toHaveText("2");
  await page.locator(".date-filter > summary").click();
  await page.locator('input[type="date"]').first().fill("2026-09-25");
  await expect(page.getByRole("alert")).toContainText("开始日期不能晚于结束日期");
});
test("exports redact identifying metadata by default and include every filtered row", async ({
  page,
}) => {
  await boot(page);
  await page.locator(".export-menu > summary").click();
  await page.getByRole("button", { name: "导出 JSON", exact: true }).click();
  await expect(page.locator(".export-popover")).toContainText("已导出 3 条");
  let content = await page.evaluate(() => window.__wtm.exports.at(-1)!.content);
  const exported = JSON.parse(content);
  expect(exported.rows).toHaveLength(3);
  expect(exported.scope.redacted).toBe(true);
  expect(exported.scope.kind).toBe("filtered_retained_window");
  expect(content).not.toContain("Alpha conversation");
  expect(content).not.toContain("conversationId");
  expect(content).not.toContain("/synthetic/");
  await page.getByLabel("包含真实标题、ID 和项目路径").check();
  await page.getByRole("button", { name: "导出 CSV", exact: true }).click();
  await expect.poll(() => page.evaluate(() => window.__wtm.exports.length)).toBe(2);
  content = await page.evaluate(() => window.__wtm.exports.at(-1)!.content);
  expect(content).toContain("Alpha conversation");
  expect(content).toContain('"conversationId"');
  expect(content.split("\r\n")).toHaveLength(5);
});
test("50k records use a bounded DOM and exports are not limited to virtual rows", async ({
  page,
}, info) => {
  test.setTimeout(60000);
  const data = Array.from({ length: 50000 }, (_, n) => {
    const timestampMs = Date.UTC(2026, 8, 23, 0, 0, n);
    return {
      ...call(`perf-${n.toString().padStart(5, "0")}`, "c1", 10, 5, 2),
      model: `model-${n}`,
      timestamp: new Date(timestampMs).toISOString(),
      timestampMs,
    };
  });
  const started = performance.now();
  await boot(page, { snapshot: makeSnapshot(data, 0, { truncated: true }) });
  await expect(summary(page, "Visible calls")).toHaveText("50,000");
  const firstReadyMs = performance.now() - started;
  expect(await rows(page).count()).toBeLessThan(100);
  await page.getByLabel("Follow latest").uncheck();
  await page.locator(".table-scroll").evaluate((element) => {
    element.scrollTop = element.scrollHeight;
  });
  await expect(rows(page).last().locator(".model-stack strong")).toHaveText("model-0");
  expect(await rows(page).count()).toBeLessThan(100);
  await page.locator(".export-menu > summary").click();
  await page.getByRole("button", { name: "导出 JSON", exact: true }).click();
  await expect
    .poll(() => page.evaluate(() => window.__wtm.exports.length), { timeout: 20000 })
    .toBe(1);
  const exported = await page.evaluate(
    () => JSON.parse(window.__wtm.exports[0].content).rows.length,
  );
  expect(exported).toBe(50000);
  await info.attach("performance.json", {
    body: JSON.stringify({ records: 50000, firstReadyMs, renderedRows: await rows(page).count() }),
    contentType: "application/json",
  });
});
test("an export failure is visible and can be retried", async ({ page }) => {
  await boot(page, { failExport: true });
  await page.locator(".export-menu > summary").click();
  await page.getByRole("button", { name: "导出 JSON", exact: true }).click();
  await expect(page.locator(".export-popover")).toContainText("Synthetic export failure");
  await page.evaluate(() => {
    window.__wtmOptions.failExport = false;
  });
  await page.getByRole("button", { name: "导出 JSON", exact: true }).click();
  await expect(page.locator(".export-popover")).toContainText("已导出 3 条");
});
