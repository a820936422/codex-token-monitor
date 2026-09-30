// Native Tauri/WebKit acceptance test. No mock IPC, live accounts, or model requests.
import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import { mkdtemp, mkdir, writeFile, appendFile, readFile, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import { createServer } from "node:net";

const binary = resolve(
  process.env.WTM_TEST_BINARY ?? "src-tauri/target/release/work-token-monitor",
);
const driverBinary = process.env.TAURI_DRIVER ?? "tauri-driver";
const workspace = await mkdtemp(join(tmpdir(), "wtm-native-smoke-"));
const output = resolve(process.env.WTM_NATIVE_OUTPUT ?? "test-results/native");
await mkdir(output, { recursive: true });
const codex = join(workspace, "codex");
await mkdir(join(codex, "sessions"), { recursive: true });
await mkdir(join(codex, "archived_sessions"), { recursive: true });
const log = join(codex, "sessions", "synthetic.jsonl");
function record(n) {
  return (
    JSON.stringify({
      type: "token_usage_record",
      timestamp: new Date(Date.UTC(2026, 8, 23, 10, 0, n)).toISOString(),
      payload: {
        response_id: `native-${n}`,
        session_id: "native-session",
        thread_id: "native-session",
        turn_id: "native-turn",
        usage: {
          input_tokens: 100,
          cached_input_tokens: 60,
          cache_write_input_tokens: 0,
          output_tokens: 20,
        },
      },
    }) + "\n"
  );
}
await writeFile(
  log,
  JSON.stringify({ type: "session_meta", payload: { id: "native-session", cwd: workspace } }) +
    "\n" +
    JSON.stringify({
      type: "turn_context",
      payload: { turn_id: "native-turn", model: "synthetic-model", effort: "high" },
    }) +
    "\n" +
    record(0) +
    record(1) +
    record(2),
);
await writeFile(
  join(codex, "session_index.jsonl"),
  JSON.stringify({
    id: "native-session",
    thread_name: "Synthetic native acceptance",
    updated_at: "2026-09-23T10:00:00Z",
  }) + "\n",
);
await writeFile(join(codex, "config.toml"), "# SYNTHETIC UNTOUCHED CONFIG\n");
async function unusedPort() {
  const server = createServer();
  await new Promise((resolve, reject) => {
    server.once("error", reject);
    server.listen(0, "127.0.0.1", resolve);
  });
  const port = server.address().port;
  await new Promise((resolve) => server.close(resolve));
  return port;
}
const port = await unusedPort(),
  nativePort = await unusedPort();
const args = [
  "--port",
  String(port),
  "--native-port",
  String(nativePort),
  "--native-host",
  "127.0.0.1",
];
if (process.env.WEBKIT_WEBDRIVER)
  args.push("--native-driver", resolve(process.env.WEBKIT_WEBDRIVER));
const env = {
  ...process.env,
  CODEX_HOME: codex,
  CODEX_SESSION_ROOT: join(codex, "sessions"),
  CODEX_SESSION_INDEX: join(codex, "session_index.jsonl"),
  WTM_RECORD_LIMIT: "100",
  XDG_CONFIG_HOME: join(workspace, "config"),
  XDG_DATA_HOME: join(workspace, "data"),
  XDG_CACHE_HOME: join(workspace, "cache"),
};
const driver = spawn(driverBinary, args, {
  env,
  stdio: ["ignore", "pipe", "pipe"],
  detached: process.platform !== "win32",
});
let driverLog = "",
  spawnError = null,
  session = null;
driver.on("error", (error) => {
  spawnError = error;
});
for (const stream of [driver.stdout, driver.stderr])
  stream.on("data", (data) => {
    driverLog = (driverLog + data.toString()).slice(-100000);
  });
const base = `http://127.0.0.1:${port}`;
async function request(method, path, body) {
  const response = await fetch(base + path, {
    method,
    headers: { "content-type": "application/json" },
    body: body === undefined ? undefined : JSON.stringify(body),
    signal: AbortSignal.timeout(30000),
  });
  const result = await response.json();
  if (!response.ok || result.value?.error)
    throw new Error(`${method} ${path}: ${JSON.stringify(result)}`);
  return result.value;
}
const pause = (ms) => new Promise((resolve) => setTimeout(resolve, ms));
async function until(predicate, label) {
  for (let n = 0; n < 150; n++) {
    if (await predicate()) return;
    await pause(100);
  }
  throw new Error(`Timed out: ${label}`);
}
try {
  await until(async () => {
    if (spawnError) throw spawnError;
    try {
      return Boolean(await request("GET", "/status"));
    } catch {
      return false;
    }
  }, "WebDriver startup");
  const created = await request("POST", "/session", {
    capabilities: { alwaysMatch: { "tauri:options": { application: binary } } },
  });
  session = created.sessionId;
  const execute = (script) =>
    request("POST", `/session/${session}/execute/sync`, { script, args: [] });
  const countScript = 'return document.querySelector(".summary > div strong")?.textContent ?? ""';
  await until(async () => (await execute(countScript)) === "3", "initial native records");
  assert.equal(
    await execute('return document.querySelector(".live-badge")?.textContent.trim()'),
    "Live",
  );
  assert.equal(
    await execute('return document.querySelector("h1")?.textContent'),
    "Work Token Monitor",
  );
  await appendFile(log, record(3));
  await until(
    async () => (await execute(countScript)) === "4",
    "real append event through native IPC",
  );
  const content = Array.from({ length: 100 }, (_, n) => record(n + 4)).join("");
  await appendFile(log, content);
  await until(async () => (await execute(countScript)) === "100", "bounded native retention");
  assert.equal(
    await execute('return document.querySelector(".scope-note").textContent.includes("已截断")'),
    true,
  );
  assert.ok(await execute('return document.querySelectorAll("tr[data-call-row]").length < 100'));
  await execute(
    'const select=document.querySelector("select[aria-label=视图]");select.value="project";select.dispatchEvent(new Event("change",{bubbles:true}));return true',
  );
  await until(
    async () =>
      (await execute('return document.querySelectorAll("tr[data-group-row]").length')) === 1,
    "native project summary",
  );
  assert.equal(
    await readFile(join(codex, "config.toml"), "utf8"),
    "# SYNTHETIC UNTOUCHED CONFIG\n",
  );
  if (process.env.WTM_NATIVE_SCREENSHOT === "1") {
    try {
      const image = await request("GET", `/session/${session}/screenshot`);
      await writeFile(join(output, "native.png"), Buffer.from(image, "base64"));
    } catch (error) {
      console.warn("Screenshot unavailable:", error.message);
    }
  }
  await writeFile(
    join(output, "result.json"),
    JSON.stringify(
      {
        passed: true,
        initialRecords: 3,
        appendedRecords: 104,
        retained: 100,
        native: true,
        mockedIPC: false,
      },
      null,
      2,
    ),
  );
  console.log(
    "Native smoke passed: initial load, real append/batch IPC, 100-record retention, virtual table, project summary, unchanged config.",
  );
} finally {
  if (session) {
    try {
      await request("DELETE", `/session/${session}`);
    } catch {}
  }
  if (driver.pid && driver.exitCode === null) {
    try {
      process.platform === "win32" ? driver.kill("SIGTERM") : process.kill(-driver.pid, "SIGTERM");
    } catch {}
    await pause(500);
    if (driver.exitCode === null) {
      try {
        process.platform === "win32"
          ? driver.kill("SIGKILL")
          : process.kill(-driver.pid, "SIGKILL");
      } catch {}
    }
  }
  await writeFile(join(output, "driver.log"), driverLog);
  await rm(workspace, { recursive: true, force: true });
}
