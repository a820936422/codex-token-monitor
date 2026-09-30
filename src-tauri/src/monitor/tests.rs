use super::*;
use serde_json::json;
use std::fs;
use std::io::Write;
use std::sync::atomic::{AtomicU64, Ordering};

struct Fixture {
    directory: PathBuf,
    monitor: Monitor,
}
impl Fixture {
    fn new(limit: usize) -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let directory = env::temp_dir().join(format!(
            "wtm-index-tests-{}-{}-{}",
            std::process::id(),
            Utc::now().timestamp_nanos_opt().unwrap(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let sessions = directory.join("sessions");
        let archived = directory.join("archived_sessions");
        fs::create_dir_all(&sessions).unwrap();
        fs::create_dir_all(&archived).unwrap();
        let monitor = Monitor::with_paths(
            vec![sessions, archived],
            directory.join("session_index.jsonl"),
            limit,
            false,
        );
        Self { directory, monitor }
    }
    fn path(&self) -> PathBuf {
        self.directory.join("sessions/session.jsonl")
    }
    fn log(&self, id: &str) -> String {
        let meta = json!({"type":"session_meta","payload":{"id":"child","session_id":"parent", "cwd":self.directory.join("project/sub")}});
        let context = json!({"type":"turn_context","payload":{"turn_id":"turn","model":"logged-model","effort":"high"}});
        format!(
            "{meta}\n{context}\n{}",
            usage_line(id, "2026-09-23T10:00:00Z")
        )
    }
    fn append(&self, bytes: &[u8]) {
        fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(self.path())
            .unwrap()
            .write_all(bytes)
            .unwrap();
    }
    fn settle(&self) -> Snapshot {
        for _ in 0..5000 {
            let update = self.monitor.scan();
            if !update.status.discovering && update.status.pending_files == 0 {
                return self.monitor.snapshot();
            }
        }
        panic!("synthetic fixture never finished indexing");
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.directory);
    }
}
fn usage_line(id: &str, time: &str) -> String {
    format!(
        "{}\n",
        json!({"timestamp":time,"type":"token_usage_record","payload":{"response_id":id,"turn_id":"turn","usage":{
        "input_tokens":100,"cached_input_tokens":60,"cache_write_input_tokens":0,"output_tokens":20}}})
    )
}
fn record(id: &str, time: &str) -> CallRecord {
    Parser::default()
        .consume(
            &serde_json::from_str::<Value>(&usage_line(id, time)).unwrap(),
            0,
            "synthetic",
            &mut Diagnostics::default(),
        )
        .unwrap()
}

#[test]
fn parses_response_usage_and_parent_conversation() {
    let f = Fixture::new(100);
    fs::write(f.path(), f.log("r1")).unwrap();
    let s = f.settle();
    let c = &s.calls[0];
    assert_eq!(c.conversation_id, "parent");
    assert_eq!(c.thread_id, "child");
    assert_eq!(c.model, "logged-model");
    assert_eq!(c.effort.as_deref(), Some("high"));
    assert_eq!(c.usage.total_tokens, Some(120));
    assert_eq!(c.fresh_input_tokens, Some(40));
    assert_eq!(c.cache_hit_rate, Some(60.0));
    assert!(c.issues.is_empty());
}
#[test]
fn total_fallback_requires_both_known_fields_and_preserves_explicit_zero() {
    assert_eq!(
        parser::normalize_usage(&json!({"input_tokens":42,"output_tokens":8}))
            .0
            .total_tokens,
        Some(50)
    );
    assert_eq!(
        parser::normalize_usage(&json!({"output_tokens":8}))
            .0
            .total_tokens,
        None
    );
    assert_eq!(
        parser::normalize_usage(&json!({"input_tokens":0,"output_tokens":0,"total_tokens":0}))
            .0
            .total_tokens,
        Some(0)
    );
}
#[test]
fn empty_and_missing_roots_are_reported_without_panicking() {
    let f = Fixture::new(10);
    fs::remove_dir_all(f.directory.join("archived_sessions")).unwrap();
    let s = f.settle();
    assert!(s.calls.is_empty());
    assert_eq!(s.status.missing_roots, 1);
    assert_eq!(s.status.parse_errors, 0);
}
#[test]
fn scanning_is_read_only_and_never_exposes_bodies_or_old_feature_data() {
    let f = Fixture::new(10);
    let log = format!(
        "{}{}\n",
        f.log("r1"),
        json!({"type":"response_item","payload":{"text":"SYNTHETIC_PRIVATE_BODY"}})
    );
    fs::write(f.path(), &log).unwrap();
    for name in [
        "auth.json",
        "config.toml",
        "model-audit/capture-old.jsonl",
        "work-token-monitor/recovery.json",
    ] {
        let path = f.directory.join(name);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, "UNTOUCHED_SYNTHETIC").unwrap();
    }
    let s = f.settle();
    let serialized = serde_json::to_string(&s).unwrap();
    assert_eq!(s.calls.len(), 1);
    assert!(!serialized.contains("SYNTHETIC_PRIVATE_BODY"));
    assert!(!serialized.contains("modelAudit"));
    assert_eq!(fs::read_to_string(f.path()).unwrap(), log);
    for name in [
        "auth.json",
        "config.toml",
        "model-audit/capture-old.jsonl",
        "work-token-monitor/recovery.json",
    ] {
        assert_eq!(
            fs::read_to_string(f.directory.join(name)).unwrap(),
            "UNTOUCHED_SYNTHETIC"
        );
    }
}
#[test]
fn incremental_appends_do_not_duplicate_usage() {
    let f = Fixture::new(100);
    fs::write(f.path(), f.log("r1")).unwrap();
    f.settle();
    assert!(f.monitor.scan().calls.is_empty());
    f.append(usage_line("r2", "2026-09-23T11:00:00Z").as_bytes());
    let update = f.monitor.scan();
    assert_eq!(update.calls.len(), 1);
    assert_eq!(update.calls[0].id, "r2");
    assert_eq!(
        f.monitor
            .snapshot()
            .calls
            .iter()
            .filter_map(|c| c.usage.total_tokens)
            .sum::<u64>(),
        240
    );
}
#[test]
fn partial_utf8_jsonl_waits_for_newline() {
    let f = Fixture::new(10);
    let log = f.log("r-中文");
    let split = log.find('中').unwrap() + 1;
    f.append(&log.as_bytes()[..split]);
    assert!(f.settle().calls.is_empty());
    f.append(&log.as_bytes()[split..log.len() - 1]);
    assert!(f.settle().calls.is_empty());
    f.append(b"\n");
    let s = f.settle();
    assert_eq!(s.calls[0].id, "r-中文");
    assert_eq!(s.status.parse_errors, 0);
}
#[test]
fn malformed_lines_do_not_block_later_records() {
    let f = Fixture::new(10);
    f.append(b"broken-json\n\n  \n");
    f.append(f.log("r1").as_bytes());
    let s = f.settle();
    assert_eq!(s.status.parse_errors, 1);
    assert_eq!(s.calls.len(), 1);
    assert_eq!(f.settle().status.parse_errors, 1);
}
#[test]
fn session_and_archive_copies_deduplicate() {
    let f = Fixture::new(10);
    fs::write(f.path(), f.log("r1")).unwrap();
    fs::write(
        f.directory.join("archived_sessions/copy.jsonl"),
        f.log("r1"),
    )
    .unwrap();
    let s = f.settle();
    assert_eq!(s.status.files, 2);
    assert_eq!(s.calls.len(), 1);
}
#[test]
fn truncated_and_same_size_replaced_files_are_reindexed() {
    let f = Fixture::new(10);
    fs::write(
        f.path(),
        format!(
            "{}{}",
            f.log("r1"),
            usage_line("r2", "2026-09-23T10:00:00Z")
        ),
    )
    .unwrap();
    f.settle();
    fs::write(f.path(), f.log("r3")).unwrap();
    assert_eq!(f.settle().calls.len(), 3);
    let replacement = f.directory.join("replacement");
    fs::write(&replacement, f.log("r4")).unwrap();
    assert_eq!(
        fs::metadata(&replacement).unwrap().len(),
        fs::metadata(f.path()).unwrap().len()
    );
    fs::rename(replacement, f.path()).unwrap();
    assert_eq!(f.settle().calls.len(), 4);
}
#[test]
fn truncate_and_regrow_above_old_length_is_detected_by_checkpoint() {
    let f = Fixture::new(10);
    fs::write(f.path(), f.log("r1")).unwrap();
    f.settle();
    fs::write(
        f.path(),
        format!(
            "{}{}",
            f.log("r2"),
            usage_line("r3", "2026-09-23T11:00:00Z")
        ),
    )
    .unwrap();
    assert_eq!(f.settle().calls.len(), 3);
}
#[test]
fn latest_title_and_git_worktree_root_are_kept() {
    let f = Fixture::new(10);
    fs::create_dir_all(f.directory.join("project/sub")).unwrap();
    fs::write(f.directory.join("project/.git"), "gitdir: synthetic").unwrap();
    fs::write(f.path(), f.log("r1")).unwrap();
    fs::write(
        f.directory.join("session_index.jsonl"),
        concat!(
            "{\"id\":\"parent\",\"thread_name\":\"New\",\"updated_at\":\"2026-09-23T11:00:00Z\"}\n",
            "{\"id\":\"parent\",\"thread_name\":\"Old\",\"updated_at\":\"2026-09-23T10:00:00Z\"}\n"
        ),
    )
    .unwrap();
    let s = f.settle();
    assert_eq!(s.catalog.conversations[0].title, "New");
    assert_eq!(s.catalog.projects[0].name, "project");
    assert!(f.monitor.scan().catalog.is_none());
}

#[test]
fn interleaved_turns_keep_their_own_model_and_effort() {
    let mut p = Parser::default();
    let mut d = Diagnostics::default();
    for (turn, model, effort) in [("a", "model-a", "low"), ("b", "model-b", "high")] {
        p.consume(&json!({"type":"turn_context","payload":{"turn_id":turn,"model":model,"effort":effort}}),0,"src",&mut d);
    }
    let row = json!({"timestamp":"2026-09-23T10:00:00Z","type":"token_usage_record","payload":{"turn_id":"a","usage":{"input_tokens":10,"output_tokens":2}}});
    let c = p.consume(&row, 100, "src", &mut d).unwrap();
    assert_eq!(c.model, "model-a");
    assert_eq!(c.effort.as_deref(), Some("low"));
}
#[test]
fn cumulative_events_are_diagnosed_but_not_counted_as_calls() {
    let f = Fixture::new(10);
    f.append(b"{\"type\":\"event_msg\",\"payload\":{\"type\":\"token_count\",\"info\":{\"total_token_usage\":{\"total_tokens\":999}}}}\n");
    let s = f.settle();
    assert_eq!(s.status.legacy_records, 1);
    assert!(s.calls.is_empty());
}
#[test]
fn fallback_ids_are_offset_scoped_and_stable_without_invented_time() {
    let row = json!({"timestamp":"2026-09-23T10:00:00Z","type":"token_usage_record","payload":{"session_id":"s","usage":{"input_tokens":12}}});
    let mut p = Parser::default();
    let mut d = Diagnostics::default();
    let a = p.consume(&row, 10, "source-a", &mut d).unwrap();
    let b = p.consume(&row, 20, "source-a", &mut d).unwrap();
    let copy = p.consume(&row, 10, "source-b", &mut d).unwrap();
    assert_ne!(a.id, b.id);
    assert_eq!(a.id, copy.id);
    let invalid = json!({"type":"token_usage_record","payload":{"usage":{"input_tokens":12}}});
    assert!(p.consume(&invalid, 0, "src", &mut d).is_none());
    assert_eq!(d.invalid_records, 1);
}
#[test]
fn zero_input_and_inconsistent_cache_never_report_a_fake_rate() {
    let (u, issues) = parser::normalize_usage(
        &json!({"input_tokens":0,"cached_input_tokens":10,"output_tokens":2}),
    );
    assert_eq!(u.cached_input_tokens, None);
    assert!(issues.contains(&"cache_exceeds_input".into()));
    let mut p = Parser::default();
    let mut d = Diagnostics::default();
    let c=p.consume(&json!({"timestamp":"2026-09-23T10:00:00Z","type":"token_usage_record","payload":{"usage":{"input_tokens":0,"cached_input_tokens":0,"output_tokens":0}}}),0,"src",&mut d).unwrap();
    assert_eq!(c.cache_hit_rate, None);
}
#[test]
fn malformed_and_unsafe_integer_counters_are_null_not_rounded_or_panicking() {
    for invalid in [
        json!(-1),
        json!(1.5),
        json!("12"),
        json!(true),
        json!(u64::MAX),
        json!(9_007_199_254_740_992_u64),
    ] {
        let (u, issues) =
            parser::normalize_usage(&json!({"input_tokens":invalid,"output_tokens":1}));
        assert_eq!(u.input_tokens, None);
        assert_eq!(u.total_tokens, None);
        assert!(issues.contains(&"invalid_input_tokens".into()));
    }
}
#[test]
fn nested_usage_details_and_missing_fields_are_distinguished() {
    let (u, _) = parser::normalize_usage(
        &json!({"input_tokens":100,"input_tokens_details":{"cached_tokens":60,"cache_write_tokens":10},"output_tokens":20,"output_tokens_details":{"reasoning_tokens":5}}),
    );
    assert_eq!(u.cached_input_tokens, Some(60));
    assert_eq!(u.cache_write_input_tokens, Some(10));
    assert_eq!(u.reasoning_output_tokens, Some(5));
    let (u, issues) = parser::normalize_usage(&json!({"output_tokens":20}));
    assert_eq!(u.input_tokens, None);
    assert_eq!(u.cached_input_tokens, None);
    assert!(issues.contains(&"missing_input_tokens".into()));
}
#[test]
fn utc_normalization_orders_equivalent_offsets_consistently() {
    let a = record("a", "2026-09-23T11:00:00+08:00");
    let b = record("b", "2026-09-23T03:00:00Z");
    assert_eq!(a.timestamp, b.timestamp);
    assert_eq!(a.timestamp_ms, b.timestamp_ms);
    assert_eq!(a.timestamp, "2026-09-23T03:00:00.000Z");
}
#[test]
fn latest_window_is_independent_of_file_traversal_order() {
    for reverse in [false, true] {
        let mut store = Store::new(3);
        let mut order: Vec<_> = (0..10).collect();
        if reverse {
            order.reverse();
        }
        for n in order {
            store.insert(record(&format!("r{n}"), &format!("2026-09-23T10:00:0{n}Z")));
        }
        assert_eq!(
            store
                .snapshot()
                .iter()
                .map(|c| c.id.as_str())
                .collect::<Vec<_>>(),
            ["r9", "r8", "r7"]
        );
        assert_eq!(store.indexed_len(), 3);
        assert!(store.truncated);
        assert!(!store.insert(record("r0", "2026-09-23T10:00:00Z")).0);
        assert_eq!(store.indexed_len(), 3);
    }
}
#[test]
fn batch_evictions_and_snapshot_have_exactly_the_same_window() {
    let f = Fixture::new(2);
    fs::write(f.path(), f.log("a")).unwrap();
    let s = f.settle();
    let mut ui: HashMap<_, _> = s.calls.into_iter().map(|c| (c.id.clone(), c)).collect();
    f.append(usage_line("b", "2026-09-23T11:00:00Z").as_bytes());
    f.append(usage_line("c", "2026-09-23T12:00:00Z").as_bytes());
    let u = f.monitor.scan();
    for id in &u.removed_ids {
        ui.remove(id);
    }
    for c in &u.calls {
        ui.insert(c.id.clone(), c.clone());
    }
    assert_eq!(ui.len(), 2);
    assert!(!ui.contains_key("a"));
    assert_eq!(u.removed_ids, ["a"]);
    assert_eq!(u.revision, f.monitor.snapshot().revision);
    assert_eq!(u.status.records, ui.len());
    assert_eq!(
        u.status.oldest_at.as_deref(),
        Some("2026-09-23T11:00:00.000Z")
    );
}
#[test]
fn duplicate_replays_cannot_expand_the_dedup_index() {
    let mut store = Store::new(100);
    for _ in 0..3 {
        for n in 0..10_000 {
            store.insert(record(&format!("r{n:05}"), "2026-09-23T10:00:00Z"));
        }
    }
    assert_eq!(store.calls.len(), 100);
    assert_eq!(store.indexed_len(), 100);
    assert!(!store.contains("r00000"));
}
#[test]
fn oversized_unterminated_lines_are_bounded_and_recover_at_newline() {
    let f = Fixture::new(10);
    f.append(&vec![b'x'; reader::MAX_LINE + 100]);
    let s = f.settle();
    assert_eq!(s.status.oversized_lines, 1);
    assert!(s.calls.is_empty());
    assert!(f.monitor.inner.lock().unwrap().buffered <= BUFFER_POOL);
    f.append(b"\n");
    f.append(f.log("r1").as_bytes());
    assert_eq!(f.settle().calls.len(), 1);
}
#[test]
fn scan_byte_budget_is_enforced_and_large_files_eventually_finish() {
    let f = Fixture::new(100);
    let mut file = fs::File::create(f.path()).unwrap();
    for n in 0..25_000 {
        file.write_all(usage_line(&format!("r{n:05}"), "2026-09-23T10:00:00Z").as_bytes())
            .unwrap();
    }
    drop(file);
    let first = f.monitor.scan();
    assert!(first.status.read_bytes <= SCAN_BYTES as u64);
    let s = f.settle();
    assert_eq!(s.calls.len(), 100);
    assert!(s.status.truncated);
    assert_eq!(s.status.pending_bytes, 0);
}
#[test]
fn partial_index_lines_and_replacement_update_titles_incrementally() {
    let f = Fixture::new(10);
    fs::write(f.path(), f.log("a")).unwrap();
    let index = f.directory.join("session_index.jsonl");
    fs::write(&index, b"{\"id\":\"parent\",\"thread_name\":\"New\"").unwrap();
    assert_eq!(
        f.settle().catalog.conversations[0].title,
        "Untitled conversation"
    );
    fs::OpenOptions::new()
        .append(true)
        .open(&index)
        .unwrap()
        .write_all(b"}\n")
        .unwrap();
    assert_eq!(f.settle().catalog.conversations[0].title, "New");
    fs::write(
        &index,
        b"{\"id\":\"parent\",\"thread_name\":\"Replaced\"}\n",
    )
    .unwrap();
    assert_eq!(f.settle().catalog.conversations[0].title, "Replaced");
}
#[test]
fn catalog_only_contains_conversations_in_the_retained_window() {
    let f = Fixture::new(1);
    let mut log = String::new();
    for (id, session) in [("a", "first"), ("b", "second")] {
        log.push_str(&format!(
            "{}\n",
            json!({"type":"session_meta","payload":{"id":session}})
        ));
        log.push_str(&usage_line(id, "2026-09-23T10:00:00Z"));
    }
    fs::write(f.path(), log).unwrap();
    let s = f.settle();
    assert_eq!(s.catalog.conversations.len(), 1);
    assert_eq!(s.catalog.conversations[0].id, "second");
}
#[test]
fn symlinked_files_are_not_followed() {
    #[cfg(unix)]
    {
        let f = Fixture::new(10);
        let outside = f.directory.join("outside.jsonl");
        fs::write(&outside, f.log("a")).unwrap();
        std::os::unix::fs::symlink(&outside, f.path()).unwrap();
        assert!(f.settle().calls.is_empty());
    }
}
#[test]
fn index_errors_are_visible_without_exposing_error_text() {
    let f = Fixture::new(10);
    fs::create_dir_all(f.directory.join("session_index.jsonl")).unwrap();
    fs::write(f.path(), f.log("a")).unwrap();
    let s = f.settle();
    assert_eq!(s.status.unreadable_files, 1);
    assert_eq!(s.calls.len(), 1);
}

#[test]
#[ignore = "synthetic performance benchmark; run with --ignored --nocapture"]
fn benchmark_100k_records() {
    let f = Fixture::new(MAX_RECORDS);
    let mut file = std::io::BufWriter::new(fs::File::create(f.path()).unwrap());
    for n in 0..100_000 {
        file.write_all(usage_line(&format!("r{n:06}"), "2026-09-23T10:00:00Z").as_bytes())
            .unwrap();
    }
    file.flush().unwrap();
    drop(file);
    let bytes = fs::metadata(f.path()).unwrap().len();
    let started = Instant::now();
    let mut scans = 0;
    let mut max_ms = 0;
    loop {
        let u = f.monitor.scan();
        scans += 1;
        max_ms = max_ms.max(u.status.scan_ms);
        if u.status.initial_load_complete {
            break;
        }
        assert!(scans < 5000);
    }
    let elapsed = started.elapsed();
    let idle = Instant::now();
    for _ in 0..20 {
        f.monitor.scan();
    }
    let idle_us = idle.elapsed().as_micros() / 20;
    assert_eq!(f.monitor.snapshot().calls.len(), MAX_RECORDS);
    println!("BENCH records=100000 retained={} bytes={} scans={} indexing_ms={} max_scan_ms={} idle_us={}",MAX_RECORDS,bytes,scans,elapsed.as_millis(),max_ms,idle_us);
}

#[test]
fn native_file_notifications_discover_new_logs_without_a_full_resweep() {
    let f = Fixture::new(100);
    let monitor = Monitor::with_paths(
        vec![f.directory.join("sessions")],
        f.directory.join("session_index.jsonl"),
        100,
        true,
    );
    monitor.scan();
    if !monitor.snapshot().status.watcher_enabled {
        // Some sandboxes have no OS notification service; the periodic fallback remains covered.
        eprintln!("Native notifications unavailable; skipping the OS-specific assertion.");
        return;
    }
    fs::write(f.path(), f.log("notified")).unwrap();
    let started = Instant::now();
    loop {
        monitor.scan();
        if monitor
            .snapshot()
            .calls
            .iter()
            .any(|call| call.id == "notified")
        {
            break;
        }
        assert!(started.elapsed() < Duration::from_secs(3));
        std::thread::sleep(Duration::from_millis(20));
    }
}
