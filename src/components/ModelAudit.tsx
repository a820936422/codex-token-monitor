import { useEffect, useId, useRef, useState, type ReactNode } from "react";
import { writeText } from "@tauri-apps/plugin-clipboard-manager";
import { auditLabels, auditStatus, issueLabels, sourceLabels } from "../modelAudit";
import type { CallRecord, ModelAuditMonitor } from "../types";

const evidenceNote = "模型标识是提供方的声明，不是后端模型身份认证。响应正文中的 model 可作为声明一致性的证据，无需响应头。日期或版本后缀不同不自动视为一致。";

export function ModelAuditCell({ call, onOpen }: { call: CallRecord; onOpen: (id: string) => void }) {
  const status = auditStatus(call);
  const model = call.modelAudit?.reportedModel || (status === "unobserved" ? "未采集响应模型" : "未取得返回模型");
  return <button type="button" className="audit-cell-button" onClick={() => onOpen(call.id)} aria-label={`查看模型核对详情：${call.model || "未知请求模型"}，${auditLabels[status]}，${call.responseId || call.id}`} aria-haspopup="dialog">
    <strong title={model}>{model}</strong>
    <span className={`audit-badge audit-${status}`}>{auditLabels[status]}</span><span className="audit-detail-hint">详情</span>
  </button>;
}

function AuditDialog({ title, onClose, children }: { title: string; onClose: () => void; children: ReactNode }) {
  const dialogRef = useRef<HTMLDialogElement>(null);
  const titleId = useId();
  const closeRef = useRef(onClose);
  closeRef.current = onClose;
  useEffect(() => {
    const dialog = dialogRef.current;
    const previousFocus = document.activeElement instanceof HTMLElement ? document.activeElement : null;
    dialog?.showModal();
    return () => {
      dialog?.close();
      if (previousFocus?.isConnected) previousFocus.focus({ preventScroll: true });
      else document.getElementById("model-audit-help")?.focus({ preventScroll: true });
    };
  }, []);
  return <dialog className="audit-dialog" ref={dialogRef} aria-labelledby={titleId} onCancel={(event) => { event.preventDefault(); closeRef.current(); }} onClick={(event) => {
    if (event.target !== event.currentTarget) return;
    const rect = event.currentTarget.getBoundingClientRect();
    if (event.clientX < rect.left || event.clientX > rect.right || event.clientY < rect.top || event.clientY > rect.bottom) closeRef.current();
  }}>
    <div className="audit-dialog-heading"><h2 id={titleId}>{title}</h2><button type="button" className="ghost-button" autoFocus onClick={onClose} aria-label={`关闭${title}`}>关闭</button></div>
    {children}
  </dialog>;
}

function CopyValue({ value, label }: { value: string; label: string }) {
  const [feedback, setFeedback] = useState("");
  const copy = async () => {
    try { await writeText(value); setFeedback("已复制"); }
    catch { setFeedback("复制失败，可选中文字手动复制"); }
  };
  return <div className="audit-copy"><code>{value}</code><button type="button" className="ghost-button" onClick={() => void copy()} aria-label={`复制${label}`}>复制</button><span role="status">{feedback}</span></div>;
}

export function ModelAuditDetails({ call, onClose }: { call: CallRecord; onClose: () => void }) {
  const audit = call.modelAudit;
  const status = auditStatus(call);
  const values = audit?.observedModels ?? [];
  const sources = audit?.sources ?? [];
  const issues = audit?.captureIssues ?? [];
  return <AuditDialog title="模型核对详情" onClose={onClose}>
    <p><span className={`audit-badge audit-${status}`}>{auditLabels[status]}</span></p>
    <p className="audit-note">{evidenceNote}</p>
    <p className="audit-note">主状态核对本地选择的请求模型与返回声明；实际发送的模型单独列出，便于检查发送前的映射。未知表示已关联采集记录但证据不足；未采集表示尚无关联记录。</p>
    <dl className="audit-facts">
      <dt>本地选择的请求模型</dt><dd><code>{call.model || "未知"}</code></dd>
      <dt>实际发送模型 · sentModel</dt><dd><code>{audit?.sentModel || "未取得"}</code></dd>
      <dt>返回模型 · reportedModel</dt><dd><code>{audit?.reportedModel || "未取得"}</code></dd>
      <dt>全部观测模型</dt><dd>{values.length ? <ul>{values.map((value, index) => <li key={index}><code>{value}</code></li>)}</ul> : "无"}</dd>
      <dt>声明来源</dt><dd>{sources.length ? <ul>{sources.map((source, index) => <li key={index}>{sourceLabels[source] || "其他来源"} · <code>{source}</code></li>)}</ul> : "未取得"}</dd>
      <dt>上游地址 · origin</dt><dd><code>{audit?.upstream || "未取得"}</code></dd>
      <dt>响应完成 · completed</dt><dd>{audit?.completed === true ? "是" : audit?.completed === false ? "否" : "未知"}</dd>
      <dt>精确 response_id</dt><dd>{call.responseId ? <CopyValue value={call.responseId} label="response_id" /> : "未取得"}</dd>
      <dt>观测时间</dt><dd>{audit?.observedAt ? <time dateTime={audit.observedAt}>{audit.observedAt}</time> : "未取得"}</dd>
      <dt>采集问题</dt><dd>{issues.length ? <ul>{issues.map((issue, index) => <li key={index}>{issueLabels[issue] || "采集异常"} · <code>{issue}</code></li>)}</ul> : audit ? "未报告问题" : "未采集"}</dd>
    </dl>
  </AuditDialog>;
}

export function ModelAuditHelp({ metadata, onClose }: { metadata?: ModelAuditMonitor | null; onClose: () => void }) {
  return <AuditDialog title="模型采集说明" onClose={onClose}>
    <p>现有 Codex JSONL 不包含响应模型字段，历史调用仍显示「未采集」。只有显式经过采集助手的流量才会记录响应模型，并通过精确 response_id 关联调用。</p>
    <p className="audit-note">{evidenceNote}</p>
    <dl className="audit-facts">
      <dt>采集元数据目录</dt><dd><code>{metadata?.directory || "监控端尚未提供"}</code></dd>
      <dt>已缓存响应 ID / 已关联调用 / 解析错误</dt><dd>{metadata?.observations ?? "—"} / {metadata?.matchedCalls ?? "—"} / {metadata?.parseErrors ?? "—"}</dd>
      <dt>最后观测时间</dt><dd>{metadata?.lastObservedAt ? <time dateTime={metadata.lastObservedAt}>{metadata.lastObservedAt}</time> : "暂无观测"}</dd>
    </dl>
    <p className="audit-note">以上是已读取的采集元数据，不表示助手当前正在运行；顶部 Live 表示本地调用监控已连接。</p>
    <p>在仓库根目录运行以下命令，随后在助手启动的 Codex 进程中发起正常请求：</p>
    <h3>当前自定义提供方</h3>
    <CopyValue value="python3 scripts/model_audit.py run --mode configured" label="自定义提供方采集命令" />
    <h3>官方订阅</h3>
    <CopyValue value="python3 scripts/model_audit.py run --mode official" label="官方订阅采集命令" />
    <p>官方模式需要先完成 Codex 登录；命令仅为本次启动的进程覆盖提供方配置。</p>
    <p>桌面端持久配置见仓库内 <code>docs/model-audit.md</code>。已运行的桌面应用不会自动接入助手，需要按文档配置并重新启动。</p>
    <p className="audit-note">采集随普通请求进行，不会额外调用模型。这里的复制按钮只写入剪贴板，不执行命令。</p>
  </AuditDialog>;
}
