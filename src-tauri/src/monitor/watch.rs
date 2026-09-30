use notify::{Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    mpsc::{sync_channel, Receiver},
    Arc,
};
use std::time::{Duration, Instant};

pub(super) struct WatchService {
    watcher: Option<RecommendedWatcher>,
    receiver: Receiver<Event>,
    overflow: Arc<AtomicBool>,
    registered: HashSet<PathBuf>,
    next_refresh: Instant,
    pub errors: usize,
    pub overflows: u64,
}
impl WatchService {
    pub fn new() -> Self {
        let (sender, receiver) = sync_channel(1024);
        let overflow = Arc::new(AtomicBool::new(false));
        let flag = overflow.clone();
        let watcher =
            notify::recommended_watcher(move |event: notify::Result<Event>| match event {
                Ok(event) if !matches!(event.kind, EventKind::Access(_)) => {
                    if sender.try_send(event).is_err() {
                        flag.store(true, Ordering::Relaxed);
                    }
                }
                Err(_) => flag.store(true, Ordering::Relaxed),
                _ => {}
            })
            .ok();
        Self {
            watcher,
            receiver,
            overflow,
            registered: HashSet::new(),
            next_refresh: Instant::now(),
            errors: 0,
            overflows: 0,
        }
    }
    pub fn enabled(&self) -> bool {
        self.watcher.is_some() && !self.registered.is_empty()
    }
    pub fn refresh(&mut self, roots: &[PathBuf], index: &Path) {
        if Instant::now() < self.next_refresh {
            return;
        }
        self.next_refresh = Instant::now() + Duration::from_secs(10);
        let Some(watcher) = &mut self.watcher else {
            self.errors = 1;
            return;
        };
        self.errors = 0;
        self.registered.retain(|path| path.exists());
        let mut paths: Vec<(PathBuf, RecursiveMode)> = roots
            .iter()
            .cloned()
            .map(|p| (p, RecursiveMode::Recursive))
            .collect();
        for root in roots
            .iter()
            .map(PathBuf::as_path)
            .chain(std::iter::once(index))
        {
            if let Some(parent) = root.parent() {
                paths.push((parent.to_path_buf(), RecursiveMode::NonRecursive));
            }
        }
        for (path, mode) in paths {
            if !path.exists() || self.registered.contains(&path) {
                continue;
            }
            match watcher.watch(&path, mode) {
                Ok(()) => {
                    self.registered.insert(path);
                }
                Err(_) => self.errors += 1,
            }
        }
    }
    pub fn drain(&mut self) -> (Vec<Event>, bool) {
        let events = self.receiver.try_iter().take(1024).collect();
        let overflow = self.overflow.swap(false, Ordering::Relaxed);
        if overflow {
            self.overflows += 1;
        }
        (events, overflow)
    }
}
