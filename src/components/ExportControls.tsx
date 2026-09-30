import { useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { buildExport } from "../export";
import type { Filters } from "../data";
import type { CallRecord, Conversation, MonitorStatus } from "../types";

export function ExportControls({
  calls,
  conversations,
  status,
  filters,
  disabled,
}: {
  calls: CallRecord[];
  conversations: Map<string, Conversation>;
  status: MonitorStatus | null;
  filters: Filters;
  disabled: boolean;
}) {
  const [identifying, setIdentifying] = useState(false),
    [busy, setBusy] = useState(false),
    [message, setMessage] = useState("");
  const save = async (format: "csv" | "json") => {
    if (!status || disabled || busy) return;
    setBusy(true);
    setMessage("");
    try {
      const content = buildExport(calls, conversations, status, filters, identifying, format);
      if (new TextEncoder().encode(content).byteLength > 64 * 1024 * 1024)
        throw new Error("导出超过 64 MiB，请缩小筛选范围。");
      const saved = await invoke<boolean>("save_export", { format, content });
      setMessage(saved ? `已导出 ${calls.length.toLocaleString()} 条筛选记录` : "已取消导出");
    } catch (reason) {
      setMessage(reason instanceof Error ? reason.message : String(reason));
    } finally {
      setBusy(false);
    }
  };
  return (
    <details
      className="export-menu"
      onToggle={(event) => {
        if (!event.currentTarget.open) {
          setIdentifying(false);
          setMessage("");
        }
      }}
    >
      <summary className="ghost-button">导出筛选结果</summary>
      <div className="export-popover">
        <p>
          导出当前筛选的全部 {calls.length.toLocaleString()}{" "}
          条已加载记录，不只是屏幕可见行；不是完整历史账单。
        </p>
        <label className="export-identifying">
          <input
            type="checkbox"
            checked={identifying}
            onChange={(e) => setIdentifying(e.target.checked)}
            disabled={busy}
          />
          包含真实标题、ID 和项目路径
        </label>
        <p>
          默认替换项目／会话名称并移除真实 ID
          和路径。时间、日志模型和用量仍可能敏感，请检查后再分享。
        </p>
        <div className="export-actions">
          <button
            type="button"
            className="ghost-button"
            disabled={disabled || busy || !calls.length}
            onClick={() => void save("csv")}
          >
            导出 CSV
          </button>
          <button
            type="button"
            className="ghost-button"
            disabled={disabled || busy || !calls.length}
            onClick={() => void save("json")}
          >
            导出 JSON
          </button>
        </div>
        <p aria-live="polite">{busy ? "正在准备导出…" : message}</p>
      </div>
    </details>
  );
}
