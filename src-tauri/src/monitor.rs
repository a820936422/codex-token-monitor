use crate::model_audit::{AuditStore, ModelAudit, ModelAuditStatus};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::env;
use std::fs::{self, File};
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::SystemTime;

pub const POLL_MS: u64 = 750;
const MAX_RECORDS: usize = 50_000;

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Usage {
    pub input_tokens: u64,
    pub cached_input_tokens: u64,
    pub cache_write_input_tokens: u64,
    pub output_tokens: u64,
    pub reasoning_output_tokens: u64,
    pub total_tokens: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CallRecord {
    pub id: String,
    pub timestamp: String,
    pub conversation_id: String,
    pub thread_id: String,
    pub turn_id: Option<String>,
    pub response_id: Option<String>,
    pub model: String,
    #[serde(default)]
    pub model_audit: Option<ModelAudit>,
    pub effort: Option<String>,
    pub service_tier: String,
    pub usage: Usage,
    pub fresh_input_tokens: u64,
    pub cache_hit_rate: f64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Conversation {
    pub id: String,
    pub title: String,
    pub cwd: Option<String>,
    pub project_id: String,
    pub project_name: String,
    pub project_path: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Project {
    pub id: String,
    pub name: String,
    pub path: Option<String>,
    pub conversations: usize,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Catalog {
    pub projects: Vec<Project>,
    pub conversations: Vec<Conversation>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MonitorStatus {
    pub records: usize,
    pub conversations: usize,
    pub projects: usize,
    pub files: usize,
    pub parse_errors: usize,
    pub poll_ms: u64,
    pub session_roots: Vec<String>,
    pub session_index: String,
    pub last_scan_at: Option<String>,
    #[serde(default)]
    pub model_audit: ModelAuditStatus,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Snapshot {
    pub calls: Vec<CallRecord>,
    pub catalog: Catalog,
    pub status: MonitorStatus,
}

#[derive(Debug, Clone, Default)]
struct TurnContext {
    model: String,
    effort: Option<String>,
    service_tier: String,
}

#[derive(Debug, Clone)]
struct ParserState {
    thread_id: Option<String>,
    session_id: Option<String>,
    current_turn_id: Option<String>,
    current_model: String,
    current_effort: Option<String>,
    current_service_tier: String,
    turn_contexts: HashMap<String, TurnContext>,
}

impl Default for ParserState {
    fn default() -> Self {
        Self {
            thread_id: None,
            session_id: None,
            current_turn_id: None,
            current_model: "unknown".into(),
            current_effort: None,
            current_service_tier: "default".into(),
            turn_contexts: HashMap::new(),
        }
    }
}

#[derive(Debug, Default)]
struct FileState {
    offset: u64,
    remainder: Vec<u8>,
    parser: ParserState,
}

#[derive(Debug, Clone, Default)]
struct ConversationMeta {
    cwd: Option<String>,
    cwd_timestamp: String,
}

#[derive(Debug, Clone, Default)]
struct TitleEntry {
    title: String,
    updated_at: String,
}

#[derive(Default)]
struct Inner {
    calls: Vec<CallRecord>,
    seen_ids: HashSet<String>,
    files: HashMap<PathBuf, FileState>,
    audit: AuditStore,
    conversation_meta: HashMap<String, ConversationMeta>,
    titles: HashMap<String, TitleEntry>,
    index_fingerprint: Option<(u64, Option<SystemTime>)>,
    project_cache: HashMap<PathBuf, PathBuf>,
    catalog: Catalog,
    catalog_signature: String,
    parse_errors: usize,
    last_scan_at: Option<String>,
}

#[derive(Clone)]
pub struct Monitor {
    roots: Vec<PathBuf>,
    index_path: PathBuf,
    audit_directory: PathBuf,
    inner: Arc<Mutex<Inner>>,
}

#[derive(Default)]
pub struct ScanResult {
    pub new_calls: Vec<CallRecord>,
    pub catalog: Option<Catalog>,
    pub status: MonitorStatus,
}

impl Monitor {
    pub fn from_env() -> Self {
        let home = env::var_os("HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("."));
        let codex_home = env::var_os("CODEX_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| home.join(".codex"));
        let roots = env::var_os("CODEX_SESSION_ROOT")
            .map(|value| env::split_paths(&value).collect::<Vec<_>>())
            .filter(|items| !items.is_empty())
            .unwrap_or_else(|| {
                vec![
                    codex_home.join("sessions"),
                    codex_home.join("archived_sessions"),
                ]
            });
        let index_path = env::var_os("CODEX_SESSION_INDEX")
            .map(PathBuf::from)
            .unwrap_or_else(|| codex_home.join("session_index.jsonl"));
        Self {
            roots,
            index_path,
            audit_directory: env::var_os("CODEX_MODEL_AUDIT_DIR")
                .map(PathBuf::from)
                .unwrap_or_else(|| codex_home.join("model-audit")),
            inner: Arc::new(Mutex::new(Inner::default())),
        }
    }

    pub fn scan(&self) -> ScanResult {
        let mut inner = self.inner.lock().expect("monitor mutex poisoned");
        sync_session_index(&self.index_path, &mut inner);
        let files = self
            .roots
            .iter()
            .flat_map(|root| list_jsonl(root))
            .collect::<Vec<_>>();
        let mut new_calls = Vec::new();
        for file in files {
            sync_file(&file, &mut inner, &mut new_calls);
        }
        inner.audit.scan(&self.audit_directory);
        enrich_calls(&mut inner, &mut new_calls);
        hydrate_projects(&mut inner);
        let next_catalog = build_catalog(&inner);
        let signature = serde_json::to_string(&next_catalog).unwrap_or_default();
        let catalog = if signature != inner.catalog_signature {
            inner.catalog_signature = signature;
            inner.catalog = next_catalog.clone();
            Some(next_catalog)
        } else {
            None
        };
        inner.last_scan_at = Some(Utc::now().to_rfc3339());
        let status = make_status(&inner, &self.roots, &self.index_path, &self.audit_directory);
        ScanResult {
            new_calls,
            catalog,
            status,
        }
    }

    pub fn snapshot(&self) -> Snapshot {
        let inner = self.inner.lock().expect("monitor mutex poisoned");
        let mut calls = inner.calls.clone();
        calls.sort_by(|a, b| b.timestamp.cmp(&a.timestamp));
        Snapshot {
            calls,
            catalog: inner.catalog.clone(),
            status: make_status(&inner, &self.roots, &self.index_path, &self.audit_directory),
        }
    }
}

// A monitor-call event is an upsert. Audit-only changes must never insert usage again.
fn enrich_calls(inner: &mut Inner, new_calls: &mut Vec<CallRecord>) {
    let mut emitted: HashMap<String, usize> = new_calls
        .iter()
        .enumerate()
        .map(|(index, call)| (call.id.clone(), index))
        .collect();
    for call in &mut inner.calls {
        let Some(audit) = inner.audit.lookup(call.response_id.as_deref(), &call.model) else {
            // Keep already-associated evidence when its cache entry is evicted.
            continue;
        };
        if call.model_audit.as_ref() == Some(&audit) {
            continue;
        }
        call.model_audit = Some(audit);
        if let Some(&index) = emitted.get(&call.id) {
            new_calls[index] = call.clone();
        } else {
            emitted.insert(call.id.clone(), new_calls.len());
            new_calls.push(call.clone());
        }
    }
}

fn make_status(
    inner: &Inner,
    roots: &[PathBuf],
    index: &Path,
    audit_directory: &Path,
) -> MonitorStatus {
    MonitorStatus {
        records: inner.calls.len(),
        conversations: inner.catalog.conversations.len(),
        projects: inner.catalog.projects.len(),
        files: inner.files.len(),
        parse_errors: inner.parse_errors,
        poll_ms: POLL_MS,
        session_roots: roots.iter().map(|p| p.display().to_string()).collect(),
        session_index: index.display().to_string(),
        last_scan_at: inner.last_scan_at.clone(),
        model_audit: inner.audit.status(
            audit_directory,
            inner
                .calls
                .iter()
                .filter(|call| call.model_audit.is_some())
                .count(),
        ),
    }
}

fn list_jsonl(root: &Path) -> Vec<PathBuf> {
    let mut found = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(current) = stack.pop() {
        let Ok(entries) = fs::read_dir(current) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let Ok(kind) = entry.file_type() else {
                continue;
            };
            if kind.is_dir() {
                stack.push(path);
            } else if kind.is_file() && path.extension().and_then(|s| s.to_str()) == Some("jsonl") {
                found.push(path);
            }
        }
    }
    found
}

fn sync_file(path: &Path, inner: &mut Inner, new_calls: &mut Vec<CallRecord>) {
    let Ok(meta) = fs::metadata(path) else { return };
    let size = meta.len();
    let mut state = inner.files.remove(path).unwrap_or_default();
    if size < state.offset {
        state = FileState::default();
    }
    if size > state.offset {
        let mut bytes = Vec::with_capacity((size - state.offset) as usize);
        if let Ok(mut file) = File::open(path) {
            if file.seek(SeekFrom::Start(state.offset)).is_ok()
                && file.read_to_end(&mut bytes).is_ok()
            {
                state.offset = size;
                consume_bytes(&mut state, bytes, inner, new_calls);
            }
        }
    }
    inner.files.insert(path.to_path_buf(), state);
}

fn consume_bytes(
    state: &mut FileState,
    bytes: Vec<u8>,
    inner: &mut Inner,
    new_calls: &mut Vec<CallRecord>,
) {
    state.remainder.extend(bytes);
    let mut start = 0usize;
    let mut lines = Vec::new();
    for (i, byte) in state.remainder.iter().enumerate() {
        if *byte == b'\n' {
            lines.push(state.remainder[start..i].to_vec());
            start = i + 1;
        }
    }
    let tail = state.remainder[start..].to_vec();
    state.remainder = tail;
    for line in lines {
        if line.iter().all(u8::is_ascii_whitespace) {
            continue;
        }
        let Ok(row) = serde_json::from_slice::<Value>(&line) else {
            inner.parse_errors += 1;
            continue;
        };
        if let Some((conversation_id, cwd, timestamp)) = session_meta(&row) {
            let entry = inner.conversation_meta.entry(conversation_id).or_default();
            if cwd.is_some() && (entry.cwd.is_none() || timestamp >= entry.cwd_timestamp) {
                entry.cwd = cwd;
                entry.cwd_timestamp = timestamp;
            }
        }
        if let Some(call) = process_row(&row, &mut state.parser) {
            if inner.seen_ids.insert(call.id.clone()) {
                inner.calls.push(call.clone());
                new_calls.push(call);
                if inner.calls.len() > MAX_RECORDS {
                    let excess = inner.calls.len() - MAX_RECORDS;
                    inner.calls.drain(0..excess);
                }
            }
        }
    }
}

fn session_meta(row: &Value) -> Option<(String, Option<String>, String)> {
    if row.get("type")?.as_str()? != "session_meta" {
        return None;
    }
    let payload = row.get("payload")?;
    let thread_id = string(payload, "id");
    let conversation_id = string(payload, "session_id").or(thread_id)?;
    let cwd = string(payload, "cwd");
    let timestamp = string(payload, "timestamp")
        .or_else(|| string(row, "timestamp"))
        .unwrap_or_default();
    Some((conversation_id, cwd, timestamp))
}

fn process_row(row: &Value, state: &mut ParserState) -> Option<CallRecord> {
    let row_type = row.get("type").and_then(Value::as_str)?;
    let payload = row.get("payload").unwrap_or(&Value::Null);
    match row_type {
        "session_meta" => {
            state.thread_id = string(payload, "id").or(state.thread_id.clone());
            state.session_id = string(payload, "session_id")
                .or_else(|| string(payload, "id"))
                .or(state.session_id.clone());
            return None;
        }
        "turn_context" => {
            state.current_turn_id = string(payload, "turn_id").or(state.current_turn_id.clone());
            state.current_model =
                string(payload, "model").unwrap_or_else(|| state.current_model.clone());
            state.current_effort = string(payload, "effort").or(state.current_effort.clone());
            state.current_service_tier = string(payload, "service_tier")
                .unwrap_or_else(|| state.current_service_tier.clone());
            if let Some(id) = state.current_turn_id.clone() {
                state.turn_contexts.insert(
                    id,
                    TurnContext {
                        model: state.current_model.clone(),
                        effort: state.current_effort.clone(),
                        service_tier: state.current_service_tier.clone(),
                    },
                );
            }
            return None;
        }
        "event_msg" => {
            match payload.get("type").and_then(Value::as_str) {
                Some("task_started") => {
                    state.current_turn_id =
                        string(payload, "turn_id").or(state.current_turn_id.clone())
                }
                Some("thread_settings_applied") => {
                    if let Some(tier) = payload
                        .get("thread_settings")
                        .and_then(|v| string(v, "service_tier"))
                    {
                        state.current_service_tier = tier;
                    }
                }
                Some("task_complete") => {
                    let id = string(payload, "turn_id");
                    if id.is_none() || id == state.current_turn_id {
                        state.current_turn_id = None;
                    }
                }
                _ => {}
            }
            return None;
        }
        "token_usage_record" => {}
        _ => return None,
    }
    let usage_raw = payload.get("usage")?;
    let usage = normalize_usage(usage_raw);
    let turn_id = string(payload, "turn_id").or(state.current_turn_id.clone());
    let context = turn_id
        .as_ref()
        .and_then(|id| state.turn_contexts.get(id))
        .cloned()
        .unwrap_or_default();
    let thread_id = string(payload, "thread_id")
        .or(state.thread_id.clone())
        .or_else(|| string(payload, "session_id"))
        .or(state.session_id.clone())
        .unwrap_or_else(|| "unknown".into());
    let conversation_id = string(payload, "session_id")
        .or(state.session_id.clone())
        .unwrap_or_else(|| thread_id.clone());
    let response_id = string(payload, "response_id");
    let timestamp = string(row, "timestamp").unwrap_or_else(|| Utc::now().to_rfc3339());
    let id = response_id.clone().unwrap_or_else(|| {
        format!(
            "{}:{}:{}:{}:{}",
            thread_id,
            turn_id.clone().unwrap_or_else(|| "no-turn".into()),
            timestamp,
            usage.input_tokens,
            usage.output_tokens
        )
    });
    let model = if context.model.is_empty() {
        state.current_model.clone()
    } else {
        context.model
    };
    let effort = context.effort.or(state.current_effort.clone());
    let service_tier = if context.service_tier.is_empty() {
        state.current_service_tier.clone()
    } else {
        context.service_tier
    };
    let fresh_input_tokens = usage
        .input_tokens
        .saturating_sub(usage.cached_input_tokens)
        .saturating_sub(usage.cache_write_input_tokens);
    let cache_hit_rate = if usage.input_tokens > 0 {
        usage.cached_input_tokens as f64 / usage.input_tokens as f64 * 100.0
    } else {
        0.0
    };
    Some(CallRecord {
        id,
        timestamp,
        conversation_id,
        thread_id,
        turn_id,
        response_id,
        model,
        model_audit: None,
        effort,
        service_tier,
        usage,
        fresh_input_tokens,
        cache_hit_rate,
    })
}

fn normalize_usage(raw: &Value) -> Usage {
    let input_tokens = u64v(raw, "input_tokens");
    let cached_input_tokens = u64v(raw, "cached_input_tokens");
    let cache_write_input_tokens = u64v(raw, "cache_write_input_tokens");
    let output_tokens = u64v(raw, "output_tokens");
    let reasoning_output_tokens = u64v(raw, "reasoning_output_tokens");
    let total_tokens = match u64v(raw, "total_tokens") {
        0 => input_tokens + output_tokens,
        value => value,
    };
    Usage {
        input_tokens,
        cached_input_tokens,
        cache_write_input_tokens,
        output_tokens,
        reasoning_output_tokens,
        total_tokens,
    }
}

fn sync_session_index(path: &Path, inner: &mut Inner) {
    let Ok(meta) = fs::metadata(path) else { return };
    let fingerprint = (meta.len(), meta.modified().ok());
    if inner.index_fingerprint == Some(fingerprint) {
        return;
    }
    let Ok(text) = fs::read_to_string(path) else {
        return;
    };
    let mut latest: HashMap<String, TitleEntry> = HashMap::new();
    for line in text.lines().filter(|line| !line.trim().is_empty()) {
        let Ok(row) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        let Some(id) = string(&row, "id") else {
            continue;
        };
        let updated_at = string(&row, "updated_at").unwrap_or_default();
        let title = string(&row, "thread_name")
            .filter(|s| !s.trim().is_empty())
            .unwrap_or_else(|| "Untitled conversation".into());
        let replace = latest
            .get(&id)
            .map(|old| updated_at >= old.updated_at)
            .unwrap_or(true);
        if replace {
            latest.insert(id, TitleEntry { title, updated_at });
        }
    }
    inner.titles = latest;
    inner.index_fingerprint = Some(fingerprint);
}

fn hydrate_projects(inner: &mut Inner) {
    let paths = inner
        .conversation_meta
        .values()
        .filter_map(|m| m.cwd.clone())
        .collect::<HashSet<_>>();
    for cwd in paths {
        let path = PathBuf::from(&cwd);
        if inner.project_cache.contains_key(&path) {
            continue;
        }
        let mut current = path.clone();
        let mut project = path.clone();
        loop {
            if current.join(".git").exists() {
                project = current.clone();
                break;
            }
            let Some(parent) = current.parent() else {
                break;
            };
            if parent == current {
                break;
            }
            current = parent.to_path_buf();
        }
        inner.project_cache.insert(path, project);
    }
}

fn build_catalog(inner: &Inner) -> Catalog {
    let mut ids = inner
        .conversation_meta
        .keys()
        .cloned()
        .collect::<HashSet<_>>();
    ids.extend(inner.calls.iter().map(|call| call.conversation_id.clone()));
    let mut conversations = Vec::new();
    for id in ids.into_iter().filter(|id| id != "unknown") {
        let meta = inner.conversation_meta.get(&id);
        let cwd = meta.and_then(|m| m.cwd.clone());
        let project_path = cwd.as_ref().map(|cwd| {
            inner
                .project_cache
                .get(&PathBuf::from(cwd))
                .cloned()
                .unwrap_or_else(|| PathBuf::from(cwd))
        });
        let (project_id, project_name, project_path_string) = if let Some(path) = project_path {
            let text = path.display().to_string();
            let name = path
                .file_name()
                .and_then(|s| s.to_str())
                .unwrap_or(&text)
                .to_string();
            (text.clone(), name, Some(text))
        } else {
            ("__unassigned__".into(), "Other / Unassigned".into(), None)
        };
        let title = inner
            .titles
            .get(&id)
            .map(|t| t.title.clone())
            .unwrap_or_else(|| "Untitled conversation".into());
        conversations.push(Conversation {
            id,
            title,
            cwd,
            project_id,
            project_name,
            project_path: project_path_string,
        });
    }
    conversations.sort_by(|a, b| {
        a.title
            .to_lowercase()
            .cmp(&b.title.to_lowercase())
            .then_with(|| a.id.cmp(&b.id))
    });
    let mut projects_map: HashMap<String, Project> = HashMap::new();
    for convo in &conversations {
        let entry = projects_map
            .entry(convo.project_id.clone())
            .or_insert(Project {
                id: convo.project_id.clone(),
                name: convo.project_name.clone(),
                path: convo.project_path.clone(),
                conversations: 0,
            });
        entry.conversations += 1;
    }
    let mut projects = projects_map.into_values().collect::<Vec<_>>();
    projects.sort_by(|a, b| {
        a.name
            .to_lowercase()
            .cmp(&b.name.to_lowercase())
            .then_with(|| a.id.cmp(&b.id))
    });
    Catalog {
        projects,
        conversations,
    }
}

fn string(value: &Value, key: &str) -> Option<String> {
    value.get(key).and_then(Value::as_str).map(str::to_owned)
}
fn u64v(value: &Value, key: &str) -> u64 {
    value.get(key).and_then(Value::as_u64).unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_response_usage_and_parent_conversation() {
        let mut state = ParserState::default();
        let meta = serde_json::json!({"type":"session_meta","payload":{"id":"child","session_id":"parent","cwd":"/tmp/p"}});
        assert!(process_row(&meta, &mut state).is_none());
        let context = serde_json::json!({"type":"turn_context","payload":{"turn_id":"turn-1","model":"gpt-test","effort":"medium","service_tier":"default"}});
        assert!(process_row(&context, &mut state).is_none());
        let usage = serde_json::json!({"timestamp":"2026-09-14T01:00:00Z","type":"token_usage_record","payload":{"thread_id":"child","session_id":"parent","turn_id":"turn-1","response_id":"resp-1","usage":{"input_tokens":1000,"cached_input_tokens":900,"output_tokens":100,"reasoning_output_tokens":10,"total_tokens":1100}}});
        let call = process_row(&usage, &mut state).expect("call");
        assert_eq!(call.conversation_id, "parent");
        assert_eq!(call.thread_id, "child");
        assert_eq!(call.model, "gpt-test");
        assert_eq!(call.fresh_input_tokens, 100);
        assert!((call.cache_hit_rate - 90.0).abs() < 0.001);
    }

    #[test]
    fn total_tokens_falls_back_to_input_plus_output() {
        let usage = normalize_usage(
            &serde_json::json!({"input_tokens":42,"cached_input_tokens":40,"output_tokens":8}),
        );
        assert_eq!(usage.total_tokens, 50);
    }

    struct AuditFixture {
        directory: PathBuf,
        monitor: Monitor,
    }

    impl AuditFixture {
        fn new() -> Self {
            static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
            let directory = env::temp_dir().join(format!(
                "model-audit-monitor-{}-{}-{}",
                std::process::id(),
                SystemTime::now()
                    .duration_since(SystemTime::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos(),
                NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
            ));
            let sessions = directory.join("sessions");
            let audit_directory = directory.join("audit");
            fs::create_dir_all(&sessions).unwrap();
            fs::create_dir_all(&audit_directory).unwrap();
            let monitor = Monitor {
                roots: vec![sessions],
                index_path: directory.join("session_index.jsonl"),
                audit_directory,
                inner: Arc::new(Mutex::new(Inner::default())),
            };
            Self { directory, monitor }
        }

        fn write_call(&self) {
            fs::write(self.monitor.roots[0].join("session.jsonl"), concat!(
                "{\"type\":\"turn_context\",\"payload\":{\"turn_id\":\"turn-1\",\"model\":\"model-1\"}}\n",
                "{\"timestamp\":\"2026-09-14T01:00:00Z\",\"type\":\"token_usage_record\",\"payload\":{\"response_id\":\"resp-1\",\"turn_id\":\"turn-1\",\"usage\":{\"input_tokens\":40,\"output_tokens\":10}}}\n"
            )).unwrap();
        }

        fn write_audit(&self, id: &str, name: &str) {
            let row = serde_json::json!({
                "schemaVersion":1, "observedAt":"2026-09-14T01:00:00Z",
                "upstream":"https://provider.example", "transport":"http_json",
                "responseId":id, "requestedModel":"wire-model", "reportedModel":"model-1",
                "responseBodyModels":["model-1"], "completed":true,
            });
            fs::write(self.monitor.audit_directory.join(name), format!("{row}\n")).unwrap();
        }
    }

    impl Drop for AuditFixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.directory);
        }
    }

    #[test]
    fn late_audit_emits_upsert_without_duplicate_usage_and_persists_snapshot() {
        let fixture = AuditFixture::new();
        fixture.write_call();
        let first = fixture.monitor.scan();
        assert_eq!(first.new_calls.len(), 1);
        assert!(first.new_calls[0].model_audit.is_none());
        // Same turn/time/model cannot join a different response ID.
        fixture.write_audit("turn-1", "capture-unrelated.jsonl");
        assert!(fixture.monitor.scan().new_calls.is_empty());
        fixture.write_audit("resp-1", "capture-exact.jsonl");
        let updated = fixture.monitor.scan();
        assert_eq!(updated.new_calls.len(), 1);
        assert_eq!(updated.new_calls[0].id, first.new_calls[0].id);
        assert_eq!(updated.new_calls[0].usage, first.new_calls[0].usage);
        assert_eq!(
            updated.new_calls[0].model_audit.as_ref().unwrap().status,
            "match"
        );
        assert_eq!(updated.status.records, 1);
        assert_eq!(updated.status.model_audit.matched_calls, 1);
        assert_eq!(updated.status.model_audit.observations, 2);
        let snapshot = fixture.monitor.snapshot();
        assert_eq!(snapshot.calls.len(), 1);
        assert_eq!(
            snapshot
                .calls
                .iter()
                .map(|call| call.usage.total_tokens)
                .sum::<u64>(),
            50
        );
        assert!(snapshot.calls[0].model_audit.is_some());
        assert!(fixture.monitor.scan().new_calls.is_empty());
    }

    #[test]
    fn audit_before_call_enriches_the_first_event() {
        let fixture = AuditFixture::new();
        fixture.write_audit("resp-1", "capture-first.jsonl");
        assert!(fixture.monitor.scan().new_calls.is_empty());
        fixture.write_call();
        let scan = fixture.monitor.scan();
        assert_eq!(scan.new_calls.len(), 1);
        assert_eq!(
            scan.new_calls[0].model_audit.as_ref().unwrap().status,
            "match"
        );
        assert_eq!(scan.status.records, 1);
        assert!(fixture.monitor.scan().new_calls.is_empty());
    }

    #[test]
    fn audit_files_recover_after_partial_append_rotation_and_truncation() {
        use std::io::Write;
        let fixture = AuditFixture::new();
        fixture.write_call();
        let path = fixture.monitor.audit_directory.join("capture-live.jsonl");
        fixture.write_audit("resp-1", "capture-live.jsonl");
        let full = fs::read(&path).unwrap();
        fs::write(&path, &full[..full.len() - 1]).unwrap();
        assert!(fixture.monitor.scan().new_calls[0].model_audit.is_none());
        fs::OpenOptions::new()
            .append(true)
            .open(&path)
            .unwrap()
            .write_all(b"\n")
            .unwrap();
        assert_eq!(
            fixture.monitor.scan().new_calls[0]
                .model_audit
                .as_ref()
                .unwrap()
                .status,
            "match"
        );
        fixture.write_audit("resp-2", "capture-rotated.jsonl");
        assert_eq!(fixture.monitor.scan().status.model_audit.observations, 2);
        fs::write(&path, b"bad-json\n").unwrap();
        assert_eq!(fixture.monitor.scan().status.model_audit.parse_errors, 1);
        fixture.write_audit("resp-3", "capture-replacement.jsonl");
        fs::rename(
            fixture
                .monitor
                .audit_directory
                .join("capture-replacement.jsonl"),
            &path,
        )
        .unwrap();
        let final_scan = fixture.monitor.scan();
        assert_eq!(final_scan.status.model_audit.observations, 3);
        assert_eq!(final_scan.status.records, 1);
        assert!(final_scan.new_calls.is_empty());
    }

    #[test]
    fn old_call_and_status_json_default_audit_fields() {
        let fixture = AuditFixture::new();
        fixture.write_call();
        let call = fixture.monitor.scan().new_calls.remove(0);
        let mut value = serde_json::to_value(call).unwrap();
        value.as_object_mut().unwrap().remove("modelAudit");
        assert!(serde_json::from_value::<CallRecord>(value)
            .unwrap()
            .model_audit
            .is_none());
        let mut value = serde_json::to_value(MonitorStatus::default()).unwrap();
        value.as_object_mut().unwrap().remove("modelAudit");
        assert_eq!(
            serde_json::from_value::<MonitorStatus>(value)
                .unwrap()
                .model_audit
                .observations,
            0
        );
    }
}
