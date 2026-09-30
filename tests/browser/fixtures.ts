import type { CallRecord, Snapshot, MonitorStatus, Update } from "../../src/types";

export function call(
  id: string,
  conversationId = "c1",
  input = 1000,
  cached = 600,
  output = 100,
  day = "22",
): CallRecord {
  return {
    id,
    timestamp: `2026-09-${day}T10:00:00Z`,
    timestampMs: Date.parse(`2026-09-${day}T10:00:00Z`),
    issues: [],
    conversationId,
    threadId: conversationId,
    turnId: "turn",
    responseId: id,
    model: `logged-${id}`,
    effort: "high",
    serviceTier: "default",
    usage: {
      inputTokens: input,
      cachedInputTokens: cached,
      cacheWriteInputTokens: 0,
      outputTokens: output,
      reasoningOutputTokens: 0,
      totalTokens: input + output,
    },
    freshInputTokens: input - cached,
    cacheHitRate: input ? (cached / input) * 100 : 0,
  };
}

export const snapshot: Snapshot = {
  revision: 0,
  calls: [
    call("A"),
    call("B", "c2", 300, 100, 50, "23"),
    {
      ...call("C", "c1", 200, 100, 20, "23"),
      timestamp: "2026-09-23T12:00:00Z",
      timestampMs: Date.parse("2026-09-23T12:00:00Z"),
    },
  ],
  catalog: {
    projects: [
      { id: "p1", name: "Alpha project", path: "/synthetic/alpha", conversations: 1 },
      { id: "p2", name: "Beta project", path: "/synthetic/beta", conversations: 1 },
    ],
    conversations: [
      {
        id: "c1",
        title: "Alpha conversation",
        cwd: "/synthetic/alpha",
        projectId: "p1",
        projectName: "Alpha project",
        projectPath: "/synthetic/alpha",
      },
      {
        id: "c2",
        title: "Beta conversation",
        cwd: "/synthetic/beta",
        projectId: "p2",
        projectName: "Beta project",
        projectPath: "/synthetic/beta",
      },
    ],
  },
  status: {
    revision: 0,
    records: 3,
    retentionLimit: 50000,
    truncated: false,
    oldestAt: "2026-09-22T10:00:00Z",
    newestAt: "2026-09-23T12:00:00Z",
    conversations: 2,
    projects: 2,
    files: 2,
    indexedFiles: 2,
    pendingFiles: 0,
    pendingBytes: 0,
    partialLines: 0,
    discovering: false,
    initialLoadComplete: true,
    fileLimitReached: false,
    metadataLimitReached: false,
    unreadableFiles: 0,
    missingRoots: 0,
    discoveryErrors: 0,
    parseErrors: 0,
    invalidRecords: 0,
    legacyRecords: 0,
    oversizedLines: 0,
    recordsWithIssues: 0,
    watcherEnabled: true,
    watcherErrors: 0,
    watcherOverflows: 0,
    scanMs: 1,
    readBytes: 0,
    pollMs: 750,
    sessionRoots: ["/synthetic/sessions"],
    sessionIndex: "/synthetic/session_index.jsonl",
    lastScanAt: "2026-09-23T12:00:00Z",
    lastSuccessAt: "2026-09-23T12:00:00Z",
  },
};

export interface HarnessOptions {
  snapshot: Snapshot;
  saved?: Record<string, string>;
  delaySnapshotMs?: number;
  failSnapshot?: boolean;
  failListen?: string;
  injectBeforeSnapshot?: CallRecord;
  injectUpdate?: Update;
  resyncSnapshot?: Snapshot;
  failExport?: boolean;
}

export function makeSnapshot(
  calls: CallRecord[],
  revision = 0,
  status: Partial<MonitorStatus> = {},
): Snapshot {
  const sorted = [...calls].sort((a, b) => b.timestampMs - a.timestampMs);
  const ids = new Set(calls.map((c) => c.conversationId));
  const conversations = snapshot.catalog.conversations.filter((c) => ids.has(c.id));
  const projectIds = new Set(conversations.map((c) => c.projectId));
  const projects = snapshot.catalog.projects.filter((p) => projectIds.has(p.id));
  return {
    revision,
    calls,
    catalog: { conversations, projects },
    status: {
      ...snapshot.status,
      records: calls.length,
      conversations: conversations.length,
      projects: projects.length,
      oldestAt: sorted.at(-1)?.timestamp ?? null,
      newestAt: sorted[0]?.timestamp ?? null,
      ...status,
      revision,
    },
  };
}
export function batch(
  calls: CallRecord[],
  revision: number,
  removedIds: string[] = [],
  records = snapshot.calls.length + calls.length - removedIds.length,
): Update {
  return {
    revision,
    calls,
    removedIds,
    catalog: null,
    status: { ...snapshot.status, records, revision },
  };
}
