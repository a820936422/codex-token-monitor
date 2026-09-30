import { useCallback, useEffect, useMemo, useState } from "react";
import { writeText } from "@tauri-apps/plugin-clipboard-manager";
import { Picker } from "./components/Picker";
import { DateFilter } from "./components/DateFilter";
import { Diagnostics, monitorHealth } from "./components/Diagnostics";
import { ExportControls } from "./components/ExportControls";
import { MonitorTable, type ViewMode } from "./components/MonitorTable";
import { useMonitor } from "./useMonitor";
import {
  COLUMN_CONFIG,
  clampColumnWidth,
  loadColumnWidths,
  persistColumnWidths,
  saved,
  persist,
  type ColumnKey,
} from "./columns";
import {
  cacheRate,
  filterCalls,
  filterError,
  formatTokenTotal,
  integer,
  ordered,
  recentDays,
  summarize,
} from "./data";
import { formatTime } from "./format";

export default function App() {
  const { calls, catalog, status, error, syncing, stale, retry } = useMonitor();
  const [projectId, setProjectId] = useState(() => saved("wtm.project"));
  const [conversationId, setConversationId] = useState(() => saved("wtm.conversation"));
  const [sort, setSort] = useState<"asc" | "desc">(() =>
    saved("wtm.sort") === "asc" ? "asc" : "desc",
  );
  const [dateFrom, setDateFrom] = useState(() => saved("wtm.dateFrom"));
  const [dateTo, setDateTo] = useState(() => saved("wtm.dateTo"));
  const [widths, setWidths] = useState(loadColumnWidths);
  const [follow, setFollow] = useState(() => saved("wtm.follow") !== "false");
  const [view, setView] = useState<ViewMode>("calls");
  const [preset, setPreset] = useState(() =>
    ["full", "compact"].includes(saved("wtm.columns")) ? saved("wtm.columns") : "auto",
  );
  const [narrow, setNarrow] = useState(() => matchMedia("(max-width: 1100px)").matches);
  const [contextMenu, setContextMenu] = useState<{ x: number; y: number; id: string } | null>(null);
  const [copyMessage, setCopyMessage] = useState("");
  const compact = preset === "compact" || (preset === "auto" && narrow);
  const health = monitorHealth(status, syncing, stale, error);
  useEffect(() => {
    const media = matchMedia("(max-width: 1100px)");
    const change = () => setNarrow(media.matches);
    media.addEventListener("change", change);
    return () => media.removeEventListener("change", change);
  }, []);
  useEffect(() => {
    const close = () => setContextMenu(null);
    const key = (event: KeyboardEvent) => {
      if (event.key === "Escape") close();
    };
    window.addEventListener("pointerdown", close);
    window.addEventListener("resize", close);
    window.addEventListener("keydown", key);
    return () => {
      window.removeEventListener("pointerdown", close);
      window.removeEventListener("resize", close);
      window.removeEventListener("keydown", key);
    };
  }, []);
  const conversationById = useMemo(
    () => new Map(catalog.conversations.map((c) => [c.id, c])),
    [catalog.conversations],
  );
  const availableConversations = useMemo(
    () => catalog.conversations.filter((c) => !projectId || c.projectId === projectId),
    [catalog.conversations, projectId],
  );
  useEffect(() => {
    if (syncing || !status?.initialLoadComplete || status.discovering || status.pendingFiles)
      return;
    const validProject = !projectId || catalog.projects.some((p) => p.id === projectId);
    if (!validProject) {
      setProjectId("");
      persist("wtm.project", "");
    }
    const conversation = conversationById.get(conversationId);
    if (
      conversationId &&
      (!conversation || (validProject && projectId && conversation.projectId !== projectId))
    ) {
      setConversationId("");
      persist("wtm.conversation", "");
    }
  }, [
    syncing,
    status?.initialLoadComplete,
    status?.discovering,
    status?.pendingFiles,
    catalog.projects,
    projectId,
    conversationId,
    conversationById,
  ]);
  const filters = useMemo(
    () => ({ projectId, conversationId, from: dateFrom, to: dateTo }),
    [projectId, conversationId, dateFrom, dateTo],
  );
  const filterKey = JSON.stringify(filters),
    dateError = filterError(filters);
  const sorted = useMemo(() => ordered(calls.values()), [calls]);
  const filtered = useMemo(
    () => filterCalls(sorted, filters, conversationById),
    [sorted, filters, conversationById],
  );
  const rows = useMemo(
    () => (sort === "desc" ? filtered : [...filtered].reverse()),
    [filtered, sort],
  );
  const totals = useMemo(() => summarize(filtered), [filtered]);
  const visibleConversations = useMemo(
    () => new Set(filtered.map((c) => c.conversationId)).size,
    [filtered],
  );
  const rate = cacheRate(totals),
    newest = filtered[0];
  const chooseProject = useCallback((id: string) => {
    setProjectId(id);
    persist("wtm.project", id);
    setConversationId("");
    persist("wtm.conversation", "");
  }, []);
  const chooseConversation = useCallback((id: string) => {
    setConversationId(id);
    persist("wtm.conversation", id);
  }, []);
  const from = (value: string) => {
    setDateFrom(value);
    persist("wtm.dateFrom", value);
  };
  const to = (value: string) => {
    setDateTo(value);
    persist("wtm.dateTo", value);
  };
  const clearDates = () => {
    from("");
    to("");
  };
  const quickDate = (days: number) => {
    const [a, b] = recentDays(days);
    from(a);
    to(b);
  };
  const resize = useCallback((key: ColumnKey, width: number, save: boolean) => {
    setWidths((old) => {
      const next = { ...old, [key]: clampColumnWidth(key, width) };
      if (save) persistColumnWidths(next);
      return next;
    });
  }, []);
  const resetColumn = useCallback(
    (key: ColumnKey) => resize(key, COLUMN_CONFIG[key].defaultWidth, true),
    [resize],
  );
  const openMenu = useCallback((event: React.MouseEvent, id: string) => {
    event.preventDefault();
    event.stopPropagation();
    setCopyMessage("");
    setContextMenu({
      x: Math.max(8, Math.min(event.clientX, innerWidth - 250)),
      y: Math.max(8, Math.min(event.clientY, innerHeight - 92)),
      id,
    });
  }, []);
  const copyId = async () => {
    if (!contextMenu) return;
    try {
      await writeText(contextMenu.id);
      setCopyMessage("已复制会话 ID");
    } catch {
      setCopyMessage("复制失败，请检查桌面剪贴板权限。");
    } finally {
      setContextMenu(null);
    }
  };
  const projectOptions = useMemo(
    () =>
      catalog.projects.map((p) => ({
        id: p.id,
        primary: `${p.name} · ${p.conversations}`,
        secondary: p.path,
      })),
    [catalog.projects],
  );
  const conversationOptions = useMemo(
    () => availableConversations.map((c) => ({ id: c.id, primary: c.title, secondary: c.id })),
    [availableConversations],
  );
  const emptyMessage =
    error ??
    dateError ??
    (syncing || health === "Indexing"
      ? "正在加载本地调用记录…"
      : health === "Unsupported"
        ? "发现旧累计日志，但没有兼容的逐调用记录。请查看运行诊断。"
        : status?.missingRoots || status?.unreadableFiles
          ? "无法完整读取日志目录，请查看运行诊断。"
          : "当前筛选条件下还没有模型调用。");
  const totalLabel =
    totals.totalKnown || !totals.count
      ? `${formatTokenTotal(totals.total)}${totals.totalKnown < totals.count ? "*" : ""}`
      : "—";
  return (
    <main className="shell">
      <header className="topbar">
        <div>
          <p className="eyebrow">TAURI · LOCAL · READ ONLY</p>
          <h1>Work Token Monitor</h1>
          <p className="subtitle">只读本地日志，查看每次调用的 token 用量与缓存命中率。</p>
        </div>
        <div
          className={`live-badge ${health === "Live" ? "live" : "connecting"}`}
          title="表示监控状态，不表示模型服务或账号状态"
        >
          <span className="dot" />
          <span>{health}</span>
        </div>
      </header>
      <section className="toolbar" aria-label="Filters">
        <Picker
          label="Project"
          selectedId={projectId}
          allLabel="All Projects"
          allSecondary={`${catalog.projects.length} 个已加载项目`}
          options={projectOptions}
          onSelect={chooseProject}
        />
        <Picker
          label="Conversation"
          selectedId={conversationId}
          allLabel="All Conversations"
          allSecondary={`${availableConversations.length} 个已加载会话`}
          options={conversationOptions}
          wide
          onSelect={chooseConversation}
        />
        <label className="field compact-field">
          <span>Sort</span>
          <select
            aria-label="Sort"
            value={sort}
            onChange={(e) => {
              const v = e.target.value as "asc" | "desc";
              setSort(v);
              persist("wtm.sort", v);
            }}
          >
            <option value="desc">最新优先</option>
            <option value="asc">最早优先</option>
          </select>
        </label>
        <label className="toggle-field">
          <input
            type="checkbox"
            checked={follow}
            onChange={(e) => {
              setFollow(e.target.checked);
              persist("wtm.follow", String(e.target.checked));
            }}
          />
          <span>Follow latest</span>
        </label>
        <button
          className="ghost-button"
          onClick={() => {
            chooseProject("");
            clearDates();
          }}
        >
          清除筛选
        </button>
      </section>
      <section className="view-toolbar" aria-label="视图与导出">
        <div className="date-presets">
          <button onClick={() => quickDate(1)}>今天</button>
          <button onClick={() => quickDate(7)}>近 7 天</button>
          <button onClick={() => quickDate(30)}>近 30 天</button>
          <button onClick={clearDates}>全部已加载日期</button>
          <DateFilter from={dateFrom} to={dateTo} onFrom={from} onTo={to} onClear={clearDates} />
        </div>
        <label>
          视图
          <select
            aria-label="视图"
            value={view}
            onChange={(e) => setView(e.target.value as ViewMode)}
          >
            <option value="calls">逐调用</option>
            <option value="project">项目汇总</option>
            <option value="conversation">会话汇总</option>
          </select>
        </label>
        <label>
          列显示
          <select
            aria-label="列显示"
            value={preset}
            onChange={(e) => {
              setPreset(e.target.value);
              persist("wtm.columns", e.target.value);
            }}
          >
            <option value="auto">自适应</option>
            <option value="full">全部列</option>
            <option value="compact">紧凑列</option>
          </select>
        </label>
        <ExportControls
          calls={rows}
          conversations={conversationById}
          status={status}
          filters={filters}
          disabled={syncing || stale || Boolean(error || dateError)}
        />
      </section>
      <div className="scope-note">
        <strong>
          当前加载窗口：{integer.format(status?.records ?? 0)} /{" "}
          {integer.format(status?.retentionLimit ?? 50000)} 条
        </strong>
        <span>
          {status?.oldestAt && status.newestAt
            ? `${formatTime(status.oldestAt)} — ${formatTime(status.newestAt)}`
            : "尚无时间覆盖范围"}
        </span>
        <span>
          {status?.truncated
            ? "已截断：仅保留按时间排序的最近记录，不是完整历史总量。"
            : "统计只覆盖当前加载记录，不是完整历史账单。"}
        </span>
        {(status?.discovering || Boolean(status?.pendingFiles)) && (
          <span>索引进行中，范围可能继续变化。</span>
        )}
        {(dateFrom || dateTo) && (
          <span>
            日期筛选：{dateFrom || "不限"} — {dateTo || "不限"}（本地时区）
          </span>
        )}
      </div>
      {(error || stale || dateError) && (
        <div className="warning-banner" role="alert">
          {error ?? dateError ?? "监控心跳已超过 5 秒未更新，当前数据可能过期。"}
          {(error || stale) && (
            <button className="ghost-button" onClick={retry}>
              重新同步
            </button>
          )}
        </div>
      )}
      <section className="summary" aria-label="Summary">
        <div>
          <span>Visible calls</span>
          <strong>{integer.format(rows.length)}</strong>
          <small>当前筛选范围</small>
        </div>
        <div>
          <span>Conversations</span>
          <strong>{integer.format(visibleConversations)}</strong>
          <small>已加载会话</small>
        </div>
        <div title={`${integer.format(totals.total)} tokens`}>
          <span>Total tokens</span>
          <strong>{totalLabel}</strong>
          <small>
            有值 {totals.totalKnown} / {totals.count}；* 为已知部分小计
          </small>
        </div>
        <div>
          <span>Average cache hit</span>
          <strong>
            {rate === null
              ? "—"
              : `${rate.toFixed(1)}%${totals.cachePairs < totals.count ? "*" : ""}`}
          </strong>
          <small>
            按输入加权；配对字段 {totals.cachePairs} / {totals.count}
          </small>
        </div>
        <div title={newest?.timestamp}>
          <span>Last update</span>
          <strong>{newest ? formatTime(newest.timestamp) : "—"}</strong>
          <small>最新调用时间，不是扫描心跳</small>
        </div>
      </section>
      {totals.issues > 0 && (
        <p className="quality-note">
          当前筛选中 {totals.issues}{" "}
          条记录存在缺失字段或异常；“—”表示未知，不作为零值参与汇总。悬停日志模型可查看问题代码。
        </p>
      )}
      <MonitorTable
        rows={rows}
        conversations={conversationById}
        view={view}
        compact={compact}
        widths={widths}
        follow={follow}
        sort={sort}
        filterKey={filterKey}
        emptyMessage={emptyMessage}
        onResize={resize}
        onReset={resetColumn}
        onProject={chooseProject}
        onConversation={chooseConversation}
        onMenu={openMenu}
      />
      <Diagnostics status={status} />
      <footer>
        <span>
          {status
            ? `Rust · ${status.files} files · ${status.scanMs} ms / scan · revision ${status.revision}`
            : "正在初始化 Rust 监控…"}
        </span>
        <span>仅读取本地日志 · 不读取认证 · 不修改 Codex 配置 · 导出需手动选择文件</span>
      </footer>
      <p className="copy-feedback" aria-live="polite">
        {copyMessage}
      </p>
      {contextMenu && (
        <div
          className="conversation-context-menu"
          style={{ left: contextMenu.x, top: contextMenu.y }}
          onPointerDown={(e) => e.stopPropagation()}
          onContextMenu={(e) => e.preventDefault()}
        >
          <button onClick={() => void copyId()}>复制会话 ID</button>
          <small>{contextMenu.id}</small>
        </div>
      )}
    </main>
  );
}
