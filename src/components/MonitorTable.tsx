import { memo, useEffect, useMemo, useRef } from "react";
import type { CallRecord, Conversation } from "../types";
import { cacheRate, formatTokenTotal, groupCalls, integer, known, type Group } from "../data";
import { formatTime, formatEffort } from "../format";
import { useVirtualWindow } from "../useVirtualWindow";
import { columnKeys, type ColumnKey, type ColumnWidths } from "../columns";
import { ColumnHeader } from "./ColumnHeader";

export type ViewMode = "calls" | "project" | "conversation";
interface Props {
  rows: CallRecord[];
  conversations: Map<string, Conversation>;
  view: ViewMode;
  compact: boolean;
  widths: ColumnWidths;
  follow: boolean;
  sort: "asc" | "desc";
  filterKey: string;
  emptyMessage: string;
  onResize: (key: ColumnKey, width: number, save: boolean) => void;
  onReset: (key: ColumnKey) => void;
  onProject: (id: string) => void;
  onConversation: (id: string) => void;
  onMenu: (event: React.MouseEvent, id: string) => void;
}
const labels: Record<ColumnKey, string> = {
  time: "Time",
  project: "Project",
  conversation: "Conversation",
  model: "日志模型",
  input: "Input",
  cached: "Cached",
  output: "Output",
  cache: "Cache hit",
};
const countText = (value: number | null) => (known(value) ? integer.format(value) : "—");

export function MonitorTable(props: Props) {
  const { rows, conversations, view, compact, widths, sort, follow, filterKey } = props;
  const ref = useRef<HTMLDivElement>(null);
  const groups = useMemo(
    () => (view === "calls" ? [] : groupCalls(rows, conversations, view)),
    [rows, conversations, view],
  );
  const count = view === "calls" ? rows.length : groups.length;
  const window = useVirtualWindow(count, ref);
  const keys = columnKeys.filter(
    (key) => !compact || (key !== "project" && key !== "conversation"),
  );
  const columns = view === "calls" ? keys.length : 6;
  const newest = sort === "desc" ? rows[0] : rows[rows.length - 1];
  useEffect(() => {
    if (ref.current) ref.current.scrollTop = 0;
  }, [filterKey, view, sort]);
  useEffect(() => {
    if (!follow || view !== "calls") return;
    const frame = requestAnimationFrame(() => {
      if (ref.current) ref.current.scrollTop = sort === "desc" ? 0 : ref.current.scrollHeight;
    });
    return () => cancelAnimationFrame(frame);
  }, [follow, view, sort, newest?.id, filterKey]);
  return (
    <section className="table-card">
      <div className="table-scroll" ref={ref} tabIndex={0} aria-label="调用记录表格，可滚动浏览">
        <table
          aria-rowcount={count + 1}
          style={{
            width:
              view === "calls"
                ? `max(100%, ${keys.reduce((sum, key) => sum + widths[key], 0)}px)`
                : "max(100%, 850px)",
          }}
        >
          {view === "calls" && (
            <colgroup>
              {keys.map((key) => (
                <col key={key} style={{ width: widths[key] }} />
              ))}
            </colgroup>
          )}
          <thead>
            <tr>
              {view === "calls" ? (
                keys.map((key) => (
                  <ColumnHeader
                    key={key}
                    column={key}
                    width={widths[key]}
                    numeric={["input", "cached", "output", "cache"].includes(key)}
                    onResize={props.onResize}
                    onReset={props.onReset}
                  >
                    <span
                      title={key === "model" ? "日志中的模型名称，不是后端模型身份验证" : undefined}
                    >
                      {labels[key]}
                    </span>
                  </ColumnHeader>
                ))
              ) : (
                <>
                  <th style={{ width: "30%" }}>{view === "project" ? "项目汇总" : "会话汇总"}</th>
                  <th className="numeric">调用数</th>
                  <th className="numeric">Input</th>
                  <th className="numeric">Output</th>
                  <th className="numeric">Total tokens</th>
                  <th className="numeric">Cache hit</th>
                </>
              )}
            </tr>
          </thead>
          <tbody>
            {window.before > 0 && (
              <tr className="virtual-spacer" aria-hidden="true">
                <td colSpan={columns} style={{ height: window.before }} />
              </tr>
            )}
            {view === "calls"
              ? rows
                  .slice(window.start, window.end)
                  .map((call, index) => (
                    <CallRow
                      key={call.id}
                      call={call}
                      index={window.start + index}
                      conversation={conversations.get(call.conversationId)}
                      compact={compact}
                      onProject={props.onProject}
                      onConversation={props.onConversation}
                      onMenu={props.onMenu}
                    />
                  ))
              : groups
                  .slice(window.start, window.end)
                  .map((group, index) => (
                    <GroupRow
                      key={group.id}
                      group={group}
                      index={window.start + index}
                      onSelect={view === "project" ? props.onProject : props.onConversation}
                    />
                  ))}
            {window.after > 0 && (
              <tr className="virtual-spacer" aria-hidden="true">
                <td colSpan={columns} style={{ height: window.after }} />
              </tr>
            )}
          </tbody>
        </table>
        {!count && (
          <div className="empty-state" role="status">
            {props.emptyMessage}
          </div>
        )}
      </div>
      <div className="table-range">
        {count ? `${window.start + 1}–${window.end} / ${integer.format(count)}` : "0"} ·{" "}
        {view === "calls" ? "筛选调用" : "汇总条目"} · 仅渲染可见行，统计与导出包含全部筛选调用
      </div>
    </section>
  );
}
const CallRow = memo(function CallRow({
  call,
  index,
  conversation,
  compact,
  onConversation,
  onProject,
  onMenu,
}: {
  call: CallRecord;
  index: number;
  conversation?: Conversation;
  compact: boolean;
  onConversation: (id: string) => void;
  onProject: (id: string) => void;
  onMenu: (event: React.MouseEvent, id: string) => void;
}) {
  return (
    <tr data-call-row="true" aria-rowindex={index + 2}>
      <td className="time-cell" title={call.timestamp}>
        {formatTime(call.timestamp)}
      </td>
      {!compact && (
        <td className="project-cell">
          <button
            className="plain-filter-button"
            onClick={() => conversation && onProject(conversation.projectId)}
          >
            {conversation?.projectName || "Unassigned"}
          </button>
        </td>
      )}
      {!compact && (
        <td className="conversation-cell">
          <button
            className="conversation-button"
            onClick={() => onConversation(call.conversationId)}
            onContextMenu={(event) => onMenu(event, call.conversationId)}
            title={`${call.conversationId} · 右键复制 ID`}
          >
            <strong>{conversation?.title || "Untitled conversation"}</strong>
            <small>{call.conversationId}</small>
          </button>
        </td>
      )}
      <td
        className="model-cell"
        title={`${conversation?.title ?? ""}\n日志模型：${call.model}\n${call.issues.join(" · ")}`}
      >
        <div className="model-stack">
          <strong>
            {call.model || "unknown"}
            {call.issues.length ? " ⚠" : ""}
          </strong>
          <small>{call.effort ? formatEffort(call.effort) : "推理强度未知"}</small>
        </div>
      </td>
      <td
        className="numeric token-input"
        title={call.usage.inputTokens === null ? "字段缺失或无效，不计作零" : undefined}
      >
        {countText(call.usage.inputTokens)}
      </td>
      <td className="numeric token-cached">{countText(call.usage.cachedInputTokens)}</td>
      <td className="numeric token-output">{countText(call.usage.outputTokens)}</td>
      <td className="numeric cache-cell">
        {call.cacheHitRate === null ? (
          "—"
        ) : (
          <>
            <span
              className="cache-meter"
              style={
                {
                  "--cache": `${Math.max(0, Math.min(100, call.cacheHitRate))}%`,
                } as React.CSSProperties
              }
            />
            <strong>{call.cacheHitRate.toFixed(2)}%</strong>
          </>
        )}
      </td>
    </tr>
  );
});
function GroupRow({
  group,
  index,
  onSelect,
}: {
  group: Group;
  index: number;
  onSelect: (id: string) => void;
}) {
  const t = group.totals,
    rate = cacheRate(t);
  const value = (amount: bigint, measured: number) =>
    measured ? `${formatTokenTotal(amount)}${measured < t.count ? "*" : ""}` : "—";
  return (
    <tr data-group-row="true" aria-rowindex={index + 2}>
      <td>
        <button
          className="conversation-button"
          onClick={() => onSelect(group.id)}
          title={group.label}
        >
          <strong>{group.label}</strong>
          <small>{t.issues ? `${t.issues} 条记录字段不完整或异常` : "当前筛选范围"}</small>
        </button>
      </td>
      <td className="numeric">{integer.format(t.count)}</td>
      <td className="numeric token-input" title={t.input.toString()}>
        {value(t.input, t.inputKnown)}
      </td>
      <td className="numeric token-output" title={t.output.toString()}>
        {value(t.output, t.outputKnown)}
      </td>
      <td className="numeric" title={t.total.toString()}>
        {value(t.total, t.totalKnown)}
      </td>
      <td className="numeric">
        {rate === null ? "—" : `${rate.toFixed(1)}%${t.cachePairs < t.count ? "*" : ""}`}
      </td>
    </tr>
  );
}
