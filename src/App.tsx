import { useEffect, useMemo, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { writeText } from "@tauri-apps/plugin-clipboard-manager";
import { Picker, type PickerOption } from "./components/Picker";
import type { CallRecord, Catalog, Conversation, MonitorStatus, Snapshot } from "./types";

const integer = new Intl.NumberFormat(undefined, { maximumFractionDigits: 0 });
const timeFormat = new Intl.DateTimeFormat(undefined, {
  month: "2-digit", day: "2-digit", hour: "2-digit", minute: "2-digit", second: "2-digit", hour12: false,
});

const COLUMN_WIDTHS_KEY = "wtm.columnWidths";
const MAX_COLUMN_WIDTH = 720;
const COLUMN_CONFIG = {
  time: { defaultWidth: 145, minWidth: 110 },
  project: { defaultWidth: 170, minWidth: 110 },
  conversation: { defaultWidth: 320, minWidth: 190 },
  model: { defaultWidth: 180, minWidth: 130 },
  input: { defaultWidth: 100, minWidth: 80 },
  cached: { defaultWidth: 100, minWidth: 80 },
  output: { defaultWidth: 100, minWidth: 80 },
  cache: { defaultWidth: 130, minWidth: 120 },
} as const;

type ColumnKey = keyof typeof COLUMN_CONFIG;
type ColumnWidths = Record<ColumnKey, number>;
const columnKeys = Object.keys(COLUMN_CONFIG) as ColumnKey[];

function saved(key: string) {
  try { return localStorage.getItem(key) || ""; } catch { return ""; }
}

function persist(key: string, value: string) {
  try { localStorage.setItem(key, value); } catch {}
}

function defaultColumnWidths(): ColumnWidths {
  return Object.fromEntries(columnKeys.map((key) => [key, COLUMN_CONFIG[key].defaultWidth])) as ColumnWidths;
}

function clampColumnWidth(key: ColumnKey, width: number) {
  return Math.max(COLUMN_CONFIG[key].minWidth, Math.min(MAX_COLUMN_WIDTH, Math.round(width)));
}

function loadColumnWidths(): ColumnWidths {
  const defaults = defaultColumnWidths();
  const raw = saved(COLUMN_WIDTHS_KEY);
  if (!raw) return defaults;
  try {
    const parsed = JSON.parse(raw) as Partial<Record<ColumnKey, unknown>>;
    for (const key of columnKeys) {
      if (typeof parsed[key] === "number" && Number.isFinite(parsed[key])) {
        defaults[key] = clampColumnWidth(key, parsed[key]);
      }
    }
  } catch {}
  return defaults;
}

function persistColumnWidths(widths: ColumnWidths) {
  persist(COLUMN_WIDTHS_KEY, JSON.stringify(widths));
}

export default function App() {
  const [calls, setCalls] = useState<Map<string, CallRecord>>(new Map());
  const [catalog, setCatalog] = useState<Catalog>({ projects: [], conversations: [] });
  const [status, setStatus] = useState<MonitorStatus | null>(null);
  const [projectId, setProjectId] = useState(() => saved("wtm.project"));
  const [conversationId, setConversationId] = useState(() => saved("wtm.conversation"));
  const [sort, setSort] = useState<"desc" | "asc">("desc");
  const [dateFrom, setDateFrom] = useState(() => saved("wtm.dateFrom"));
  const [dateTo, setDateTo] = useState(() => saved("wtm.dateTo"));
  const [columnWidths, setColumnWidths] = useState<ColumnWidths>(loadColumnWidths);
  const [follow, setFollow] = useState(true);
  const [live, setLive] = useState(false);
  const [contextMenu, setContextMenu] = useState<{ x: number; y: number; conversationId: string } | null>(null);
  const scrollRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    let active = true;
    const unlisten: Array<() => void> = [];
    void (async () => {
      const [callOff, catalogOff, statusOff] = await Promise.all([
        listen<CallRecord>("monitor-call", ({ payload }) => {
          if (!active) return;
          setCalls((current) => {
            const next = new Map(current);
            next.set(payload.id, payload);
            return next;
          });
        }),
        listen<Catalog>("monitor-catalog", ({ payload }) => active && setCatalog(payload)),
        listen<MonitorStatus>("monitor-status", ({ payload }) => active && setStatus(payload)),
      ]);
      unlisten.push(callOff, catalogOff, statusOff);
      const snapshot = await invoke<Snapshot>("get_snapshot");
      if (!active) return;
      setCalls(new Map(snapshot.calls.map((call) => [call.id, call])));
      setCatalog(snapshot.catalog);
      setStatus(snapshot.status);
      setLive(true);
    })().catch((error) => {
      console.error("Failed to initialize monitor", error);
      setLive(false);
    });
    return () => {
      active = false;
      unlisten.forEach((off) => off());
    };
  }, []);

  useEffect(() => {
    if (projectId && !catalog.projects.some((project) => project.id === projectId)) setProjectId("");
  }, [catalog.projects, projectId]);

  useEffect(() => {
    const close = () => setContextMenu(null);
    const onKey = (event: KeyboardEvent) => { if (event.key === "Escape") close(); };
    window.addEventListener("pointerdown", close);
    window.addEventListener("resize", close);
    window.addEventListener("keydown", onKey);
    return () => {
      window.removeEventListener("pointerdown", close);
      window.removeEventListener("resize", close);
      window.removeEventListener("keydown", onKey);
    };
  }, []);

  const conversationsForProject = useMemo(() => catalog.conversations.filter((conversation) => !projectId || conversation.projectId === projectId), [catalog.conversations, projectId]);

  useEffect(() => {
    if (conversationId && !conversationsForProject.some((conversation) => conversation.id === conversationId)) setConversationId("");
  }, [conversationId, conversationsForProject]);

  const conversationById = useMemo(() => new Map(catalog.conversations.map((conversation) => [conversation.id, conversation])), [catalog.conversations]);

  const rows = useMemo(() => {
    const fromMs = dateBoundary(dateFrom, false);
    const toMs = dateBoundary(dateTo, true);
    const result = [...calls.values()].filter((call) => {
      if (conversationId && call.conversationId !== conversationId) return false;
      if (!conversationId && projectId && conversationById.get(call.conversationId)?.projectId !== projectId) return false;
      const timestamp = Date.parse(call.timestamp);
      if (fromMs !== null && (!Number.isFinite(timestamp) || timestamp < fromMs)) return false;
      if (toMs !== null && (!Number.isFinite(timestamp) || timestamp >= toMs)) return false;
      return true;
    });
    result.sort((a, b) => {
      const delta = a.timestamp.localeCompare(b.timestamp);
      return sort === "asc" ? delta : -delta;
    });
    return result;
  }, [calls, conversationId, conversationById, dateFrom, dateTo, projectId, sort]);

  useEffect(() => {
    if (!follow || rows.length === 0) return;
    requestAnimationFrame(() => {
      const target = scrollRef.current;
      if (!target) return;
      if (sort === "desc") target.scrollTop = 0;
      else target.scrollTop = target.scrollHeight;
    });
  }, [rows.length, follow, sort]);

  const chooseProject = (id: string) => {
    setProjectId(id);
    persist("wtm.project", id);
    setConversationId("");
    persist("wtm.conversation", "");
  };

  const chooseConversation = (id: string) => {
    setConversationId(id);
    persist("wtm.conversation", id);
  };

  const changeDateFrom = (value: string) => { setDateFrom(value); persist("wtm.dateFrom", value); };
  const changeDateTo = (value: string) => { setDateTo(value); persist("wtm.dateTo", value); };
  const clearDates = () => { changeDateFrom(""); changeDateTo(""); };
  const clearAllFilters = () => { chooseProject(""); clearDates(); };
  const resizeColumn = (key: ColumnKey, width: number, save: boolean) => {
    setColumnWidths((current) => {
      const next = { ...current, [key]: clampColumnWidth(key, width) };
      if (save) persistColumnWidths(next);
      return next;
    });
  };
  const resetColumn = (key: ColumnKey) => resizeColumn(key, COLUMN_CONFIG[key].defaultWidth, true);

  const openConversationMenu = (event: React.MouseEvent, id: string) => {
    event.preventDefault();
    event.stopPropagation();
    const x = Math.max(8, Math.min(event.clientX, window.innerWidth - 250));
    const y = Math.max(8, Math.min(event.clientY, window.innerHeight - 92));
    setContextMenu({ x, y, conversationId: id });
  };

  const copyConversationId = async () => {
    if (!contextMenu) return;
    try { await writeText(contextMenu.conversationId); }
    catch (error) { console.error("Failed to copy conversation ID", error); }
    finally { setContextMenu(null); }
  };

  const projectOptions: PickerOption[] = catalog.projects.map((project) => ({
    id: project.id,
    primary: `${project.name} · ${project.conversations}`,
    secondary: project.path || "未分配项目",
  }));
  const conversationOptions: PickerOption[] = conversationsForProject.map((conversation) => ({
    id: conversation.id,
    primary: conversation.title,
    secondary: conversation.id,
  }));

  const totalInput = rows.reduce((sum, call) => sum + call.usage.inputTokens, 0);
  const totalCached = rows.reduce((sum, call) => sum + call.usage.cachedInputTokens, 0);
  const totalTokens = rows.reduce((sum, call) => sum + call.usage.totalTokens, 0);
  const visibleConversations = new Set(rows.map((call) => call.conversationId)).size;
  const newest = rows.reduce<CallRecord | null>((best, call) => !best || call.timestamp > best.timestamp ? call : best, null);
  const tableWidth = columnKeys.reduce((sum, key) => sum + columnWidths[key], 0);

  return (
    <main className="shell">
      <header className="topbar">
        <div>
          <p className="eyebrow">TAURI · LOCAL · READ ONLY</p>
          <h1>Work Token Monitor</h1>
          <p className="subtitle">实时查看 ChatGPT Work / Codex 每一次模型调用的 Input、Cached、Output 与缓存命中率。</p>
        </div>
        <div className={`live-badge ${live ? "live" : "connecting"}`}><span className="dot" /><span>{live ? "Live" : "Starting"}</span></div>
      </header>

      <section className="toolbar" aria-label="Filters">
        <Picker label="Project" selectedId={projectId} allLabel="All Projects" allSecondary={`${catalog.projects.length} 个项目`} options={projectOptions} onSelect={chooseProject} />
        <Picker label="Conversation" selectedId={conversationId} allLabel="All Conversations" allSecondary={`${conversationsForProject.length} 个会话`} options={conversationOptions} wide onSelect={chooseConversation} />
        <label className="field compact-field"><span>Sort</span><select value={sort} onChange={(event) => setSort(event.target.value as "asc" | "desc")}><option value="desc">最新优先</option><option value="asc">最早优先</option></select></label>
        <label className="toggle-field"><input type="checkbox" checked={follow} onChange={(event) => setFollow(event.target.checked)} /><span>Follow latest</span></label>
        <button className="ghost-button" type="button" onClick={clearAllFilters}>清除筛选</button>
      </section>

      <section className="summary" aria-label="Summary">
        <div><span>Visible calls</span><strong>{integer.format(rows.length)}</strong></div>
        <div><span>Conversations</span><strong>{integer.format(visibleConversations)}</strong></div>
        <div title={`${integer.format(totalTokens)} tokens`}><span>Total tokens</span><strong>{formatTokenTotal(totalTokens)}</strong></div>
        <div><span>Average cache hit</span><strong>{totalInput ? `${(totalCached / totalInput * 100).toFixed(1)}%` : "—"}</strong></div>
        <div><span>Last update</span><strong>{newest ? formatTime(newest.timestamp) : "—"}</strong></div>
      </section>

      <section className="table-card">
        <div className="table-scroll" ref={scrollRef}>
          <table style={{ width: `max(100%, ${tableWidth}px)` }}>
            <colgroup>
              {columnKeys.map((key) => <col key={key} style={{ width: columnWidths[key] }} />)}
            </colgroup>
            <thead><tr>
              <ColumnHeader column="time" width={columnWidths.time} onResize={resizeColumn} onReset={resetColumn}><DateFilter from={dateFrom} to={dateTo} onFrom={changeDateFrom} onTo={changeDateTo} onClear={clearDates} /></ColumnHeader>
              <ColumnHeader column="project" width={columnWidths.project} onResize={resizeColumn} onReset={resetColumn}>Project</ColumnHeader>
              <ColumnHeader column="conversation" width={columnWidths.conversation} onResize={resizeColumn} onReset={resetColumn}>Conversation</ColumnHeader>
              <ColumnHeader column="model" width={columnWidths.model} onResize={resizeColumn} onReset={resetColumn}>Model</ColumnHeader>
              <ColumnHeader column="input" width={columnWidths.input} numeric onResize={resizeColumn} onReset={resetColumn}>Input</ColumnHeader>
              <ColumnHeader column="cached" width={columnWidths.cached} numeric onResize={resizeColumn} onReset={resetColumn}>Cached</ColumnHeader>
              <ColumnHeader column="output" width={columnWidths.output} numeric onResize={resizeColumn} onReset={resetColumn}>Output</ColumnHeader>
              <ColumnHeader column="cache" width={columnWidths.cache} numeric onResize={resizeColumn} onReset={resetColumn}>Cache hit</ColumnHeader>
            </tr></thead>
            <tbody>
              {rows.map((call) => <CallRow key={call.id} call={call} conversation={conversationById.get(call.conversationId)} onConversation={chooseConversation} onProject={chooseProject} onConversationMenu={openConversationMenu} />)}
            </tbody>
          </table>
          {rows.length === 0 && <div className="empty-state">当前筛选条件下还没有模型调用。</div>}
        </div>
      </section>

      <footer>
        <span>{status ? `Rust 监控 · ${status.files} files · ${status.pollMs} ms polling · ${status.parseErrors} errors` : "正在初始化 Rust 监控…"}</span>
        <span>不读取对话正文 · 不读取 auth.json · 不上传数据</span>
      </footer>

      {contextMenu && (
        <div className="conversation-context-menu" style={{ left: contextMenu.x, top: contextMenu.y }} onPointerDown={(event) => event.stopPropagation()} onContextMenu={(event) => event.preventDefault()}>
          <button type="button" onClick={() => void copyConversationId()}>复制会话 ID</button>
          <small>{contextMenu.conversationId}</small>
        </div>
      )}
    </main>
  );
}

function ColumnHeader({ column, width, numeric = false, children, onResize, onReset }: { column: ColumnKey; width: number; numeric?: boolean; children: React.ReactNode; onResize: (key: ColumnKey, width: number, save: boolean) => void; onReset: (key: ColumnKey) => void }) {
  const config = COLUMN_CONFIG[column];
  const beginResize = (event: React.PointerEvent<HTMLSpanElement>) => {
    if (event.button !== 0) return;
    event.preventDefault();
    event.stopPropagation();
    const target = event.currentTarget;
    const pointerId = event.pointerId;
    const startX = event.clientX;
    const startWidth = width;
    let latestWidth = width;
    document.body.classList.add("column-resizing");
    target.setPointerCapture(pointerId);
    const move = (moveEvent: PointerEvent) => {
      latestWidth = clampColumnWidth(column, startWidth + moveEvent.clientX - startX);
      onResize(column, latestWidth, false);
    };
    const finish = () => {
      if (target.hasPointerCapture(pointerId)) target.releasePointerCapture(pointerId);
      target.removeEventListener("pointermove", move);
      target.removeEventListener("pointerup", finish);
      target.removeEventListener("pointercancel", finish);
      document.body.classList.remove("column-resizing");
      onResize(column, latestWidth, true);
    };
    target.addEventListener("pointermove", move);
    target.addEventListener("pointerup", finish);
    target.addEventListener("pointercancel", finish);
  };
  const changeWithKeyboard = (event: React.KeyboardEvent<HTMLSpanElement>) => {
    if (event.key !== "ArrowLeft" && event.key !== "ArrowRight") return;
    event.preventDefault();
    const step = event.shiftKey ? 25 : 10;
    onResize(column, width + (event.key === "ArrowRight" ? step : -step), true);
  };
  return (
    <th className={`resizable-header${numeric ? " numeric" : ""}`}>
      {children}
      <span
        className="column-resizer"
        role="separator"
        aria-label={`调整 ${column} 列宽`}
        aria-orientation="vertical"
        aria-valuemin={config.minWidth}
        aria-valuemax={MAX_COLUMN_WIDTH}
        aria-valuenow={width}
        tabIndex={0}
        title="拖动调整列宽；双击恢复默认宽度"
        onPointerDown={beginResize}
        onDoubleClick={() => onReset(column)}
        onKeyDown={changeWithKeyboard}
      />
    </th>
  );
}

function formatTokenTotal(value: number) {
  if (value < 1_000) return integer.format(value);
  const units: Array<[number, string]> = [[1_000_000_000, "B"], [1_000_000, "M"], [1_000, "K"]];
  const [divisor, suffix] = units.find(([divisor]) => value >= divisor) ?? units[units.length - 1];
  const scaled = value / divisor;
  const digits = scaled >= 100 ? 0 : scaled >= 10 ? 1 : 2;
  return `${scaled.toFixed(digits).replace(/\.0+$|(?<=\.[0-9])0+$/, "")}${suffix}`;
}

function formatEffort(value: string) {
  const normalized = value.trim().toLowerCase();
  if (normalized === "xhigh" || normalized === "extra_high" || normalized === "extra-high") return "Extra High";
  return normalized.split(/[_-]+/).filter(Boolean).map((part) => part.charAt(0).toUpperCase() + part.slice(1)).join(" ") || value;
}

function CallRow({ call, conversation, onConversation, onProject, onConversationMenu }: { call: CallRecord; conversation?: Conversation; onConversation: (id: string) => void; onProject: (id: string) => void; onConversationMenu: (event: React.MouseEvent, id: string) => void }) {
  return (
    <tr>
      <td className="time-cell" title={call.timestamp}>{formatTime(call.timestamp)}</td>
      <td className="project-cell"><button className="plain-filter-button" type="button" onClick={() => conversation && onProject(conversation.projectId)}>{conversation?.projectName || "Unassigned"}</button></td>
      <td className="conversation-cell"><button className="conversation-button" type="button" onClick={() => onConversation(call.conversationId)} onContextMenu={(event) => onConversationMenu(event, call.conversationId)} title={`${call.conversationId} · 右键复制 ID`}><strong>{conversation?.title || "Untitled conversation"}</strong><small>{call.conversationId}</small></button></td>
      <td className="model-cell" title={call.effort ? `Reasoning effort: ${formatEffort(call.effort)}` : undefined}>
        <div className="model-stack"><strong>{call.model || "unknown"}</strong>{call.effort && <small>{formatEffort(call.effort)}</small>}</div>
      </td>
      <td className="numeric token-input">{integer.format(call.usage.inputTokens)}</td>
      <td className="numeric token-cached">{integer.format(call.usage.cachedInputTokens)}</td>
      <td className="numeric token-output">{integer.format(call.usage.outputTokens)}</td>
      <td className="numeric cache-cell"><span className="cache-meter" style={{ "--cache": `${Math.max(0, Math.min(100, call.cacheHitRate))}%` } as React.CSSProperties} /><strong>{call.cacheHitRate.toFixed(2)}%</strong></td>
    </tr>
  );
}

function DateFilter({ from, to, onFrom, onTo, onClear }: { from: string; to: string; onFrom: (value: string) => void; onTo: (value: string) => void; onClear: () => void }) {
  const active = Boolean(from || to);
  return (
    <details className={`date-filter${active ? " active" : ""}`}>
      <summary title="筛选日期范围"><span>Time</span><span className="date-filter-arrow">▾</span></summary>
      <div className="date-filter-menu" onClick={(event) => event.stopPropagation()}>
        <label><span>From</span><input type="date" value={from} onChange={(event) => onFrom(event.target.value)} /></label>
        <label><span>To</span><input type="date" value={to} onChange={(event) => onTo(event.target.value)} /></label>
        <button type="button" onClick={onClear} disabled={!active}>清除日期</button>
      </div>
    </details>
  );
}

function dateBoundary(value: string, exclusiveEnd: boolean) {
  if (!value) return null;
  const date = new Date(`${value}T00:00:00`);
  if (Number.isNaN(date.getTime())) return null;
  if (exclusiveEnd) date.setDate(date.getDate() + 1);
  return date.getTime();
}

function formatTime(timestamp: string) {
  const date = new Date(timestamp);
  return Number.isNaN(date.getTime()) ? timestamp || "—" : timeFormat.format(date);
}
