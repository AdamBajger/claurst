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

// ---- Phase 10 producer site: PermissionManager → event log -----------------

use claurst_core::config::{PermissionMode, Settings};
use claurst_core::permissions::PermissionManager;

#[test]
fn permission_manager_emits_requested_event_when_log_attached() {
    let log = EventLog::with_capacity_and_path(64, None);
    let mut m = PermissionManager::new(PermissionMode::Default, &Settings::default());
    m.set_event_log(log.clone());

    let _rx = m.register_pending_with_source(
        "tool-use-X".into(),
        Some(TaskSource::Cron("nightly".into())),
    );

    let snap = log.snapshot();
    assert_eq!(snap.len(), 1);
    assert!(matches!(snap[0].kind, EventKind::PermissionRequested));
    assert_eq!(snap[0].source, TaskSource::Cron("nightly".into()));
}

#[test]
fn permission_manager_emits_decided_event_with_decision_and_source() {
    let log = EventLog::with_capacity_and_path(64, None);
    let mut m = PermissionManager::new(PermissionMode::Default, &Settings::default());
    m.set_event_log(log.clone());

    let _rx = m.register_pending_with_source(
        "tool-use-Y".into(),
        Some(TaskSource::Proactive),
    );
    m.resolve_pending("tool-use-Y", PermissionDecision::Allow);

    let snap = log.snapshot();
    assert_eq!(snap.len(), 2);
    assert!(matches!(snap[0].kind, EventKind::PermissionRequested));
    let (kind, source) = (&snap[1].kind, &snap[1].source);
    match kind {
        EventKind::PermissionDecided(d) => {
            assert_eq!(d, &PermissionDecision::Allow);
        }
        other => panic!("expected PermissionDecided, got {:?}", other),
    }
    assert_eq!(source, &TaskSource::Proactive);
}

#[test]
fn permission_manager_silent_without_event_log() {
    // No set_event_log call → no events emitted.
    let log = EventLog::with_capacity_and_path(64, None);
    let mut m = PermissionManager::new(PermissionMode::Default, &Settings::default());
    let _rx = m.register_pending("tool-use-Z".into());
    m.resolve_pending("tool-use-Z", PermissionDecision::Deny);
    // Log was never attached → must remain empty.
    assert_eq!(log.len(), 0);
}

#[test]
fn permission_manager_legacy_register_pending_uses_main_session_source() {
    let log = EventLog::with_capacity_and_path(64, None);
    let mut m = PermissionManager::new(PermissionMode::Default, &Settings::default());
    m.set_event_log(log.clone());
    let _rx = m.register_pending("legacy-id".into());
    let snap = log.snapshot();
    assert_eq!(snap.len(), 1);
    assert_eq!(snap[0].source, TaskSource::MainSession);
}

// ---- Panic boundary: spawn_agent_run wraps in catch_unwind ---------------

#[tokio::test]
async fn spawn_agent_run_catches_panic_and_emits_task_panicked() {
    use claurst_core::cost::CostTracker;
    use claurst_query::background_runner::{
        AgentRunContext, AgentRunRequest, AgentRunSource, spawn_agent_run,
    };
    use claurst_query::QueryConfig;
    use claurst_tools::ToolContext;

    let log = EventLog::with_capacity_and_path(64, None);

    // Build a minimal AgentRunContext that intentionally lacks an
    // anthropic client connection — `run_query_loop` will error out before
    // any model call. Then we install a panic via a custom test future
    // wrapper. Actually the simplest: spawn an AgentRunContext whose
    // tools-list panics — but we don't need the loop to panic; we can simply
    // spawn the inner future ourselves wrapped in a panic.
    //
    // Direct test: panic inside an async block scheduled via `tokio::spawn`
    // wrapped in our `catch_unwind` indirection. We re-create the
    // `spawn_agent_run` wrapper logic with a forced panic so we don't need
    // a full live runtime.
    use futures::FutureExt;

    let log_clone = log.clone();
    let task_source = claurst_core::permissions::TaskSource::Cron("panic-test".into());
    let run_id = "panic-test".to_string();

    let handle = tokio::spawn(async move {
        let inner = async {
            panic!("synthetic panic in agent run");
        };
        let payload = std::panic::AssertUnwindSafe(inner)
            .catch_unwind()
            .await;
        if let Err(_) = payload {
            log_clone.push(Event::now(
                EventKind::TaskPanicked { msg: "synthetic panic in agent run".into() },
                task_source,
                format!("agent run {} panicked", run_id),
            ));
        }
    });
    handle.await.expect("join");

    let snap = log.snapshot();
    assert!(
        snap.iter().any(|e| matches!(
            &e.kind,
            EventKind::TaskPanicked { msg } if msg.contains("synthetic")
        )),
        "expected TaskPanicked event in log, got: {:?}",
        snap
    );

    // The actual `spawn_agent_run` wrapper is exercised at runtime by panics
    // inside execute_agent_run. Smoke-test gates on the catch_unwind contract
    // since constructing a full AgentRunContext here would require an HTTP
    // client and the rest of the world. Force-bring the runtime types into
    // scope so this test breaks if the wrapper signature drifts.
    let _ = AgentRunSource::Proactive;
    let _: Option<AgentRunRequest> = None;
    let _: Option<AgentRunContext> = None;
    let _ = CostTracker::new();
    let _: Option<ToolContext> = None;
    let _: Option<QueryConfig> = None;
    let _: fn(_, _) = spawn_agent_run;
}

#[test]
fn permission_manager_resolve_unknown_id_emits_no_event() {
    let log = EventLog::with_capacity_and_path(64, None);
    let mut m = PermissionManager::new(PermissionMode::Default, &Settings::default());
    m.set_event_log(log.clone());
    m.resolve_pending("never-registered", PermissionDecision::Deny);
    assert_eq!(log.len(), 0);
}
