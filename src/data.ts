import type { CallRecord, Conversation } from "./types";

export const integer = new Intl.NumberFormat(undefined, { maximumFractionDigits: 0 });
export function known(value: number | null | undefined): value is number {
  return typeof value === "number" && Number.isSafeInteger(value) && value >= 0;
}
export interface Totals {
  count: number;
  input: bigint;
  cached: bigint;
  output: bigint;
  total: bigint;
  inputKnown: number;
  cachedKnown: number;
  outputKnown: number;
  totalKnown: number;
  cacheInput: bigint;
  cacheCached: bigint;
  cachePairs: number;
  issues: number;
}
export function emptyTotals(): Totals {
  return {
    count: 0,
    input: 0n,
    cached: 0n,
    output: 0n,
    total: 0n,
    inputKnown: 0,
    cachedKnown: 0,
    outputKnown: 0,
    totalKnown: 0,
    cacheInput: 0n,
    cacheCached: 0n,
    cachePairs: 0,
    issues: 0,
  };
}
function add(t: Totals, call: CallRecord) {
  t.count++;
  if (call.issues.length) t.issues++;
  const u = call.usage;
  if (known(u.inputTokens)) {
    t.input += BigInt(u.inputTokens);
    t.inputKnown++;
  }
  if (known(u.cachedInputTokens)) {
    t.cached += BigInt(u.cachedInputTokens);
    t.cachedKnown++;
  }
  if (known(u.outputTokens)) {
    t.output += BigInt(u.outputTokens);
    t.outputKnown++;
  }
  if (known(u.totalTokens)) {
    t.total += BigInt(u.totalTokens);
    t.totalKnown++;
  }
  if (known(u.inputTokens) && known(u.cachedInputTokens) && u.cachedInputTokens <= u.inputTokens) {
    t.cacheInput += BigInt(u.inputTokens);
    t.cacheCached += BigInt(u.cachedInputTokens);
    t.cachePairs++;
  }
}
export function summarize(calls: Iterable<CallRecord>): Totals {
  const totals = emptyTotals();
  for (const call of calls) add(totals, call);
  return totals;
}
export function cacheRate(t: Totals): number | null {
  return t.cacheInput > 0n ? (Number(t.cacheCached) / Number(t.cacheInput)) * 100 : null;
}
export function formatTokenTotal(value: bigint | number): string {
  const n = typeof value === "bigint" ? value : BigInt(Math.trunc(value));
  if (n < 1000n) return integer.format(n);
  const units: Array<[bigint, string]> = [
    [1_000_000_000n, "B"],
    [1_000_000n, "M"],
    [1000n, "K"],
  ];
  const [divisor, suffix] = units.find(([d]) => n >= d)!;
  const scaled = Number(n) / Number(divisor),
    digits = scaled >= 100 ? 0 : scaled >= 10 ? 1 : 2;
  return `${scaled.toFixed(digits).replace(/\.0+$|(?<=\.[0-9])0+$/, "")}${suffix}`;
}
export function dateBoundary(value: string, end: boolean): number | null {
  if (!/^\d{4}-\d{2}-\d{2}$/.test(value)) return null;
  const [year, month, day] = value.split("-").map(Number);
  const date = new Date(0);
  date.setHours(0, 0, 0, 0);
  date.setFullYear(year, month - 1, day);
  if (date.getFullYear() !== year || date.getMonth() !== month - 1 || date.getDate() !== day)
    return null;
  if (end) date.setDate(date.getDate() + 1);
  return date.getTime();
}
export function localDate(date: Date): string {
  return `${String(date.getFullYear()).padStart(4, "0")}-${String(date.getMonth() + 1).padStart(2, "0")}-${String(date.getDate()).padStart(2, "0")}`;
}
export function recentDays(days: number, now = new Date()): [string, string] {
  const start = new Date(now);
  start.setDate(start.getDate() - days + 1);
  return [localDate(start), localDate(now)];
}
export interface Filters {
  projectId: string;
  conversationId: string;
  from: string;
  to: string;
}
export function filterError(filters: Filters): string | null {
  const from = dateBoundary(filters.from, false),
    to = dateBoundary(filters.to, true);
  if ((filters.from && from === null) || (filters.to && to === null))
    return "日期格式无效，请重新选择。";
  if (from !== null && to !== null && from >= to) return "开始日期不能晚于结束日期。";
  return null;
}
export function ordered(calls: Iterable<CallRecord>): CallRecord[] {
  return [...calls].sort((a, b) => b.timestampMs - a.timestampMs || b.id.localeCompare(a.id));
}
export function filterCalls(
  calls: CallRecord[],
  filters: Filters,
  conversations: Map<string, Conversation>,
): CallRecord[] {
  if (filterError(filters)) return [];
  const from = dateBoundary(filters.from, false),
    to = dateBoundary(filters.to, true);
  return calls.filter(
    (call) =>
      (!filters.conversationId || call.conversationId === filters.conversationId) &&
      (!filters.projectId ||
        conversations.get(call.conversationId)?.projectId === filters.projectId) &&
      (from === null || call.timestampMs >= from) &&
      (to === null || call.timestampMs < to),
  );
}
export interface Group {
  id: string;
  label: string;
  totals: Totals;
  latest: number;
}
export function groupCalls(
  calls: CallRecord[],
  conversations: Map<string, Conversation>,
  by: "project" | "conversation",
): Group[] {
  const groups = new Map<string, Group>();
  for (const call of calls) {
    const conversation = conversations.get(call.conversationId);
    const id =
      by === "project" ? (conversation?.projectId ?? "__unassigned__") : call.conversationId;
    let group = groups.get(id);
    if (!group) {
      group = {
        id,
        label:
          by === "project"
            ? (conversation?.projectName ?? "未分配项目")
            : (conversation?.title ?? "未命名会话"),
        totals: emptyTotals(),
        latest: call.timestampMs,
      };
      groups.set(id, group);
    }
    add(group.totals, call);
    group.latest = Math.max(group.latest, call.timestampMs);
  }
  return [...groups.values()].sort((a, b) =>
    a.totals.total === b.totals.total
      ? a.label.localeCompare(b.label)
      : a.totals.total > b.totals.total
        ? -1
        : 1,
  );
}
