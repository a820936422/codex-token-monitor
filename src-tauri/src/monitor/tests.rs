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

struct Fixture {
    directory: PathBuf,
    monitor: Monitor,
}

impl Fixture {
    fn new() -> Self {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let directory = env::temp_dir().join(format!(
            "wtm-token-tests-{}-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .unwrap()
                .as_nanos(),
            NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
        ));
        let sessions = directory.join("sessions");
        let archived = directory.join("archived_sessions");
        fs::create_dir_all(&sessions).unwrap();
        fs::create_dir_all(&archived).unwrap();
        Self {
            monitor: Monitor {
                roots: vec![sessions, archived],
                index_path: directory.join("session_index.jsonl"),
                inner: Arc::new(Mutex::new(Inner::default())),
            },
            directory,
        }
    }

    fn path(&self) -> PathBuf {
        self.monitor.roots[0].join("session.jsonl")
    }

    fn log(&self, id: &str) -> String {
        let meta = serde_json::json!({"type":"session_meta","payload":{
            "id":"child", "session_id":"parent", "cwd":self.directory.join("project/sub"),
            "timestamp":"2026-09-23T10:00:00Z"
        }});
        let context = serde_json::json!({"type":"turn_context","payload":{
            "turn_id":"turn-1","model":"logged-model","effort":"high"
        }});
        format!("{meta}\n{context}\n{}", usage_line(id))
    }

    fn append(&self, bytes: &[u8]) {
        use std::io::Write;
        fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(self.path())
            .unwrap()
            .write_all(bytes)
            .unwrap();
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.directory);
    }
}

fn usage_line(id: &str) -> String {
    let row = serde_json::json!({"timestamp":"2026-09-23T10:00:00Z","type":"token_usage_record",
    "payload":{"response_id":id,"turn_id":"turn-1","usage":{
        "input_tokens":100,"cached_input_tokens":60,"output_tokens":20
    }}});
    format!("{row}\n")
}

#[test]
fn empty_and_missing_roots_are_safe() {
    let f = Fixture::new();
    fs::remove_dir_all(&f.monitor.roots[1]).unwrap();
    let scan = f.monitor.scan();
    assert!(scan.new_calls.is_empty());
    assert_eq!(scan.status.files, 0);
    assert_eq!(scan.status.parse_errors, 0);
    assert!(f.monitor.snapshot().calls.is_empty());
}

#[test]
fn scans_read_only_and_snapshot_contains_only_token_metadata() {
    let f = Fixture::new();
    let mut log = f.log("resp-1");
    log.push_str(
        "{\"type\":\"response_item\",\"payload\":{\"text\":\"SYNTHETIC_PRIVATE_BODY\"}}\n",
    );
    fs::write(f.path(), &log).unwrap();
    // These deliberately invalid synthetic files must not be read or rewritten.
    let mut untouched = vec![(f.path(), log.into_bytes())];
    for (name, contents) in [
        ("config.toml", "NOT_VALID_TOML_SYNTHETIC"),
        ("auth.json", "NOT_VALID_JSON_SYNTHETIC"),
        ("model-audit/capture-old.jsonl", "OLD_SYNTHETIC_DIAGNOSTIC"),
        ("work-token-monitor/recovery.json", "OLD_SYNTHETIC_RECOVERY"),
    ] {
        let path = f.directory.join(name);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, contents).unwrap();
        untouched.push((path, contents.as_bytes().to_vec()));
    }
    let scan = f.monitor.scan();
    assert_eq!(scan.new_calls.len(), 1);
    assert_eq!(scan.status.parse_errors, 0);
    let value = serde_json::to_value(f.monitor.snapshot()).unwrap();
    assert!(value["calls"][0].get("modelAudit").is_none());
    assert!(value["status"].get("modelAudit").is_none());
    assert_eq!(value["calls"][0]["model"], "logged-model");
    assert_eq!(value["calls"][0]["usage"]["totalTokens"], 120);
    assert!(!value.to_string().contains("SYNTHETIC_PRIVATE_BODY"));
    for (path, contents) in untouched {
        assert_eq!(fs::read(path).unwrap(), contents);
    }
}

#[test]
fn incremental_appends_emit_only_new_calls() {
    let f = Fixture::new();
    fs::write(f.path(), f.log("resp-1")).unwrap();
    assert_eq!(f.monitor.scan().new_calls.len(), 1);
    assert!(f.monitor.scan().new_calls.is_empty());
    f.append(usage_line("resp-2").as_bytes());
    let next = f.monitor.scan();
    assert_eq!(next.new_calls.len(), 1);
    assert_eq!(next.new_calls[0].id, "resp-2");
    assert_eq!(next.status.records, 2);
    assert_eq!(
        f.monitor
            .snapshot()
            .calls
            .iter()
            .map(|c| c.usage.total_tokens)
            .sum::<u64>(),
        240
    );
}

#[test]
fn partial_utf8_jsonl_waits_for_newline() {
    let f = Fixture::new();
    let text = f.log("resp-中文");
    let split = text.find('中').unwrap() + 1;
    f.append(&text.as_bytes()[..split]);
    let first = f.monitor.scan();
    assert!(first.new_calls.is_empty());
    assert_eq!(first.status.parse_errors, 0);
    f.append(&text.as_bytes()[split..text.len() - 1]);
    assert!(f.monitor.scan().new_calls.is_empty());
    f.append(b"\n");
    let next = f.monitor.scan();
    assert_eq!(next.new_calls.len(), 1);
    assert_eq!(next.new_calls[0].id, "resp-中文");
    assert_eq!(next.status.parse_errors, 0);
}

#[test]
fn malformed_lines_do_not_stop_later_records() {
    let f = Fixture::new();
    f.append(b"broken-json\n\n  \n");
    f.append(f.log("resp-1").as_bytes());
    let scan = f.monitor.scan();
    assert_eq!(scan.status.parse_errors, 1);
    assert_eq!(scan.new_calls.len(), 1);
    assert_eq!(f.monitor.scan().status.parse_errors, 1);
}

#[test]
fn duplicate_responses_in_sessions_and_archive_are_counted_once() {
    let f = Fixture::new();
    fs::write(f.path(), f.log("resp-1")).unwrap();
    fs::write(f.monitor.roots[1].join("archive.jsonl"), f.log("resp-1")).unwrap();
    let scan = f.monitor.scan();
    assert_eq!(scan.status.files, 2);
    assert_eq!(scan.status.records, 1);
    assert_eq!(scan.new_calls.len(), 1);
    assert!(f.monitor.scan().new_calls.is_empty());
}

#[test]
fn truncated_file_resets_parser_and_reads_new_record() {
    let f = Fixture::new();
    fs::write(
        f.path(),
        format!("{}{}", f.log("resp-1"), usage_line("resp-2")),
    )
    .unwrap();
    assert_eq!(f.monitor.scan().new_calls.len(), 2);
    fs::write(f.path(), f.log("resp-3")).unwrap();
    let scan = f.monitor.scan();
    assert_eq!(scan.new_calls.len(), 1);
    assert_eq!(scan.new_calls[0].id, "resp-3");
    assert_eq!(scan.new_calls[0].model, "logged-model");
}

#[test]
fn same_size_file_replacement_is_not_missed() {
    let f = Fixture::new();
    fs::write(f.path(), f.log("resp-1")).unwrap();
    assert_eq!(f.monitor.scan().new_calls.len(), 1);
    let replacement = f.directory.join("replacement.jsonl");
    fs::write(&replacement, f.log("resp-2")).unwrap();
    assert_eq!(
        fs::metadata(&replacement).unwrap().len(),
        fs::metadata(f.path()).unwrap().len()
    );
    fs::rename(replacement, f.path()).unwrap();
    let scan = f.monitor.scan();
    assert_eq!(scan.new_calls.len(), 1);
    assert_eq!(scan.new_calls[0].id, "resp-2");
}

#[test]
fn latest_title_and_git_root_are_preserved() {
    let f = Fixture::new();
    fs::create_dir_all(f.directory.join("project/sub")).unwrap();
    // Worktrees use a .git file instead of a directory.
    fs::write(f.directory.join("project/.git"), "gitdir: synthetic").unwrap();
    fs::write(f.path(), f.log("resp-1")).unwrap();
    fs::write(&f.monitor.index_path, concat!(
        "{\"id\":\"parent\",\"thread_name\":\"New title\",\"updated_at\":\"2026-09-23T11:00:00Z\"}\n",
        "{\"id\":\"parent\",\"thread_name\":\"Old title\",\"updated_at\":\"2026-09-23T10:00:00Z\"}\n"
    )).unwrap();
    let scan = f.monitor.scan();
    let catalog = scan.catalog.unwrap();
    assert_eq!(catalog.conversations.len(), 1);
    assert_eq!(catalog.conversations[0].id, "parent");
    assert_eq!(catalog.conversations[0].title, "New title");
    assert_eq!(catalog.conversations[0].project_name, "project");
    assert!(f.monitor.scan().catalog.is_none());
}

#[test]
fn interleaved_turns_keep_their_logged_model_and_effort() {
    let mut state = ParserState::default();
    for (turn, model, effort) in [("a", "model-a", "low"), ("b", "model-b", "high")] {
        process_row(
            &serde_json::json!({"type":"turn_context","payload":{
                "turn_id":turn,"model":model,"effort":effort
            }}),
            &mut state,
        );
    }
    let call = process_row(
        &serde_json::json!({"type":"token_usage_record","payload":{
            "turn_id":"a","response_id":"resp-a","usage":{"input_tokens":10}
        }}),
        &mut state,
    )
    .unwrap();
    assert_eq!(call.model, "model-a");
    assert_eq!(call.effort.as_deref(), Some("low"));
}

#[test]
fn legacy_aggregate_events_do_not_double_count_per_call_usage() {
    let mut state = ParserState::default();
    let row = serde_json::json!({"type":"event_msg","payload":{
        "type":"token_count","info":{"total_token_usage":{"total_tokens":999}}
    }});
    assert!(process_row(&row, &mut state).is_none());
}

#[test]
fn missing_response_id_uses_a_stable_fallback_for_the_same_record() {
    let mut state = ParserState::default();
    let row = serde_json::json!({"timestamp":"2026-09-23T10:00:00Z","type":"token_usage_record",
        "payload":{"session_id":"session","turn_id":"turn","usage":{"input_tokens":12}}});
    let first = process_row(&row, &mut state).unwrap();
    let second = process_row(&row, &mut state).unwrap();
    assert_eq!(first.id, second.id);
    assert_eq!(first.conversation_id, "session");
    assert!(first.response_id.is_none());
}

#[test]
fn zero_input_and_cache_write_do_not_underflow() {
    let mut state = ParserState::default();
    let call = process_row(&serde_json::json!({"type":"token_usage_record","payload":{"usage":{
        "input_tokens":0,"cached_input_tokens":10,"cache_write_input_tokens":20,"output_tokens":2
    }}}), &mut state).unwrap();
    assert_eq!(call.fresh_input_tokens, 0);
    assert_eq!(call.cache_hit_rate, 0.0);
}

#[test]
fn malformed_large_usage_cannot_panic_from_addition_overflow() {
    let usage = normalize_usage(&serde_json::json!({"input_tokens":u64::MAX,"output_tokens":1}));
    assert_eq!(usage.total_tokens, u64::MAX);
}

#[test]
fn backend_snapshot_retention_is_bounded() {
    let mut state = FileState::default();
    let mut inner = Inner::default();
    let mut emitted = Vec::new();
    for index in 0..MAX_RECORDS + 3 {
        consume_bytes(
            &mut state,
            usage_line(&format!("resp-{index}")).into_bytes(),
            &mut inner,
            &mut emitted,
        );
    }
    assert_eq!(inner.calls.len(), MAX_RECORDS);
    assert_eq!(inner.calls[0].id, "resp-3");
}
