use serde::{Deserialize, Serialize};

/// Unknown counters stay null; missing measurement is not measured zero.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Usage {
    pub input_tokens: Option<u64>,
    pub cached_input_tokens: Option<u64>,
    pub cache_write_input_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
    pub reasoning_output_tokens: Option<u64>,
    pub total_tokens: Option<u64>,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CallRecord {
    pub id: String,
    pub timestamp: String,
    pub timestamp_ms: i64,
    pub conversation_id: String,
    pub thread_id: String,
    pub turn_id: Option<String>,
    pub response_id: Option<String>,
    pub model: String,
    pub effort: Option<String>,
    pub service_tier: String,
    pub usage: Usage,
    pub fresh_input_tokens: Option<u64>,
    pub cache_hit_rate: Option<f64>,
    pub issues: Vec<String>,
    #[serde(skip)]
    pub cwd: Option<String>,
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
    pub revision: u64,
    pub records: usize,
    pub retention_limit: usize,
    pub truncated: bool,
    pub oldest_at: Option<String>,
    pub newest_at: Option<String>,
    pub conversations: usize,
    pub projects: usize,
    pub files: usize,
    pub indexed_files: usize,
    pub pending_files: usize,
    pub pending_bytes: u64,
    pub partial_lines: usize,
    pub discovering: bool,
    pub initial_load_complete: bool,
    pub file_limit_reached: bool,
    pub metadata_limit_reached: bool,
    pub unreadable_files: usize,
    pub missing_roots: usize,
    pub discovery_errors: usize,
    pub parse_errors: u64,
    pub invalid_records: u64,
    pub legacy_records: u64,
    pub oversized_lines: u64,
    pub records_with_issues: usize,
    pub watcher_enabled: bool,
    pub watcher_errors: usize,
    pub watcher_overflows: u64,
    pub scan_ms: u64,
    pub read_bytes: u64,
    pub poll_ms: u64,
    pub session_roots: Vec<String>,
    pub session_index: String,
    pub last_scan_at: Option<String>,
    pub last_success_at: Option<String>,
}
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Snapshot {
    pub revision: u64,
    pub calls: Vec<CallRecord>,
    pub catalog: Catalog,
    pub status: MonitorStatus,
}
/// One ordered batch per scan, including retention deletions.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Update {
    pub revision: u64,
    pub calls: Vec<CallRecord>,
    pub removed_ids: Vec<String>,
    pub catalog: Option<Catalog>,
    pub status: MonitorStatus,
}
#[derive(Debug, Clone, Default)]
pub(super) struct Diagnostics {
    pub parse_errors: u64,
    pub invalid_records: u64,
    pub legacy_records: u64,
    pub oversized_lines: u64,
}
