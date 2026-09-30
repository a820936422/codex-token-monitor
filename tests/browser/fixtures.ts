import type { CallRecord, Snapshot } from "../../src/types";

export function call(id: string, conversationId = "c1", input = 1000, cached = 600, output = 100, day = "22"): CallRecord {
  return { id, timestamp: `2026-09-${day}T10:00:00Z`, conversationId, threadId: conversationId,
    turnId: "turn", responseId: id, model: `logged-${id}`, effort: "high", serviceTier: "default",
    usage: { inputTokens: input, cachedInputTokens: cached, cacheWriteInputTokens: 0, outputTokens: output, reasoningOutputTokens: 0, totalTokens: input + output },
    freshInputTokens: input - cached, cacheHitRate: input ? cached / input * 100 : 0 };
}

export const snapshot: Snapshot = {
  calls: [call("A"), call("B", "c2", 300, 100, 50, "23"), { ...call("C", "c1", 200, 100, 20, "23"), timestamp: "2026-09-23T12:00:00Z" }],
  catalog: {
    projects: [
      { id: "p1", name: "Alpha project", path: "/synthetic/alpha", conversations: 1 },
      { id: "p2", name: "Beta project", path: "/synthetic/beta", conversations: 1 },
    ],
    conversations: [
      { id: "c1", title: "Alpha conversation", cwd: "/synthetic/alpha", projectId: "p1", projectName: "Alpha project", projectPath: "/synthetic/alpha" },
      { id: "c2", title: "Beta conversation", cwd: "/synthetic/beta", projectId: "p2", projectName: "Beta project", projectPath: "/synthetic/beta" },
    ],
  },
  status: { records: 3, conversations: 2, projects: 2, files: 2, parseErrors: 0, pollMs: 750, sessionRoots: ["/synthetic/sessions"], sessionIndex: "/synthetic/session_index.jsonl", lastScanAt: "2026-09-23T12:00:00Z" },
};

export interface HarnessOptions {
  snapshot: Snapshot;
  saved?: Record<string, string>;
  delaySnapshotMs?: number;
  failSnapshot?: boolean;
  failListen?: string;
  injectBeforeSnapshot?: CallRecord;
}
