//! Read-only, bounded session indexing. No model requests or credential/config reads.
mod catalogue;
mod discovery;
mod model;
mod parser;
mod reader;
mod store;
mod watch;

use chrono::Utc;
use discovery::{Discovery, MAX_FILES};
use model::{CallRecord, Catalog, Diagnostics, MonitorStatus};
pub use model::{Snapshot, Update};
use parser::{text, timestamp, Parser};
use reader::Cursor;
use serde_json::Value;
use std::collections::{HashMap, HashSet, VecDeque};
use std::env;
use std::hash::{Hash, Hasher};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use store::Store;
use watch::WatchService;

pub const POLL_MS: u64 = 750;
const MAX_RECORDS: usize = 50_000;
const MAX_TITLES: usize = 50_000;
const SCAN_BYTES: usize = 4 * 1024 * 1024;
const FILE_BYTES: usize = 256 * 1024;
const BUFFER_POOL: usize = 8 * 1024 * 1024;
const FILE_CHECKS: usize = 128;
const SCAN_TIME: Duration = Duration::from_millis(50);

#[derive(Default)]
struct FileState {
    cursor: Cursor,
    parser: Parser,
}
#[derive(Clone)]
pub struct Monitor {
    inner: Arc<Mutex<Engine>>,
}
struct Engine {
    roots: Vec<PathBuf>,
    index_path: PathBuf,
    store: Store,
    files: HashMap<PathBuf, FileState>,
    paths: Vec<PathBuf>,
    next_file: usize,
    priority: VecDeque<PathBuf>,
    queued: HashSet<PathBuf>,
    discovery: Discovery,
    watcher: Option<WatchService>,
    index_cursor: Cursor,
    titles: HashMap<String, (i64, String)>,
    title_limit_reached: bool,
    catalog: Catalog,
    catalog_dirty: bool,
    project_cache: HashMap<PathBuf, PathBuf>,
    diagnostics: Diagnostics,
    buffered: usize,
    revision: u64,
    status: MonitorStatus,
}
impl Monitor {
    pub fn from_env() -> Self {
        let home = env::var_os("HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("."));
        let home = env::var_os("CODEX_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| home.join(".codex"));
        let roots = env::var_os("CODEX_SESSION_ROOT")
            .map(|value| env::split_paths(&value).take(32).collect::<Vec<_>>())
            .filter(|roots| !roots.is_empty())
            .unwrap_or_else(|| vec![home.join("sessions"), home.join("archived_sessions")]);
        let index = env::var_os("CODEX_SESSION_INDEX")
            .map(PathBuf::from)
            .unwrap_or_else(|| home.join("session_index.jsonl"));
        let limit = env::var("WTM_RECORD_LIMIT")
            .ok()
            .and_then(|s| s.parse::<usize>().ok())
            .filter(|n| (100..=100_000).contains(n))
            .unwrap_or(MAX_RECORDS);
        Self::with_paths(roots, index, limit, true)
    }
    fn with_paths(roots: Vec<PathBuf>, index_path: PathBuf, limit: usize, watch: bool) -> Self {
        let roots = roots
            .into_iter()
            .map(|p| p.canonicalize().unwrap_or(p))
            .collect::<Vec<_>>();
        let status = MonitorStatus {
            retention_limit: limit,
            poll_ms: POLL_MS,
            session_roots: roots
                .iter()
                .map(|p| p.to_string_lossy().into_owned())
                .collect(),
            session_index: index_path.to_string_lossy().into_owned(),
            ..MonitorStatus::default()
        };
        Self {
            inner: Arc::new(Mutex::new(Engine {
                roots,
                index_path,
                store: Store::new(limit),
                files: HashMap::new(),
                paths: Vec::new(),
                next_file: 0,
                priority: VecDeque::new(),
                queued: HashSet::new(),
                discovery: Discovery::default(),
                watcher: watch.then(WatchService::new),
                index_cursor: Cursor::default(),
                titles: HashMap::new(),
                title_limit_reached: false,
                catalog: Catalog::default(),
                catalog_dirty: false,
                project_cache: HashMap::new(),
                diagnostics: Diagnostics::default(),
                buffered: 0,
                revision: 0,
                status,
            })),
        }
    }
    pub fn scan(&self) -> Update {
        self.inner.lock().unwrap_or_else(|p| p.into_inner()).scan()
    }
    pub fn snapshot(&self) -> Snapshot {
        let inner = self.inner.lock().unwrap_or_else(|p| p.into_inner());
        Snapshot {
            revision: inner.revision,
            calls: inner.store.snapshot(),
            catalog: inner.catalog.clone(),
            status: inner.status.clone(),
        }
    }
}
impl Engine {
    fn queue(&mut self, path: PathBuf) {
        if self.queued.len() < 1024 && self.queued.insert(path.clone()) {
            self.priority.push_back(path);
        }
    }
    fn add_path(&mut self, path: PathBuf) {
        if !self.files.contains_key(&path) && self.files.len() < MAX_FILES {
            self.files.insert(path.clone(), FileState::default());
            self.paths.push(path.clone());
            self.queue(path);
        }
    }
    fn watch_events(&mut self) {
        let Some(watcher) = self.watcher.as_mut() else {
            return;
        };
        watcher.refresh(&self.roots, &self.index_path);
        let (events, overflow) = watcher.drain();
        if overflow {
            self.discovery.request();
        }
        for event in events {
            if event.kind.is_create() || event.kind.is_remove() {
                self.discovery.request();
            }
            for path in event.paths {
                if path.extension().is_some_and(|e| e == "jsonl")
                    && self.roots.iter().any(|root| path.starts_with(root))
                    && path.is_file()
                {
                    if self.discovery.active && self.discovery.seen.len() < MAX_FILES {
                        self.discovery.seen.insert(path.clone());
                    }
                    self.add_path(path.clone());
                    self.queue(path);
                }
            }
        }
    }
    fn insert(
        &mut self,
        call: CallRecord,
        changed: &mut HashMap<String, CallRecord>,
        removed: &mut HashSet<String>,
    ) {
        let (added, evicted) = self.store.insert(call.clone());
        if added {
            self.catalog_dirty = true;
            changed.insert(call.id.clone(), call);
            if let Some(id) = evicted {
                changed.remove(&id);
                removed.insert(id);
            }
        }
    }

    fn poll_file(
        &mut self,
        path: &PathBuf,
        budget: usize,
        deadline: Instant,
        changed: &mut HashMap<String, CallRecord>,
        removed: &mut HashSet<String>,
    ) -> (usize, bool) {
        let Some(mut state) = self.files.remove(path) else {
            return (0, false);
        };
        self.buffered = self.buffered.saturating_sub(state.cursor.remainder.len());
        let mut read = 0;
        let mut success = true;
        match state.cursor.open(path) {
            Ok(Some(mut file)) => {
                if state.cursor.reset {
                    state.parser = Parser::default();
                }
                let mut hasher = std::collections::hash_map::DefaultHasher::new();
                path.hash(&mut hasher);
                let source = format!("file:{:x}", hasher.finish());
                let mut diagnostics = std::mem::take(&mut self.diagnostics);
                let mut oversized = 0;
                let parser = &mut state.parser;
                let before = state.cursor.offset;
                let result = state.cursor.read(
                    &mut file,
                    budget,
                    BUFFER_POOL.saturating_sub(self.buffered),
                    deadline,
                    |bytes, offset| {
                        if bytes.iter().all(u8::is_ascii_whitespace) {
                            return;
                        }
                        match serde_json::from_slice::<Value>(bytes) {
                            Ok(row) => {
                                if row.get("type").and_then(Value::as_str) == Some("session_meta") {
                                    self.catalog_dirty = true;
                                }
                                if let Some(call) =
                                    parser.consume(&row, offset, &source, &mut diagnostics)
                                {
                                    self.insert(call, changed, removed);
                                }
                            }
                            Err(_) => diagnostics.parse_errors += 1,
                        }
                    },
                    &mut oversized,
                );
                read = state.cursor.offset.saturating_sub(before) as usize;
                diagnostics.oversized_lines += oversized;
                self.diagnostics = diagnostics;
                if result.is_err() {
                    state.cursor.unreadable = true;
                    success = false;
                }
            }
            Ok(None) => {}
            Err(_) => {
                state.cursor.unreadable = true;
                state.cursor.initialized = true;
                success = false;
            }
        }
        state.cursor.remainder.shrink_to_fit();
        self.buffered += state.cursor.remainder.len();
        let pending = !state.cursor.unreadable && state.cursor.offset < state.cursor.size;
        self.files.insert(path.clone(), state);
        if pending {
            self.queue(path.clone());
        }
        (read, success)
    }
    fn poll_index(&mut self, budget: usize, deadline: Instant) -> (usize, bool) {
        self.buffered = self
            .buffered
            .saturating_sub(self.index_cursor.remainder.len());
        let opened = self.index_cursor.open(&self.index_path);
        let mut read = 0;
        let mut success = true;
        match opened {
            Ok(Some(mut file)) => {
                if self.index_cursor.reset {
                    self.titles.clear();
                    self.title_limit_reached = false;
                    self.catalog_dirty = true;
                }
                let titles = &mut self.titles;
                let dirty = &mut self.catalog_dirty;
                let limited = &mut self.title_limit_reached;
                let diagnostics = &mut self.diagnostics;
                let mut oversized = 0;
                let before = self.index_cursor.offset;
                let result = self.index_cursor.read(
                    &mut file,
                    budget,
                    BUFFER_POOL.saturating_sub(self.buffered),
                    deadline,
                    |bytes, _| {
                        if bytes.iter().all(u8::is_ascii_whitespace) {
                            return;
                        }
                        let Ok(row) = serde_json::from_slice::<Value>(bytes) else {
                            diagnostics.parse_errors += 1;
                            return;
                        };
                        let Some(id) = text(&row, "id", 256) else {
                            diagnostics.invalid_records += 1;
                            return;
                        };
                        let title = text(&row, "thread_name", 1024)
                            .unwrap_or_else(|| "Untitled conversation".into());
                        let updated = row
                            .get("updated_at")
                            .and_then(Value::as_str)
                            .and_then(timestamp)
                            .map(|(_, ms)| ms)
                            .unwrap_or(i64::MIN);
                        if !titles.contains_key(&id) && titles.len() >= MAX_TITLES {
                            *limited = true;
                            return;
                        }
                        if titles
                            .get(&id)
                            .is_none_or(|old| (updated, &title) > (old.0, &old.1))
                        {
                            titles.insert(id, (updated, title));
                            *dirty = true;
                        }
                    },
                    &mut oversized,
                );
                diagnostics.oversized_lines += oversized;
                read = self.index_cursor.offset.saturating_sub(before) as usize;
                if result.is_err() {
                    self.index_cursor.unreadable = true;
                    success = false;
                }
            }
            Ok(None) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                if !self.titles.is_empty() {
                    self.catalog_dirty = true;
                    self.titles.clear();
                }
                self.index_cursor = Cursor::default();
                self.index_cursor.initialized = true;
                success = false;
            }
            Err(_) => {
                self.index_cursor.unreadable = true;
                self.index_cursor.initialized = true;
                success = false;
            }
        }
        self.index_cursor.remainder.shrink_to_fit();
        self.buffered += self.index_cursor.remainder.len();
        (read, success)
    }
    fn scan(&mut self) -> Update {
        let started = Instant::now();
        self.watch_events();
        let (added, discovered) = self.discovery.step(&self.roots);
        for path in added {
            self.add_path(path);
        }
        if discovered {
            if !self.discovery.limited && self.discovery.errors == 0 {
                self.files
                    .retain(|path, _| self.discovery.seen.contains(path));
                self.paths.retain(|path| self.files.contains_key(path));
                self.priority.retain(|path| self.files.contains_key(path));
                self.queued.retain(|path| self.files.contains_key(path));
                self.buffered = self
                    .files
                    .values()
                    .map(|f| f.cursor.remainder.len())
                    .sum::<usize>()
                    + self.index_cursor.remainder.len();
            }
            self.project_cache.clear();
            self.catalog_dirty = true;
        }
        let mut changed = HashMap::new();
        let mut removed = HashSet::new();
        let (mut read_bytes, mut any_success) = self.poll_index(FILE_BYTES, started + SCAN_TIME);
        let mut checked = HashSet::new();
        for step in 0..FILE_CHECKS {
            if read_bytes >= SCAN_BYTES || started.elapsed() >= SCAN_TIME {
                break;
            }
            let preferred = if step % 2 == 0 {
                self.priority.pop_front()
            } else {
                None
            };
            let path = match preferred {
                Some(path) => {
                    self.queued.remove(&path);
                    path
                }
                None if !self.paths.is_empty() => {
                    self.next_file %= self.paths.len();
                    let path = self.paths[self.next_file].clone();
                    self.next_file += 1;
                    path
                }
                None => break,
            };
            if !checked.insert(path.clone()) {
                continue;
            }
            let allowance = (SCAN_BYTES / self.paths.len().max(1)).max(FILE_BYTES);
            let (bytes, ok) = self.poll_file(
                &path,
                allowance.min(SCAN_BYTES - read_bytes),
                started + SCAN_TIME,
                &mut changed,
                &mut removed,
            );
            read_bytes += bytes;
            any_success |= ok;
        }
        let mut catalog = None;
        if self.catalog_dirty {
            let mut root_cwds = HashMap::new();
            for call in self.store.calls.values().rev() {
                if call.thread_id == call.conversation_id {
                    if let Some(cwd) = &call.cwd {
                        root_cwds
                            .entry(call.conversation_id.clone())
                            .or_insert_with(|| cwd.clone());
                    }
                }
            }
            let mut fallback: Vec<_> = self.files.iter().collect();
            fallback.sort_unstable_by_key(|(path, _)| *path);
            for (_, file) in fallback {
                if file.parser.thread_id == file.parser.session_id {
                    if let (Some(id), Some(cwd)) = (&file.parser.session_id, &file.parser.cwd) {
                        root_cwds.entry(id.clone()).or_insert_with(|| cwd.clone());
                    }
                }
            }
            let next = catalogue::build(
                &self.store,
                &self.titles,
                &root_cwds,
                &mut self.project_cache,
            );
            if next != self.catalog {
                self.catalog = next;
                catalog = Some(self.catalog.clone());
            }
            self.catalog_dirty = false;
        }
        self.revision += 1;
        let cursors: Vec<_> = self
            .files
            .values()
            .map(|f| &f.cursor)
            .chain(std::iter::once(&self.index_cursor))
            .collect();
        let pending_files = cursors
            .iter()
            .filter(|c| !c.unreadable && (!c.initialized || c.offset < c.size))
            .count();
        let now = Utc::now().to_rfc3339();
        let success = any_success
            || (discovered
                && self.discovery.errors == 0
                && self.discovery.missing_roots < self.roots.len());
        self.status = MonitorStatus {
            revision: self.revision,
            records: self.store.calls.len(),
            retention_limit: self.store.limit,
            truncated: self.store.truncated,
            oldest_at: self
                .store
                .calls
                .first_key_value()
                .map(|(_, c)| c.timestamp.clone()),
            newest_at: self
                .store
                .calls
                .last_key_value()
                .map(|(_, c)| c.timestamp.clone()),
            conversations: self.catalog.conversations.len(),
            projects: self.catalog.projects.len(),
            files: self.files.len(),
            indexed_files: self.files.values().filter(|f| f.cursor.initialized).count(),
            pending_files,
            pending_bytes: cursors
                .iter()
                .map(|c| c.size.saturating_sub(c.offset))
                .sum(),
            partial_lines: cursors.iter().filter(|c| !c.remainder.is_empty()).count(),
            discovering: self.discovery.active,
            initial_load_complete: self.status.initial_load_complete
                || (!self.discovery.active && pending_files == 0),
            file_limit_reached: self.discovery.limited || self.files.len() >= MAX_FILES,
            metadata_limit_reached: self.title_limit_reached,
            unreadable_files: cursors.iter().filter(|c| c.unreadable).count(),
            missing_roots: self.discovery.missing_roots,
            discovery_errors: self.discovery.errors,
            parse_errors: self.diagnostics.parse_errors,
            invalid_records: self.diagnostics.invalid_records,
            legacy_records: self.diagnostics.legacy_records,
            oversized_lines: self.diagnostics.oversized_lines,
            records_with_issues: self
                .store
                .calls
                .values()
                .filter(|c| !c.issues.is_empty())
                .count(),
            watcher_enabled: self.watcher.as_ref().is_some_and(WatchService::enabled),
            watcher_errors: self.watcher.as_ref().map_or(0, |w| w.errors),
            watcher_overflows: self.watcher.as_ref().map_or(0, |w| w.overflows),
            scan_ms: started.elapsed().as_millis() as u64,
            read_bytes: read_bytes as u64,
            poll_ms: POLL_MS,
            session_roots: self
                .roots
                .iter()
                .map(|p| p.to_string_lossy().into_owned())
                .collect(),
            session_index: self.index_path.to_string_lossy().into_owned(),
            last_scan_at: Some(now.clone()),
            last_success_at: if success {
                Some(now)
            } else {
                self.status.last_success_at.clone()
            },
        };
        let mut calls: Vec<_> = changed
            .into_values()
            .filter(|c| self.store.contains(&c.id))
            .collect();
        calls.sort_unstable_by(|a, b| (a.timestamp_ms, &a.id).cmp(&(b.timestamp_ms, &b.id)));
        let mut removed_ids: Vec<_> = removed
            .into_iter()
            .filter(|id| !self.store.contains(id))
            .collect();
        removed_ids.sort_unstable();
        Update {
            revision: self.revision,
            calls,
            removed_ids,
            catalog,
            status: self.status.clone(),
        }
    }
}

#[cfg(test)]
mod tests;
