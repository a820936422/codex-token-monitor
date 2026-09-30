// IPC contract: src-tauri/src/monitor/model.rs. Unknown counters are null.

export interface Usage {
  inputTokens: number | null;
  cachedInputTokens: number | null;
  cacheWriteInputTokens: number | null;
  outputTokens: number | null;
  reasoningOutputTokens: number | null;
  totalTokens: number | null;
}

export interface CallRecord {
  id: string;
  timestamp: string;
  timestampMs: number;
  conversationId: string;
  threadId: string;
  turnId: string | null;
  responseId: string | null;
  model: string;
  effort: string | null;
  serviceTier: string;
  usage: Usage;
  freshInputTokens: number | null;
  cacheHitRate: number | null;
  issues: string[];
}

export interface Conversation {
  id: string;
  title: string;
  cwd: string | null;
  projectId: string;
  projectName: string;
  projectPath: string | null;
}

export interface Project {
  id: string;
  name: string;
  path: string | null;
  conversations: number;
}

export interface Catalog {
  projects: Project[];
  conversations: Conversation[];
}

export interface MonitorStatus {
  revision: number;
  records: number;
  retentionLimit: number;
  truncated: boolean;
  oldestAt: string | null;
  newestAt: string | null;
  conversations: number;
  projects: number;
  files: number;
  indexedFiles: number;
  pendingFiles: number;
  pendingBytes: number;
  partialLines: number;
  discovering: boolean;
  initialLoadComplete: boolean;
  fileLimitReached: boolean;
  metadataLimitReached: boolean;
  unreadableFiles: number;
  missingRoots: number;
  discoveryErrors: number;
  parseErrors: number;
  invalidRecords: number;
  legacyRecords: number;
  oversizedLines: number;
  recordsWithIssues: number;
  watcherEnabled: boolean;
  watcherErrors: number;
  watcherOverflows: number;
  scanMs: number;
  readBytes: number;
  pollMs: number;
  sessionRoots: string[];
  sessionIndex: string;
  lastScanAt: string | null;
  lastSuccessAt: string | null;
}

export interface Snapshot {
  revision: number;
  calls: CallRecord[];
  catalog: Catalog;
  status: MonitorStatus;
}

export interface Update {
  revision: number;
  calls: CallRecord[];
  removedIds: string[];
  catalog: Catalog | null;
  status: MonitorStatus;
}
