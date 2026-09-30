use super::model::{CallRecord, Diagnostics, Usage};
use chrono::{DateTime, Datelike, SecondsFormat, Utc};
use serde_json::Value;
use std::collections::{HashMap, VecDeque};

const MAX_TURNS: usize = 64;
const MAX_SAFE_INTEGER: u64 = 9_007_199_254_740_991;
pub(super) fn text(value: &Value, key: &str, max: usize) -> Option<String> {
    value
        .get(key)?
        .as_str()
        .filter(|s| !s.is_empty() && s.len() <= max && !s.chars().any(char::is_control))
        .map(str::to_owned)
}
pub(super) fn timestamp(value: &str) -> Option<(String, i64)> {
    let parsed = DateTime::parse_from_rfc3339(value)
        .ok()?
        .with_timezone(&Utc);
    if !(0..=9999).contains(&parsed.year()) {
        return None;
    }
    Some((
        parsed.to_rfc3339_opts(SecondsFormat::Millis, true),
        parsed.timestamp_millis(),
    ))
}
#[derive(Debug, Clone)]
struct Context {
    model: String,
    effort: Option<String>,
    tier: String,
}
impl Default for Context {
    fn default() -> Self {
        Self {
            model: "unknown".into(),
            effort: None,
            tier: "default".into(),
        }
    }
}
#[derive(Debug, Default)]
pub(super) struct Parser {
    pub thread_id: Option<String>,
    pub session_id: Option<String>,
    pub cwd: Option<String>,
    turn_id: Option<String>,
    current: Context,
    turns: HashMap<String, Context>,
    turn_order: VecDeque<String>,
}
impl Parser {
    pub fn consume(
        &mut self,
        row: &Value,
        line_offset: u64,
        source_key: &str,
        diagnostics: &mut Diagnostics,
    ) -> Option<CallRecord> {
        let kind = row.get("type").and_then(Value::as_str)?;
        let payload = row.get("payload").unwrap_or(&Value::Null);
        match kind {
            "session_meta" => {
                self.thread_id = text(payload, "id", 256).or(self.thread_id.take());
                self.session_id = text(payload, "session_id", 256).or(self.thread_id.clone());
                self.cwd = text(payload, "cwd", 4096);
                return None;
            }
            "turn_context" => {
                self.turn_id = text(payload, "turn_id", 256).or(self.turn_id.take());
                self.current.model =
                    text(payload, "model", 256).unwrap_or_else(|| "unknown".into());
                self.current.effort = text(payload, "effort", 64);
                self.current.tier =
                    text(payload, "service_tier", 64).unwrap_or_else(|| "default".into());
                if let Some(turn) = &self.turn_id {
                    if !self.turns.contains_key(turn) {
                        self.turn_order.push_back(turn.clone());
                    }
                    self.turns.insert(turn.clone(), self.current.clone());
                    while self.turn_order.len() > MAX_TURNS {
                        if let Some(old) = self.turn_order.pop_front() {
                            self.turns.remove(&old);
                        }
                    }
                }
                return None;
            }
            "event_msg" => {
                match payload.get("type").and_then(Value::as_str) {
                    Some("token_count") => diagnostics.legacy_records += 1,
                    Some("task_started") => self.turn_id = text(payload, "turn_id", 256),
                    Some("task_complete") => {
                        let id = text(payload, "turn_id", 256);
                        if id.is_none() || id == self.turn_id {
                            self.turn_id = None;
                        }
                    }
                    Some("thread_settings_applied") => {
                        if let Some(settings) = payload.get("thread_settings") {
                            if let Some(tier) = text(settings, "service_tier", 64) {
                                self.current.tier = tier;
                            }
                        }
                    }
                    _ => {}
                }
                return None;
            }
            "token_usage_record" => {}
            _ => return None,
        }
        let Some((timestamp, timestamp_ms)) = row
            .get("timestamp")
            .and_then(Value::as_str)
            .and_then(timestamp)
        else {
            diagnostics.invalid_records += 1;
            return None;
        };
        let Some(raw) = payload.get("usage").filter(|v| v.is_object()) else {
            diagnostics.invalid_records += 1;
            return None;
        };
        let (usage, mut issues) = normalize_usage(raw);
        if usage.input_tokens.is_none()
            && usage.output_tokens.is_none()
            && usage.total_tokens.is_none()
        {
            diagnostics.invalid_records += 1;
            return None;
        }
        let turn_id = text(payload, "turn_id", 256).or(self.turn_id.clone());
        let unknown = Context::default();
        let context = match &turn_id {
            Some(id) => self.turns.get(id).unwrap_or(&unknown),
            None => &self.current,
        };
        let thread_id = text(payload, "thread_id", 256)
            .or(self.thread_id.clone())
            .or_else(|| text(payload, "session_id", 256))
            .unwrap_or_else(|| source_key.to_owned());
        let conversation_id = text(payload, "session_id", 256)
            .or(self.session_id.clone())
            .unwrap_or_else(|| thread_id.clone());
        if thread_id == source_key {
            issues.push("missing_session_id".into());
        }
        let response_id = text(payload, "response_id", 256);
        let id = response_id.clone().unwrap_or_else(|| {
            issues.push("missing_response_id".into());
            // Offsets distinguish two same-time, same-usage records. Session copies
            // retain their metadata/offsets, so archive moves do not change these IDs.
            format!("log:{thread_id}:{line_offset}:{timestamp_ms}")
        });
        let fresh_input_tokens = usage
            .input_tokens
            .zip(usage.cached_input_tokens)
            .zip(usage.cache_write_input_tokens)
            .and_then(|((input, cached), written)| input.checked_sub(cached)?.checked_sub(written));
        let cache_hit_rate =
            usage
                .input_tokens
                .zip(usage.cached_input_tokens)
                .and_then(|(input, cached)| {
                    (input > 0).then_some(cached as f64 / input as f64 * 100.0)
                });
        let model = text(payload, "model", 256).unwrap_or_else(|| context.model.clone());
        if model == "unknown" {
            issues.push("missing_model_context".into());
        }
        Some(CallRecord {
            id,
            timestamp,
            timestamp_ms,
            conversation_id,
            thread_id,
            turn_id,
            response_id,
            model,
            effort: context.effort.clone(),
            service_tier: context.tier.clone(),
            usage,
            fresh_input_tokens,
            cache_hit_rate,
            issues,
            cwd: self.cwd.clone(),
        })
    }
}
fn counter(
    raw: &Value,
    name: &str,
    nested: Option<(&str, &str)>,
    issues: &mut Vec<String>,
) -> Option<u64> {
    let value = raw
        .get(name)
        .or_else(|| nested.and_then(|(parent, key)| raw.get(parent)?.get(key)));
    match value {
        Some(value) => match value.as_u64().filter(|n| *n <= MAX_SAFE_INTEGER) {
            Some(n) => Some(n),
            None => {
                issues.push(format!("invalid_{name}"));
                None
            }
        },
        None => None,
    }
}
pub(super) fn normalize_usage(raw: &Value) -> (Usage, Vec<String>) {
    let mut issues = Vec::new();
    let input_tokens = counter(raw, "input_tokens", None, &mut issues);
    let mut cached_input_tokens = counter(
        raw,
        "cached_input_tokens",
        Some(("input_tokens_details", "cached_tokens")),
        &mut issues,
    );
    let cache_write_input_tokens = counter(
        raw,
        "cache_write_input_tokens",
        Some(("input_tokens_details", "cache_write_tokens")),
        &mut issues,
    );
    let output_tokens = counter(raw, "output_tokens", None, &mut issues);
    let reasoning_output_tokens = counter(
        raw,
        "reasoning_output_tokens",
        Some(("output_tokens_details", "reasoning_tokens")),
        &mut issues,
    );
    if input_tokens.is_none() {
        issues.push("missing_input_tokens".into());
    }
    if output_tokens.is_none() {
        issues.push("missing_output_tokens".into());
    }
    if cached_input_tokens.is_none() {
        issues.push("missing_cached_input_tokens".into());
    }
    if input_tokens
        .zip(cached_input_tokens)
        .is_some_and(|(input, cached)| cached > input)
    {
        issues.push("cache_exceeds_input".into());
        cached_input_tokens = None;
    }
    let explicit_total = counter(raw, "total_tokens", None, &mut issues);
    let sum = input_tokens
        .zip(output_tokens)
        .and_then(|(a, b)| a.checked_add(b))
        .filter(|n| *n <= MAX_SAFE_INTEGER);
    let total_tokens = if raw.get("total_tokens").is_some() {
        explicit_total
    } else {
        sum
    };
    if total_tokens.is_none() {
        issues.push("missing_total_tokens".into());
    }
    if let (Some(total), Some(sum)) = (explicit_total, sum) {
        if total != sum {
            issues.push("total_mismatch".into());
        }
    }
    (
        Usage {
            input_tokens,
            cached_input_tokens,
            cache_write_input_tokens,
            output_tokens,
            reasoning_output_tokens,
            total_tokens,
        },
        issues,
    )
}
