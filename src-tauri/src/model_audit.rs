//! Read-only, bounded ingestion of proxy metadata. No network traffic or response bodies.
use chrono::DateTime;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet, VecDeque};
use std::fs::{self, File};
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

const MAX_LINE: usize = 64 * 1024;
const SCAN_BUDGET: usize = 1024 * 1024;
const FILE_BUDGET: usize = 256 * 1024;
const MAX_OBSERVATIONS: usize = 50_000;
const MAX_ARRAY: usize = 32;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ModelAudit {
    pub status: String,
    pub sent_model: Option<String>,
    pub reported_model: Option<String>,
    pub observed_models: Vec<String>,
    pub sources: Vec<String>,
    pub upstream: String,
    pub observed_at: String,
    pub completed: bool,
    pub capture_issues: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ModelAuditStatus {
    pub directory: String,
    /// Number of distinct response IDs currently retained as evidence.
    pub observations: usize,
    /// Calls associated by response ID, regardless of their audit verdict.
    pub matched_calls: usize,
    pub parse_errors: usize,
    pub last_observed_at: Option<String>,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
struct Observation {
    schema_version: u32,
    observed_at: String,
    upstream: String,
    transport: String,
    #[serde(default)]
    response_id: Option<String>,
    #[serde(default)]
    requested_model: Option<String>,
    #[serde(default)]
    reported_model: Option<String>,
    #[serde(default)]
    http_header_models: Vec<String>,
    #[serde(default)]
    event_header_models: Vec<String>,
    #[serde(default)]
    response_body_models: Vec<String>,
    #[serde(default)]
    completed: bool,
    #[serde(default)]
    capture_issues: Vec<String>,
    // verdict, evidenceSources and eventTypes are deliberately not trusted or stored.
}

pub(crate) fn valid_identifier(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 256
        && value.as_bytes()[0].is_ascii_alphanumeric()
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"._:/+@-".contains(&b))
}

fn valid_origin(value: &str) -> bool {
    let Some(authority) = value
        .strip_prefix("https://")
        .or_else(|| value.strip_prefix("http://"))
    else {
        return false;
    };
    if value.len() > 2048 || authority.is_empty() {
        return false;
    }
    let (host, port) = if authority.starts_with('[') {
        let Some(end) = authority.find(']') else {
            return false;
        };
        if authority[1..end].parse::<std::net::Ipv6Addr>().is_err() {
            return false;
        }
        let rest = &authority[end + 1..];
        if !rest.is_empty() && !rest.starts_with(':') {
            return false;
        }
        (&authority[..=end], rest.strip_prefix(':'))
    } else {
        let mut parts = authority.split(':');
        let host = parts.next().unwrap_or_default();
        let port = parts.next();
        if parts.next().is_some()
            || !host
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b".-".contains(&b))
        {
            return false;
        }
        (host, port)
    };
    !host.is_empty()
        && port.map_or(true, |p| {
            !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()) && p.parse::<u16>().is_ok()
        })
}

impl Observation {
    fn parse(line: &[u8]) -> Option<Self> {
        if line.len() > MAX_LINE {
            return None;
        }
        let row: Self = serde_json::from_slice(line).ok()?;
        if row.schema_version != 1
            || !matches!(row.transport.as_str(), "http_sse" | "http_json")
            || !valid_origin(&row.upstream)
            || DateTime::parse_from_rfc3339(&row.observed_at)
                .ok()?
                .offset()
                .local_minus_utc()
                != 0
            || [&row.response_id, &row.requested_model, &row.reported_model]
                .iter()
                .any(|v| v.as_ref().is_some_and(|s| !valid_identifier(s)))
            || [
                &row.http_header_models,
                &row.event_header_models,
                &row.response_body_models,
            ]
            .iter()
            .any(|v| v.len() > MAX_ARRAY || v.iter().any(|s| !valid_identifier(s)))
            || row.capture_issues.len() > MAX_ARRAY
            || row
                .capture_issues
                .iter()
                .any(|s| s.len() > 256 || s.chars().any(char::is_control))
        {
            return None;
        }
        // Also bound ignored input arrays. They cannot supply model evidence.
        let value: serde_json::Value = serde_json::from_slice(line).ok()?;
        if value
            .as_object()?
            .values()
            .any(|v| v.as_array().is_some_and(|a| a.len() > MAX_ARRAY))
        {
            return None;
        }
        Some(row)
    }

    fn audit(&self) -> ModelAudit {
        let mut models = Vec::new();
        let mut sources = Vec::new();
        for (values, source) in [
            (&self.response_body_models, "response_body"),
            (&self.http_header_models, "http_header"),
            (&self.event_header_models, "event_header"),
        ] {
            if !values.is_empty() {
                sources.push(source.to_owned());
            }
            for model in values {
                push_unique(&mut models, model);
            }
        }
        // Only source-backed declarations can supply the displayed model.
        let mut capture_issues = self.capture_issues.clone();
        let reported_model = match &self.reported_model {
            Some(model) if models.contains(model) => Some(model.clone()),
            Some(_) => {
                push_unique(&mut capture_issues, "inconsistent_metadata");
                None
            }
            None => models.first().cloned(),
        };
        if !self.completed {
            push_unique(&mut capture_issues, "incomplete_response");
        }
        // Count before bounding the public union (each input array is already bounded).
        let conflict = models.len() > 1;
        models.truncate(MAX_ARRAY);
        ModelAudit {
            status: if conflict { "conflict" } else { "unknown" }.into(),
            sent_model: self.requested_model.clone(),
            reported_model,
            observed_models: models,
            sources,
            upstream: self.upstream.clone(),
            observed_at: self.observed_at.clone(),
            completed: self.completed,
            capture_issues,
        }
    }
}

fn push_unique(values: &mut Vec<String>, value: &str) {
    if !values.iter().any(|v| v == value) {
        values.push(value.to_owned());
    }
}

struct CachedObservation {
    original: Observation,
    audit: ModelAudit,
}

#[derive(Default)]
struct Cursor {
    offset: u64,
    remainder: Vec<u8>,
    discarding: bool,
    modified: Option<SystemTime>,
    identity: Option<(u64, u64)>,
    // Verify a small tail at the offset to notice truncate-and-regrow between polls.
    checkpoint: Vec<u8>,
}

#[cfg(unix)]
fn identity(meta: &fs::Metadata) -> Option<(u64, u64)> {
    use std::os::unix::fs::MetadataExt;
    Some((meta.dev(), meta.ino()))
}
#[cfg(not(unix))]
fn identity(_meta: &fs::Metadata) -> Option<(u64, u64)> {
    None
}

#[derive(Default)]
pub(crate) struct AuditStore {
    files: HashMap<PathBuf, Cursor>,
    observations: HashMap<String, CachedObservation>,
    order: VecDeque<String>,
    parse_errors: usize,
    last_observed_at: Option<String>,
    next_file: usize,
}

impl AuditStore {
    pub fn status(&self, directory: &Path, matched_calls: usize) -> ModelAuditStatus {
        ModelAuditStatus {
            directory: directory.display().to_string(),
            observations: self.observations.len(),
            matched_calls,
            parse_errors: self.parse_errors,
            last_observed_at: self.last_observed_at.clone(),
        }
    }

    pub fn lookup(&self, response_id: Option<&str>, local_model: &str) -> Option<ModelAudit> {
        let id = response_id.filter(|id| valid_identifier(id))?;
        let mut audit = self.observations.get(id)?.audit.clone();
        if audit.status != "conflict" {
            audit.status = if !audit.capture_issues.is_empty()
                || local_model == "unknown"
                || !valid_identifier(local_model)
            {
                "unknown"
            } else {
                match audit.reported_model.as_deref() {
                    Some("unknown") | None => "unknown",
                    Some(model) if model == local_model => "match",
                    Some(_) => "different",
                }
            }
            .into();
        }
        Some(audit)
    }

    fn ingest(&mut self, line: &[u8]) {
        if line.iter().all(u8::is_ascii_whitespace) {
            return;
        }
        let Some(row) = Observation::parse(line) else {
            self.parse_errors += 1;
            return;
        };
        if self.last_observed_at.as_ref().map_or(true, |previous| {
            DateTime::parse_from_rfc3339(&row.observed_at).ok()
                > DateTime::parse_from_rfc3339(previous).ok()
        }) {
            self.last_observed_at = Some(row.observed_at.clone());
        }
        let Some(id) = row.response_id.clone() else {
            return;
        };
        if let Some(existing) = self.observations.get_mut(&id) {
            if existing.original == row {
                return;
            }
            let other = row.audit();
            // Nonidentical attempts sharing an ID cannot safely identify one request.
            existing.audit.status = "conflict".into();
            if existing.audit.sent_model != other.sent_model {
                existing.audit.sent_model = None;
            }
            if existing.audit.reported_model != other.reported_model {
                existing.audit.reported_model = None;
            }
            existing.audit.completed &= other.completed;
            for model in other.observed_models {
                push_unique(&mut existing.audit.observed_models, &model);
            }
            existing.audit.observed_models.truncate(MAX_ARRAY);
            for source in other.sources {
                push_unique(&mut existing.audit.sources, &source);
            }
            for issue in other.capture_issues {
                push_unique(&mut existing.audit.capture_issues, &issue);
            }
            existing.audit.capture_issues.truncate(MAX_ARRAY - 1);
            push_unique(&mut existing.audit.capture_issues, "response_id_reused");
            return;
        }
        if self.observations.len() >= MAX_OBSERVATIONS {
            if let Some(oldest) = self.order.pop_front() {
                self.observations.remove(&oldest);
            }
        }
        let audit = row.audit();
        self.order.push_back(id.clone());
        self.observations.insert(
            id,
            CachedObservation {
                original: row,
                audit,
            },
        );
    }

    fn consume(&mut self, cursor: &mut Cursor, bytes: &[u8]) {
        for &byte in bytes {
            if byte == b'\n' {
                if !cursor.discarding {
                    self.ingest(&cursor.remainder);
                }
                cursor.remainder.clear();
                cursor.discarding = false;
            } else if !cursor.discarding {
                if cursor.remainder.len() == MAX_LINE {
                    cursor.remainder.clear();
                    cursor.discarding = true;
                    self.parse_errors += 1;
                } else {
                    cursor.remainder.push(byte);
                }
            }
        }
    }

    pub fn scan(&mut self, directory: &Path) {
        let Ok(entries) = fs::read_dir(directory) else {
            return;
        };
        let mut paths: Vec<_> = entries
            .flatten()
            .filter_map(|entry| {
                let path = entry.path();
                (entry.file_type().ok()?.is_file()
                    && path.extension().and_then(|s| s.to_str()) == Some("jsonl")
                    && path
                        .file_name()
                        .and_then(|s| s.to_str())
                        .is_some_and(|s| s.starts_with("capture-")))
                .then_some(path)
            })
            .collect();
        paths.sort();
        let present: HashSet<_> = paths.iter().cloned().collect();
        self.files.retain(|path, _| present.contains(path));
        if paths.is_empty() {
            return;
        }
        let start = self.next_file % paths.len();
        let mut budget = SCAN_BUDGET;
        for step in 0..paths.len() {
            let index = (start + step) % paths.len();
            self.next_file = (index + 1) % paths.len();
            self.scan_file(&paths[index], &mut budget);
            if budget == 0 {
                break;
            }
        }
    }

    fn scan_file(&mut self, path: &Path, budget: &mut usize) {
        let Ok(mut file) = File::open(path) else {
            return;
        };
        let Ok(meta) = file.metadata() else { return };
        let mut cursor = self.files.remove(path).unwrap_or_default();
        let modified = meta.modified().ok();
        let file_identity = identity(&meta);
        let mut reset = meta.len() < cursor.offset || cursor.identity != file_identity;
        if !reset && modified != cursor.modified && !cursor.checkpoint.is_empty() {
            let count = cursor.checkpoint.len();
            let mut previous = vec![0; count];
            if file
                .seek(SeekFrom::Start(cursor.offset - count as u64))
                .is_err()
                || file.read_exact(&mut previous).is_err()
                || previous != cursor.checkpoint
            {
                reset = true;
            }
            *budget = budget.saturating_sub(count);
        }
        if reset {
            cursor = Cursor::default();
        }
        cursor.identity = file_identity;
        cursor.modified = modified;
        let count = meta
            .len()
            .saturating_sub(cursor.offset)
            .min(FILE_BUDGET.min(*budget) as u64) as usize;
        if count > 0 && file.seek(SeekFrom::Start(cursor.offset)).is_ok() {
            let mut bytes = vec![0; count];
            if let Ok(read) = file.read(&mut bytes) {
                bytes.truncate(read);
                cursor.offset += read as u64;
                *budget -= read;
                self.consume(&mut cursor, &bytes);
                cursor.checkpoint.extend_from_slice(&bytes);
                let excess = cursor.checkpoint.len().saturating_sub(64);
                cursor.checkpoint.drain(..excess);
            }
        }
        self.files.insert(path.to_path_buf(), cursor);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{json, Value};

    pub(super) fn row(id: &str, model: &str) -> Value {
        json!({"schemaVersion":1,"observedAt":"2026-09-14T01:00:00Z",
            "upstream":"https://provider.example","transport":"http_sse",
            "responseId":id,"requestedModel":"sent-model","reportedModel":model,
            "httpHeaderModels":[],"eventHeaderModels":[],"responseBodyModels":[model],
            "evidenceSources":["untrusted"],"completed":true,"verdict":"match",
            "captureIssues":[],"eventTypes":{"response.completed":1}})
    }

    fn ingest(store: &mut AuditStore, value: &Value) {
        store.ingest(&serde_json::to_vec(value).unwrap());
    }

    #[test]
    fn recomputes_match_different_unknown_and_sources() {
        let mut store = AuditStore::default();
        ingest(&mut store, &row("resp-1", "model-1"));
        let audit = store.lookup(Some("resp-1"), "model-1").unwrap();
        assert_eq!(audit.status, "match");
        assert_eq!(audit.sent_model.as_deref(), Some("sent-model"));
        assert_eq!(audit.sources, vec!["response_body"]);
        assert_eq!(
            store
                .lookup(Some("resp-1"), "model-1-20260914")
                .unwrap()
                .status,
            "different"
        );
        assert_eq!(
            store.lookup(Some("resp-1"), "unknown").unwrap().status,
            "unknown"
        );
        assert_eq!(store.lookup(Some("resp-1"), "").unwrap().status, "unknown");
        let mut missing = row("resp-2", "model-1");
        missing["reportedModel"] = Value::Null;
        missing["responseBodyModels"] = json!([]);
        ingest(&mut store, &missing);
        assert_eq!(
            store.lookup(Some("resp-2"), "model-1").unwrap().status,
            "unknown"
        );
        let mut incomplete = row("resp-3", "model-1");
        incomplete["completed"] = json!(false);
        ingest(&mut store, &incomplete);
        let audit = store.lookup(Some("resp-3"), "model-1").unwrap();
        assert_eq!(audit.status, "unknown");
        assert!(audit
            .capture_issues
            .contains(&"incomplete_response".to_owned()));
    }

    #[test]
    fn conflicting_names_beat_capture_issues_and_preserve_incomplete() {
        let mut store = AuditStore::default();
        let mut value = row("resp-1", "model-1");
        value["httpHeaderModels"] = json!(["model-2"]);
        value["captureIssues"] = json!(["stream_interrupted"]);
        value["completed"] = json!(false);
        ingest(&mut store, &value);
        let audit = store.lookup(Some("resp-1"), "model-1").unwrap();
        assert_eq!(audit.status, "conflict");
        assert!(!audit.completed);
        assert_eq!(audit.observed_models, vec!["model-1", "model-2"]);
        value["responseId"] = json!("resp-2");
        value["httpHeaderModels"] = json!([]);
        ingest(&mut store, &value);
        assert_eq!(
            store.lookup(Some("resp-2"), "model-1").unwrap().status,
            "unknown"
        );
    }

    #[test]
    fn joins_only_exact_valid_response_ids() {
        let mut store = AuditStore::default();
        ingest(&mut store, &row("resp-1", "model-1"));
        for id in [
            None,
            Some("resp"),
            Some("RESP-1"),
            Some("resp-1 "),
            Some("turn-1"),
        ] {
            assert!(store.lookup(id, "model-1").is_none());
        }
        let mut missing = row("resp-2", "model-1");
        missing["responseId"] = Value::Null;
        ingest(&mut store, &missing);
        assert_eq!(store.observations.len(), 1);
    }

    #[test]
    fn repeats_deduplicate_but_response_reuse_conflicts() {
        let mut store = AuditStore::default();
        let first = row("resp-1", "model-1");
        ingest(&mut store, &first);
        ingest(&mut store, &first);
        assert_eq!(store.observations.len(), 1);
        assert_eq!(
            store.lookup(Some("resp-1"), "model-1").unwrap().status,
            "match"
        );
        let mut reused = first.clone();
        reused["observedAt"] = json!("2026-09-14T01:00:01Z");
        reused["requestedModel"] = json!("different-sent-model");
        ingest(&mut store, &reused);
        ingest(&mut store, &first);
        let audit = store.lookup(Some("resp-1"), "model-1").unwrap();
        assert_eq!(audit.status, "conflict");
        assert!(audit.sent_model.is_none());
        assert!(audit.capture_issues.contains(&"response_id_reused".into()));
    }

    #[test]
    fn partial_lines_wait_for_newline_and_oversized_lines_recover() {
        let mut store = AuditStore::default();
        let mut cursor = Cursor::default();
        let bytes = serde_json::to_vec(&row("resp-1", "model-1")).unwrap();
        store.consume(&mut cursor, &bytes[..20]);
        store.consume(&mut cursor, &bytes[20..]);
        assert!(store.observations.is_empty());
        store.consume(&mut cursor, b"\n");
        assert_eq!(store.observations.len(), 1);
        store.consume(&mut cursor, &vec![b'x'; MAX_LINE + 100]);
        assert!(cursor.remainder.is_empty());
        store.consume(&mut cursor, b"ignored\n");
        store.consume(&mut cursor, &bytes);
        store.consume(&mut cursor, b"\n");
        assert_eq!(store.parse_errors, 1);
        assert_eq!(store.observations.len(), 1);
    }

    #[test]
    fn rejects_invalid_metadata_and_ignores_claimed_sources() {
        let mut store = AuditStore::default();
        for (key, value) in [
            ("schemaVersion", json!(2)),
            ("responseId", json!("bad id")),
            ("requestedModel", json!("x".repeat(257))),
            ("reportedModel", json!("méta")),
            ("observedAt", json!("not-a-time")),
            ("upstream", json!("https://provider.example/path?secret=1")),
            ("upstream", json!("https://user:password@provider.example")),
            ("httpHeaderModels", json!(vec!["model-1"; 33])),
            ("transport", json!("websocket")),
        ] {
            let mut bad = row("resp-1", "model-1");
            bad[key] = value;
            ingest(&mut store, &bad);
        }
        assert_eq!(store.parse_errors, 9);
        assert!(store.observations.is_empty());
        let mut unsubstantiated = row("resp-1", "model-1");
        unsubstantiated["responseBodyModels"] = json!([]);
        ingest(&mut store, &unsubstantiated);
        let audit = store.lookup(Some("resp-1"), "model-1").unwrap();
        assert_eq!(audit.status, "unknown");
        assert!(audit.reported_model.is_none());
        assert!(audit.sources.is_empty());
    }
}
