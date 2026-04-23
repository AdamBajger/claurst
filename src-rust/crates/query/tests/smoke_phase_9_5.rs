//! Phase 9.5 smoke tests — TaskTracker + SimpleTrackedTask + AgentRunSource→TaskSource.

use std::sync::Arc;

use claurst_core::permissions::TaskSource;
use claurst_core::task_tracker::{
    SimpleTrackedTask, TaskKind, TaskStatus, TaskTracker, TrackedTask,
};
use claurst_query::background_runner::AgentRunSource;
use tokio_util::sync::CancellationToken;

fn make(id: &str) -> Arc<SimpleTrackedTask> {
    SimpleTrackedTask::new(
        id,
        TaskKind::Agent,
        TaskSource::MainSession,
        format!("agent {}", id),
        CancellationToken::new(),
    )
}

#[test]
fn tracker_register_list_deregister() {
    let t = TaskTracker::new();
    assert!(t.is_empty());

    let a = make("a");
    let b = make("b");
    t.register(a.clone());
    t.register(b.clone());

    assert_eq!(t.len(), 2);
    let listed = t.list_active();
    assert_eq!(listed.len(), 2);

    let removed = t.deregister("a").expect("a present");
    assert_eq!(removed.id(), "a");
    assert_eq!(t.len(), 1);
    assert!(t.get("b").is_some());
    assert!(t.get("a").is_none());
}

#[test]
fn tracker_get_returns_none_for_unknown() {
    let t = TaskTracker::new();
    assert!(t.get("ghost").is_none());
}

#[test]
fn cancel_propagates_via_token_and_marks_status_via_producer() {
    let t = TaskTracker::new();
    let task = make("a");
    let token = task.cancel_token().clone();
    t.register(task.clone());

    assert!(!token.is_cancelled());
    let returned = t.cancel("a").expect("found");
    assert_eq!(returned.id(), "a");
    assert!(token.is_cancelled());

    // Status flip happens at producer site (mirrors execute_agent_run); simulate it.
    task.set_status(TaskStatus::Cancelled);
    assert_eq!(task.status(), TaskStatus::Cancelled);
    assert!(task.status().is_terminal());
}

#[test]
fn cancel_unknown_id_returns_none() {
    let t = TaskTracker::new();
    assert!(t.cancel("ghost").is_none());
}

#[test]
fn cancel_all_signals_every_task() {
    let t = TaskTracker::new();
    let a = make("a");
    let b = make("b");
    let c = make("c");
    let tokens: Vec<_> = [&a, &b, &c].iter().map(|x| x.cancel_token().clone()).collect();
    t.register(a);
    t.register(b);
    t.register(c);

    let n = t.cancel_all();
    assert_eq!(n, 3);
    for tok in &tokens {
        assert!(tok.is_cancelled());
    }
    // cancel_all does NOT auto-deregister — producer sites do that on terminal status.
    assert_eq!(t.len(), 3);
}

#[test]
fn task_status_terminal_classification() {
    assert!(TaskStatus::Completed.is_terminal());
    assert!(TaskStatus::Failed { error: "x".into() }.is_terminal());
    assert!(TaskStatus::Cancelled.is_terminal());
    assert!(!TaskStatus::Running.is_terminal());
    assert!(!TaskStatus::Waiting { reason: "mcp".into() }.is_terminal());
}

#[test]
fn simple_tracked_task_set_status_and_summary() {
    let task = make("a");
    assert_eq!(task.status(), TaskStatus::Running);
    assert_eq!(task.summary(), "agent a");

    task.set_status(TaskStatus::Waiting { reason: "permission".into() });
    assert_eq!(task.status(), TaskStatus::Waiting { reason: "permission".into() });

    task.set_summary("agent a (running git push)");
    task.set_details("running command: git push origin main");
    assert_eq!(task.summary(), "agent a (running git push)");
    assert_eq!(task.details(), "running command: git push origin main");
}

#[test]
fn agent_run_source_maps_to_task_source() {
    assert_eq!(
        AgentRunSource::Cron { task_id: "abc".into() }.as_task_source(),
        TaskSource::Cron("abc".into())
    );
    assert_eq!(
        AgentRunSource::SlashCommand { name: "btw".into() }.as_task_source(),
        TaskSource::SlashCommand("btw".into())
    );
    assert_eq!(
        AgentRunSource::Proactive.as_task_source(),
        TaskSource::Proactive
    );
}

#[test]
fn tracker_clone_shares_inner_state() {
    let t1 = TaskTracker::new();
    let t2 = t1.clone();
    t1.register(make("x"));
    assert_eq!(t2.len(), 1);
    t2.deregister("x");
    assert!(t1.is_empty());
}
