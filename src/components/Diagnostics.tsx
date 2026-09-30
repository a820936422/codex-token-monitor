import type { MonitorStatus } from "../types";

export function monitorHealth(
  status: MonitorStatus | null,
  syncing: boolean,
  stale: boolean,
  error: string | null,
) {
  if (error) return "Error";
  if (syncing || !status) return "Starting";
  if (stale) return "Stale";
  if (status.discovering || status.pendingFiles) return "Indexing";
  if (
    status.unreadableFiles ||
    status.missingRoots ||
    status.discoveryErrors ||
    status.parseErrors ||
    status.invalidRecords ||
    status.oversizedLines ||
    status.fileLimitReached ||
    status.metadataLimitReached
  )
    return "Partial";
  if (!status.records && status.legacyRecords) return "Unsupported";
  return "Live";
}
export function Diagnostics({ status }: { status: MonitorStatus | null }) {
  if (!status) return null;
  const timestamp = (value: string | null) => (value ? new Date(value).toLocaleString() : "尚无");
  return (
    <details className="diagnostics">
      <summary>
        运行诊断 · {status.files} 个日志文件 · 本轮 {status.scanMs} ms
      </summary>
      <dl>
        <dt>扫描心跳</dt>
        <dd>{timestamp(status.lastScanAt)}</dd>
        <dt>最近成功读取／检查</dt>
        <dd>{timestamp(status.lastSuccessAt)}</dd>
        <dt>索引进度</dt>
        <dd>
          {status.indexedFiles} / {status.files} 文件；待处理 {status.pendingFiles} 文件 /{" "}
          {(status.pendingBytes / 1048576).toFixed(2)} MiB
        </dd>
        <dt>本轮读取</dt>
        <dd>
          {(status.readBytes / 1024).toFixed(1)} KiB；半行等待 {status.partialLines} 个文件
        </dd>
        <dt>文件变化通知</dt>
        <dd>
          {status.watcherEnabled ? "已启用，轮询兜底" : "轮询模式"}；注册错误 {status.watcherErrors}
          ；队列溢出 {status.watcherOverflows}
        </dd>
        <dt>文件可读性</dt>
        <dd>
          不可读 {status.unreadableFiles}；缺失根目录 {status.missingRoots}；目录扫描错误{" "}
          {status.discoveryErrors}
        </dd>
        <dt>解析计数（本次运行）</dt>
        <dd>
          坏 JSON {status.parseErrors}；无效记录 {status.invalidRecords}；超大行{" "}
          {status.oversizedLines}；旧累计格式 {status.legacyRecords}
        </dd>
        <dt>当前数据质量</dt>
        <dd>{status.recordsWithIssues} 条记录存在缺失字段或异常；未知值不计作零</dd>
        <dt>索引上限</dt>
        <dd>
          文件目录 {status.fileLimitReached ? "已触及上限" : "未触及"}；标题{" "}
          {status.metadataLimitReached ? "已触及上限" : "未触及"}
        </dd>
        <dt>数据目录（仅本地展示）</dt>
        <dd>{status.sessionRoots.join(" · ")}</dd>
      </dl>
      <p>
        旧累计 token_count
        不会伪装成逐调用数据。历史回读会重新计入解析诊断次数；这些计数不是独立故障数。扫描时间预算为软限制，大量文件或磁盘繁忙时可能超过。
      </p>
    </details>
  );
}
