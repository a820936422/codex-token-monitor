use std::collections::{BTreeSet, HashSet, VecDeque};
use std::fs::{self, ReadDir};
use std::path::PathBuf;
use std::time::{Duration, Instant};

pub(super) const MAX_FILES: usize = 20_000;
const MAX_DIRS: usize = 20_000;
/// Directory iteration is incremental too; startup never walks an entire tree at once.
pub(super) struct Discovery {
    pub active: bool,
    pub limited: bool,
    pub errors: usize,
    pub missing_roots: usize,
    pub seen: BTreeSet<PathBuf>,
    stack: VecDeque<PathBuf>,
    visited: HashSet<PathBuf>,
    entries: Option<ReadDir>,
    next_due: Instant,
}
impl Default for Discovery {
    fn default() -> Self {
        Self {
            active: false,
            limited: false,
            errors: 0,
            missing_roots: 0,
            seen: BTreeSet::new(),
            stack: VecDeque::new(),
            visited: HashSet::new(),
            entries: None,
            next_due: Instant::now(),
        }
    }
}
impl Discovery {
    pub fn request(&mut self) {
        self.next_due = Instant::now();
    }
    pub fn step(&mut self, roots: &[PathBuf]) -> (Vec<PathBuf>, bool) {
        if !self.active && Instant::now() < self.next_due {
            return (Vec::new(), false);
        }
        if !self.active {
            self.active = true;
            self.limited = false;
            self.errors = 0;
            self.missing_roots = 0;
            self.seen.clear();
            self.stack.clear();
            self.visited.clear();
            self.entries = None;
            for root in roots {
                if root.is_dir() {
                    self.stack.push_back(root.clone());
                } else if !root.exists() {
                    self.missing_roots += 1;
                } else {
                    self.errors += 1;
                }
            }
        }
        let started = Instant::now();
        let mut added = Vec::new();
        for _ in 0..1024 {
            if started.elapsed() > Duration::from_millis(8) {
                break;
            }
            if self.entries.is_none() {
                let Some(dir) = self.stack.pop_front() else {
                    self.active = false;
                    self.next_due = Instant::now() + Duration::from_secs(10);
                    return (added, true);
                };
                if !self.visited.insert(dir.clone()) {
                    continue;
                }
                match fs::read_dir(dir) {
                    Ok(entries) => self.entries = Some(entries),
                    Err(_) => {
                        self.errors += 1;
                        continue;
                    }
                }
            }
            let Some(entry) = self.entries.as_mut().and_then(Iterator::next) else {
                self.entries = None;
                continue;
            };
            let Ok(entry) = entry else {
                self.errors += 1;
                continue;
            };
            let Ok(kind) = entry.file_type() else {
                self.errors += 1;
                continue;
            };
            let path = entry.path();
            if kind.is_dir() {
                if self.stack.len() + self.visited.len() < MAX_DIRS {
                    self.stack.push_back(path);
                } else {
                    self.limited = true;
                }
            } else if kind.is_file() && path.extension().is_some_and(|ext| ext == "jsonl") {
                if self.seen.len() < MAX_FILES {
                    if self.seen.insert(path.clone()) {
                        added.push(path);
                    }
                } else {
                    self.limited = true;
                }
            }
        }
        (added, false)
    }
}
