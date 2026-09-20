import { useEffect, useId, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import type { CollectorStatus } from "../collectorTypes";

type CollectorCommand = "set_collection_enabled" | "set_collector_auto_start" | "restore_collector_route" | "stop_collector_forwarder";

function errorText(error: unknown) {
  return error instanceof Error ? error.message : String(error);
}

function stateLabel(status: CollectorStatus | null) {
  if (!status) return "等待采集状态";
  if (status.phase === "error") return "采集异常";
  if (status.phase === "starting") return "正在启动采集";
  if (!status.enabled) return status.forwarding ? "已暂停采集 · 仍在转发" : "采集已停止";
  if (!status.forwarding || status.phase === "stopped") return "等待采集服务就绪";
  return status.observations === 0 ? "采集已开启 · 等待响应" : "正在采集";
}

export function CollectorControl() {
  const [status, setStatus] = useState<CollectorStatus | null>(null);
  const [loading, setLoading] = useState(true);
  const [busy, setBusy] = useState<string | null>(null);
  const [error, setError] = useState<{ text: string; reconnect: boolean } | null>(null);
  const [attempt, setAttempt] = useState(0);
  const revision = useRef(0);
  const lifecycle = useRef(0);
  const busyRef = useRef(false);
  const headingId = useId();
  const noteId = useId();
  const stopNoteId = useId();

  useEffect(() => {
    const generation = ++lifecycle.current;
    const initialRevision = revision.current;
    let active = true;
    let off: (() => void) | undefined;
    setLoading(true);
    setError(null);
    void (async () => {
      const unlisten = await listen<CollectorStatus>("collector-status", ({ payload }) => {
        if (!active) return;
        revision.current += 1;
        setStatus(payload);
      });
      if (!active) { unlisten(); return; }
      off = unlisten;
      const snapshot = await invoke<CollectorStatus>("get_collector_status");
      // Events received during registration or the request are newer than the snapshot.
      if (active && revision.current === initialRevision) setStatus(snapshot);
    })().catch((cause) => {
      if (active) setError({ text: `无法连接采集状态：${errorText(cause)}`, reconnect: true });
    }).finally(() => {
      if (active) setLoading(false);
    });
    return () => {
      active = false;
      if (lifecycle.current === generation) lifecycle.current += 1;
      off?.();
    };
  }, [attempt]);

  const run = async (command: CollectorCommand, label: string, args?: { enabled: boolean }) => {
    if (busyRef.current || loading || !status) return;
    busyRef.current = true;
    setBusy(label);
    setError(null);
    const generation = lifecycle.current;
    const before = revision.current;
    try {
      const result = await invoke<CollectorStatus>(command, args);
      // A status event emitted while a command runs takes precedence over its return value.
      if (lifecycle.current === generation && revision.current === before) setStatus(result);
    } catch (cause) {
      if (lifecycle.current === generation) setError({ text: `${label}失败：${errorText(cause)}`, reconnect: false });
    } finally {
      busyRef.current = false;
      if (lifecycle.current === generation) setBusy(null);
    }
  };

  const disabled = loading || !!busy || !status;
  const rss = status?.rssBytes;
  const cpu = status?.cpuPercent;
  const hasError = !!error || status?.phase === "error";

  return <section className="collector-panel" aria-labelledby={headingId} aria-busy={loading || !!busy}>
    <div className="collector-main">
      <div className="collector-heading">
        <h2 id={headingId}>响应模型采集</h2>
        <span className={`collector-state${hasError ? " collector-warning" : status?.enabled ? " collector-on" : ""}`} role="status" aria-atomic="true">
          {busy ? `${busy}…` : loading ? "正在连接采集服务…" : error?.reconnect ? "采集状态连接失败" : stateLabel(status)}
        </span>
      </div>
      <div className="collector-controls">
        <button type="button" className="collector-switch" role="switch" aria-label="响应模型采集" aria-checked={status?.enabled ?? false} aria-describedby={noteId} disabled={disabled} onClick={() => void run("set_collection_enabled", status?.enabled ? "暂停采集" : "开启采集", { enabled: !status?.enabled })}>
          <span className="collector-switch-track" aria-hidden="true"><span /></span>
          <span>{status?.enabled ? "采集已开启" : "开启采集"}</span>
        </button>
        <label className="collector-auto"><input type="checkbox" checked={status?.autoStart ?? true} disabled={disabled} onChange={(event) => void run("set_collector_auto_start", "保存自动采集设置", { enabled: event.target.checked })} />随程序启动自动采集</label>
      </div>
      <dl className="collector-metrics" aria-label="采集服务实时资源占用">
        <div><dt>内存 RSS</dt><dd>{rss != null && Number.isFinite(rss) ? `${(rss / 1024 / 1024).toFixed(1)} MiB` : "—"}</dd></div>
        <div><dt>CPU</dt><dd>{cpu != null && Number.isFinite(cpu) ? `${cpu.toFixed(1)}%` : "—"}</dd></div>
        <div><dt>已采集响应</dt><dd>{status?.observations.toLocaleString() ?? "—"}</dd></div>
      </dl>
    </div>
    <p className="collector-note" id={noteId}>暂停会停止模型观测和写入，转发继续运行，保障已打开的 Codex 会话连接。</p>
    {status?.routeInstalled && <p className="collector-note">采集路由已写入所选用户配置。首次接入或重新安装路由后，请重启 Codex，让后续请求经过采集器。</p>}
    {!status?.routeInstalled && status?.enabled && <p className="collector-note">当前配置为直连；已打开且使用采集地址的 Codex 仍可能经过转发，新启动的 Codex 不会经过此采集路由。</p>}
    {error && <div className="collector-error"><p role="alert">{error.text}</p>{error.reconnect && <button type="button" className="ghost-button" disabled={loading || !!busy} onClick={() => setAttempt((value) => value + 1)}>重新连接</button>}</div>}
    {status?.message && <p className={status.phase === "error" ? "collector-warning collector-note" : "collector-note"} role={status.phase === "error" ? "alert" : undefined}>{status.message}</p>}
    {!!status?.writeErrors && <p className="collector-warning collector-note" role="status">采集记录写入失败 {status.writeErrors.toLocaleString()} 次，模型核对记录可能不完整。</p>}
    <details className="collector-details">
      <summary>连接与退出设置</summary>
      <p>开启采集会将所选用户配置的 base URL 指向本地转发服务，正常请求仍发往原上游。首次接入需重启 Codex；采集随正常请求进行，不会额外调用模型。</p>
      <p>监控器正常退出时会恢复配置并关闭采集。为保障已打开的 Codex 会话连接，一个小型进程会继续仅转发请求。重启 Codex 后，可在此停止转发。</p>
      <p>资源读数仅统计采集助手；CPU 100% 表示占满一个逻辑核心。暂停采集后，转发服务仍占用基础内存。</p>
      {status && <p>已转发请求：{status.requests.toLocaleString()} · 进行中：{status.activeRequests.toLocaleString()}</p>}
      {status?.upstreamOrigin && <p>上游：<code>{status.upstreamOrigin}</code></p>}
      {status?.configPath && <p>所选配置：<code>{status.configPath}</code></p>}
      <p>连接配置：{status ? status.routeInstalled ? "已接入本地采集" : "直连配置" : "等待状态"} · 转发服务：{status ? status.forwarding ? "运行中" : "已停止" : "等待状态"}</p>
      <button type="button" className="ghost-button" disabled={disabled || !status?.routeInstalled} onClick={() => void run("restore_collector_route", "恢复直连配置")}>恢复直连配置</button>
      {status?.forwarding && !status.routeInstalled && <div className="collector-stop">
        <p id={stopNoteId}>直连配置已恢复。请先重启使用采集地址的所有 Codex 客户端，再停止转发，否则旧客户端将无法连接。{status.activeRequests > 0 ? `当前仍有 ${status.activeRequests} 个进行中的请求，暂时无法停止。` : "进行中的请求结束后才能停止转发。"}</p>
        <button type="button" className="ghost-button" aria-describedby={stopNoteId} disabled={disabled || status.activeRequests > 0} onClick={() => void run("stop_collector_forwarder", "停止转发")}>已重启 Codex，停止转发</button>
      </div>}
    </details>
  </section>;
}
