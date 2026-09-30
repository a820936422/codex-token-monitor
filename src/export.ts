import { summarize, type Filters } from "./data";
import type { CallRecord, Conversation, MonitorStatus } from "./types";

export function csvCell(value: unknown): string {
  let text = value == null ? "" : String(value);
  if (typeof value === "string" && /^[\s\uFEFF]*[=+\-@]/.test(text)) text = "'" + text;
  return '"' + text.replace(/"/g, '""') + '"';
}
export function buildExport(
  calls: CallRecord[],
  conversations: Map<string, Conversation>,
  status: MonitorStatus,
  filters: Filters,
  identifying: boolean,
  format: "csv" | "json",
): string {
  const projects = new Map<string, string>(),
    sessions = new Map<string, string>();
  const alias = (map: Map<string, string>, id: string, prefix: string) => {
    if (!map.has(id)) map.set(id, `${prefix}-${map.size + 1}`);
    return map.get(id)!;
  };
  const rows = calls.map((call) => {
    const conversation = conversations.get(call.conversationId);
    const project = conversation?.projectId ?? "__unassigned__";
    const row = {
      timestamp: call.timestamp,
      project: identifying
        ? (conversation?.projectName ?? "Unassigned")
        : alias(projects, project, "project"),
      conversation: identifying
        ? (conversation?.title ?? "Untitled")
        : alias(sessions, call.conversationId, "conversation"),
      model: call.model,
      effort: call.effort,
      serviceTier: call.serviceTier,
      ...call.usage,
      cacheHitRate: call.cacheHitRate,
      issues: call.issues.join(";"),
    };
    return identifying
      ? {
          ...row,
          responseId: call.responseId,
          threadId: call.threadId,
          conversationId: call.conversationId,
          turnId: call.turnId,
          projectPath: conversation?.projectPath ?? null,
        }
      : row;
  });
  const scope = {
    kind: "filtered_retained_window",
    revision: status.revision,
    retainedRecords: status.records,
    exportedRecords: rows.length,
    retentionLimit: status.retentionLimit,
    truncated: status.truncated,
    indexing: status.discovering || status.pendingFiles > 0,
    partial: Boolean(
      status.unreadableFiles ||
      status.missingRoots ||
      status.discoveryErrors ||
      status.parseErrors ||
      status.invalidRecords ||
      status.oversizedLines ||
      status.fileLimitReached ||
      status.metadataLimitReached,
    ),
    oldestRetainedAt: status.oldestAt,
    newestRetainedAt: status.newestAt,
    fromLocalDate: filters.from || null,
    toLocalDate: filters.to || null,
    projectFilter: identifying ? filters.projectId || null : filters.projectId ? "selected" : "all",
    conversationFilter: identifying
      ? filters.conversationId || null
      : filters.conversationId
        ? "selected"
        : "all",
    redacted: !identifying,
  };
  if (format === "json") {
    const totals = summarize(calls);
    const summary = Object.fromEntries(
      Object.entries(totals).map(([key, value]) => [
        key,
        typeof value === "bigint" ? value.toString() : value,
      ]),
    );
    return (
      JSON.stringify(
        {
          schemaVersion: 1,
          exportedAt: new Date().toISOString(),
          scope,
          summary,
          aggregateIntegerEncoding: "decimal strings",
          rows,
        },
        null,
        2,
      ) + "\n"
    );
  }
  // Scope travels with each CSV row, so a detached CSV cannot claim full-history totals.
  const records: Record<string, unknown>[] = rows.map((row) => ({ ...scope, ...row }));
  const columns = Object.keys(records[0] ?? scope);
  return (
    "\uFEFF" +
    [
      columns.map(csvCell).join(","),
      ...records.map((row) => columns.map((key) => csvCell(row[key])).join(",")),
    ].join("\r\n") +
    "\r\n"
  );
}
