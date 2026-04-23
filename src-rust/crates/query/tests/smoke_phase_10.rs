//! Phase 10 smoke tests — EventLog ring + JSONL flush + filtering.

use std::sync::Arc;
use std::thread;

use claurst_core::event_log::{Event, EventKind, EventLog, ToolStatus};
use claurst_core::permissions::{PermissionDecision, TaskSource};

#[test]
fn ring_evicts_oldest_at_capacity() {
    let log = EventLog::with_capacity_and_path(3, None);
    for i in 0..5 {
        log.push(Event::now(
            EventKind::Info(format!("event-{}", i)),
            TaskSource::MainSession,
            format!("msg-{}", i),
        ));
    }
    assert_eq!(log.len(), 3);
    let snap = log.snapshot();
    assert_eq!(snap[0].summary, "msg-2");
    assert_eq!(snap[1].summary, "msg-3");
    assert_eq!(snap[2].summary, "msg-4");
}

#[test]
fn most_recent_returns_last_pushed() {
    let log = EventLog::with_capacity_and_path(10, None);
    assert!(log.most_recent().is_none());

    log.push(Event::now(EventKind::TurnStart, TaskSource::MainSession, "first"));
    log.push(Event::now(EventKind::TurnEnd, TaskSource::MainSession, "second"));

    let mr = log.most_recent().unwrap();
    assert_eq!(mr.summary, "second");
    assert!(matches!(mr.kind, EventKind::TurnEnd));
}

#[test]
fn filter_by_source_isolates_subset() {
    let log = EventLog::with_capacity_and_path(50, None);
    log.push(Event::now(EventKind::TurnStart, TaskSource::MainSession, "main-1"));
    log.push(Event::now(
        EventKind::CronFired { task_id: "nightly".into() },
        TaskSource::Cron("nightly".into()),
        "cron fired",
    ));
    log.push(Event::now(EventKind::Info("p".into()), TaskSource::Proactive, "tick"));
    log.push(Event::now(EventKind::TurnEnd, TaskSource::MainSession, "main-2"));

    let main_only = log.filter_by_source(|s| matches!(s, TaskSource::MainSession));
    assert_eq!(main_only.len(), 2);

    let cron_only = log.filter_by_source(|s| matches!(s, TaskSource::Cron(_)));
    assert_eq!(cron_only.len(), 1);
    assert_eq!(cron_only[0].summary, "cron fired");
}

#[test]
fn event_kind_serde_round_trip() {
    let cases = vec![
        EventKind::TurnStart,
        EventKind::TurnEnd,
        EventKind::ToolCall {
            tool: "Bash".into(),
            status: ToolStatus::Succeeded,
        },
        EventKind::BackgroundStart,
        EventKind::BackgroundFinish { is_error: true },
        EventKind::PermissionRequested,
        EventKind::PermissionDecided(PermissionDecision::Allow),
        EventKind::CronFired { task_id: "hourly".into() },
        EventKind::AgentSpawned { agent_name: Some("docs-rag".into()) },
        EventKind::ConfigChanged {
            entity: "agent".into(),
            action: "create".into(),
            scope: "session".into(),
        },
        EventKind::TaskPanicked { msg: "boom".into() },
        EventKind::SnapshotPartialLoad { failed: vec!["a".into(), "b".into()] },
        EventKind::Error("oops".into()),
        EventKind::Info("hello".into()),
    ];
    for k in cases {
        let json = serde_json::to_string(&k).unwrap();
        let back: EventKind = serde_json::from_str(&json).unwrap();
        assert_eq!(back, k);
    }
}

#[test]
fn jsonl_flush_then_load_round_trips() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("events.jsonl");

    let log = EventLog::with_capacity_and_path(100, Some(path.clone()));
    log.push(Event::now(EventKind::TurnStart, TaskSource::MainSession, "a"));
    log.push(
        Event::now(
            EventKind::ToolCall {
                tool: "Read".into(),
                status: ToolStatus::Started,
            },
            TaskSource::MainSession,
            "read /tmp/x",
        )
        .with_details("offset=0 limit=10"),
    );
    let written = log.flush_to_jsonl().expect("flush ok");
    assert_eq!(written, 2);

    let loaded = EventLog::load_from_jsonl(&path, 100).expect("load ok");
    assert_eq!(loaded.len(), 2);
    let snap = loaded.snapshot();
    assert_eq!(snap[0].summary, "a");
    assert_eq!(snap[1].details.as_deref(), Some("offset=0 limit=10"));
}

#[test]
fn load_from_missing_jsonl_is_empty_not_error() {
    let tmp = tempfile::tempdir().unwrap();
    let missing = tmp.path().join("never_written.jsonl");
    let loaded = EventLog::load_from_jsonl(&missing, 10).expect("ok");
    assert!(loaded.is_empty());
}

#[test]
fn load_skips_malformed_lines_keeps_valid() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("mixed.jsonl");
    let valid = serde_json::to_string(&Event::now(
        EventKind::Info("ok".into()),
        TaskSource::MainSession,
        "good",
    ))
    .unwrap();
    let contents = format!("{}\n{{not valid json\n\n{}\n", valid, valid);
    std::fs::write(&path, contents).unwrap();

    let loaded = EventLog::load_from_jsonl(&path, 10).expect("load ok");
    assert_eq!(loaded.len(), 2); // two valid events, garbage skipped, blank line ignored
}

#[test]
fn flush_no_path_is_noop_returns_zero() {
    let log = EventLog::with_capacity_and_path(5, None);
    log.push(Event::now(
        EventKind::Info("x".into()),
        TaskSource::MainSession,
        "x",
    ));
    let n = log.flush_to_jsonl().unwrap();
    assert_eq!(n, 0);
}

#[test]
fn concurrent_pushes_do_not_lose_events() {
    let log = Arc::new(EventLog::with_capacity_and_path(10_000, None));
    let mut handles = Vec::new();
    for t in 0..8 {
        let log = log.clone();
        handles.push(thread::spawn(move || {
            for i in 0..200 {
                log.push(Event::now(
                    EventKind::Info(format!("t{}-{}", t, i)),
                    TaskSource::MainSession,
                    format!("t{}-{}", t, i),
                ));
            }
        }));
    }
    for h in handles {
        h.join().unwrap();
    }
    assert_eq!(log.len(), 8 * 200);
}

#[test]
fn event_with_details_helper_sets_field() {
    let e = Event::now(EventKind::TurnStart, TaskSource::MainSession, "s")
        .with_details("d");
    assert_eq!(e.details.as_deref(), Some("d"));
}
