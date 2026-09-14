export interface Usage {
  inputTokens: number;
  cachedInputTokens: number;
  cacheWriteInputTokens: number;
  outputTokens: number;
  reasoningOutputTokens: number;
  totalTokens: number;
}

export interface CallRecord {
  id: string;
  timestamp: string;
  conversationId: string;
  threadId: string;
  turnId: string | null;
  responseId: string | null;
  model: string;
  effort: string | null;
  serviceTier: string;
  usage: Usage;
  freshInputTokens: number;
  cacheHitRate: number;
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
  records: number;
  conversations: number;
  projects: number;
  files: number;
  parseErrors: number;
  pollMs: number;
  sessionRoots: string[];
  sessionIndex: string;
  lastScanAt: string | null;
}

export interface Snapshot {
  calls: CallRecord[];
  catalog: Catalog;
  status: MonitorStatus;
}
