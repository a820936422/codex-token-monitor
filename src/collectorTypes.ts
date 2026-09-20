export interface CollectorStatus {
  phase: string;
  enabled: boolean;
  autoStart: boolean;
  forwarding: boolean;
  routeInstalled: boolean;
  localUrl: string | null;
  upstreamOrigin: string | null;
  configPath: string;
  requests: number;
  observations: number;
  activeRequests: number;
  writeErrors: number;
  rssBytes: number | null;
  cpuPercent: number | null;
  message: string | null;
}
