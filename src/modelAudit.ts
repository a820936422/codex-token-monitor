import type { CallRecord, ModelAuditStatus } from "./types";

export type AuditFilter = "all" | ModelAuditStatus | "unobserved";
export const auditLabels: Record<Exclude<AuditFilter, "all">, string> = {
  match: "声明一致", different: "声明不同", conflict: "声明冲突", unknown: "未知", unobserved: "未采集",
};

export function auditStatus(call: CallRecord): Exclude<AuditFilter, "all"> {
  const audit = call.modelAudit;
  if (!audit) return "unobserved";
  switch (audit.status) {
    case "match": case "different": case "conflict": case "unknown": return audit.status;
    default: return "unknown";
  }
}

export const sourceLabels: Record<string, string> = {
  response_body: "响应正文", http_header: "HTTP 响应头", event_header: "事件头",
};

export const issueLabels: Record<string, string> = {
  invalid_json: "响应 JSON 无法解析", event_too_large: "响应事件超过采集大小限制",
  empty_response: "响应为空", too_many_models: "模型声明数量超过采集限制",
  response_id_conflict: "响应 ID 冲突", incomplete_response: "响应未完成",
  stream_interrupted: "响应流中断", unsupported_encoding: "不支持的响应编码",
  response_id_reused: "同一响应 ID 存在不同采集记录", inconsistent_metadata: "返回模型缺少对应来源证据",
  invalid_identifier: "模型或响应 ID 格式无效",
  upstream_http_error: "上游返回 HTTP 错误", upstream_redirect: "上游重定向已拒绝",
  upstream_connection_failed: "上游连接失败",
};
