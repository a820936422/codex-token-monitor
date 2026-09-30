import { test, expect, type Page } from "@playwright/test";
import { call, snapshot, type HarnessOptions } from "./fixtures";

async function boot(page: Page, options: Partial<HarnessOptions> = {}) {
  await page.addInitScript((settings) => {
    window.__wtmOptions = settings;
    for (const [key, value] of Object.entries(settings.saved ?? {})) localStorage.setItem(key, value);
  }, { snapshot, ...options });
  await page.goto("/tests/browser/");
}

async function ready(page: Page) { await expect(page.locator(".live-badge")).toHaveText("Live"); }
const rows = (page: Page) => page.locator("tbody tr");
const summary = (page: Page, label: string) => page.locator(".summary > div").filter({ has: page.locator("span", { hasText: new RegExp(`^${label}$`) }) }).locator("strong");

test("token-only table, weighted cache rate, and restricted commands", async ({ page }) => {
  const external: string[] = [];
  page.on("request", (request) => { if (!request.url().startsWith("http://127.0.0.1:5178/")) external.push(request.url()); });
  await boot(page); await ready(page);
  await expect(rows(page)).toHaveCount(3);
  await expect(page.locator("thead th")).toHaveCount(8);
  await expect(page.getByText("日志模型", { exact: true })).toBeVisible();
  await expect(page.getByText(/模型核对|模型采集|声明一致|声明冲突/)).toHaveCount(0);
  await expect(summary(page, "Total tokens")).toHaveText("1.67K");
  await expect(summary(page, "Average cache hit")).toHaveText("53.3%");
  const commands = await page.evaluate(() => window.__wtm.commands);
  expect(commands.every((cmd) => ["get_snapshot", "plugin:event|listen", "plugin:event|unlisten"].includes(cmd))).toBe(true);
  expect(await page.evaluate(() => window.__wtm.activeListeners.size)).toBe(3);
  expect(external).toEqual([]);
});

test("project and conversation filters update the token summary", async ({ page }) => {
  await boot(page); await ready(page);
  await page.locator(".project-field summary").click();
  await page.locator(".project-field .picker-option").filter({ hasText: "Alpha project" }).click();
  await expect(rows(page)).toHaveCount(2);
  await expect(summary(page, "Total tokens")).toHaveText("1.32K");
  await page.getByRole("button", { name: "清除筛选", exact: true }).click();
  await page.locator(".conversation-field summary").click();
  await page.locator(".conversation-field .picker-option").filter({ hasText: "Beta conversation" }).click();
  await expect(rows(page)).toHaveCount(1);
  await expect(summary(page, "Total tokens")).toHaveText("350");
});

test("date filter includes the entire selected local day", async ({ page }) => {
  await boot(page); await ready(page);
  await page.locator(".date-filter summary").click();
  await page.getByLabel("From", { exact: true }).fill("2026-09-23");
  await page.getByLabel("To", { exact: true }).fill("2026-09-23");
  await expect(rows(page)).toHaveCount(2);
  await expect(summary(page, "Total tokens")).toHaveText("570");
  await page.getByRole("button", { name: "清除日期", exact: true }).click();
  await expect(rows(page)).toHaveCount(3);
});

test("sort direction changes chronological row order", async ({ page }) => {
  await boot(page); await ready(page);
  await expect(rows(page).first().locator(".model-stack strong")).toHaveText("logged-C");
  await page.getByLabel("Sort", { exact: true }).selectOption("asc");
  await expect(rows(page).first().locator(".model-stack strong")).toHaveText("logged-A");
});

test("saved project and conversation survive asynchronous startup", async ({ page }) => {
  await boot(page, { saved: { "wtm.project": "p2", "wtm.conversation": "c2" }, delaySnapshotMs: 80 });
  await ready(page);
  await expect(rows(page)).toHaveCount(1);
  await expect(page.locator(".project-field summary")).toContainText("Beta project");
  await expect(page.locator(".conversation-field summary")).toContainText("Beta conversation");
});

test("obsolete column settings are ignored and keyboard resizing persists", async ({ page }) => {
  await boot(page, { saved: { "wtm.columnWidths": JSON.stringify({ model: 205, modelAudit: 700 }) } });
  await ready(page);
  const handle = page.getByRole("separator", { name: "调整 model 列宽", exact: true });
  await expect(handle).toHaveAttribute("aria-valuenow", "205");
  await handle.focus(); await page.keyboard.press("ArrowRight");
  await expect(handle).toHaveAttribute("aria-valuenow", "215");
  expect(await page.evaluate(() => JSON.parse(localStorage.getItem("wtm.columnWidths")!).model)).toBe(215);
  await expect(page.locator("thead th")).toHaveCount(8);
  await handle.dblclick();
  await expect(handle).toHaveAttribute("aria-valuenow", "180");
});

test("events arriving before the snapshot are retained", async ({ page }) => {
  await boot(page, { delaySnapshotMs: 30, injectBeforeSnapshot: call("D") }); await ready(page);
  await expect(rows(page)).toHaveCount(4);
  await expect(summary(page, "Total tokens")).toHaveText("2.77K");
});

test("repeated response events replace a record instead of duplicating usage", async ({ page }) => {
  await boot(page); await ready(page);
  const updated = call("A", "c1", 1000, 600, 200);
  await page.evaluate(async (item) => { await window.__wtm.emit("monitor-call", item); await window.__wtm.emit("monitor-call", item); }, updated);
  await expect(rows(page)).toHaveCount(3);
  await expect(summary(page, "Total tokens")).toHaveText("1.77K");
});

test("initialization errors are visible and listeners are cleaned up", async ({ page }) => {
  await boot(page, { failSnapshot: true });
  await expect(page.locator(".live-badge")).toHaveText("Error");
  await expect(page.getByRole("status")).toContainText("监控初始化失败");
  await expect.poll(() => page.evaluate(() => window.__wtm.activeListeners.size)).toBe(0);
});

test("partially failed listener registration does not leak listeners", async ({ page }) => {
  await boot(page, { failListen: "monitor-catalog" });
  await expect(page.locator(".live-badge")).toHaveText("Error");
  await expect.poll(() => page.evaluate(() => window.__wtm.activeListeners.size)).toBe(0);
});

test("empty log collection shows a useful empty state", async ({ page }) => {
  await boot(page, { snapshot: { ...snapshot, calls: [], catalog: { projects: [], conversations: [] } } });
  await ready(page);
  await expect(page.getByRole("status")).toContainText("当前筛选条件下还没有模型调用");
  await expect(summary(page, "Total tokens")).toHaveText("0");
  await expect(summary(page, "Average cache hit")).toHaveText("—");
});

test("conversation content is rendered as text, and ID copy uses write-only IPC", async ({ page }) => {
  const hostile = structuredClone(snapshot);
  hostile.catalog.conversations[0].title = '<img src=x onerror="alert(1)">';
  await boot(page, { snapshot: hostile }); await ready(page);
  await expect(page.locator("tbody img")).toHaveCount(0);
  const cell = rows(page).filter({ hasText: "logged-A" }).locator(".conversation-button");
  await expect(cell).toContainText("<img src=x");
  await cell.click({ button: "right" });
  await page.getByRole("button", { name: "复制会话 ID", exact: true }).click();
  expect(await page.evaluate(() => window.__wtm.copied)).toBe("c1");
});

for (const [width, height, theme] of [[1280, 760, "dark"], [900, 600, "light"]] as const) {
  test(`layout ${width}x${height} ${theme}`, async ({ page }, testInfo) => {
    await page.setViewportSize({ width, height });
    await page.emulateMedia({ colorScheme: theme });
    await boot(page); await ready(page);
    expect(await page.evaluate(() => document.documentElement.scrollWidth <= window.innerWidth)).toBe(true);
    await expect(page.locator(".toolbar")).toBeVisible();
    await expect(page.locator(".collector-panel")).toHaveCount(0);
    await page.screenshot({ path: testInfo.outputPath("token-only.png"), fullPage: true });
  });
}
