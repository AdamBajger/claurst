//! StopAllTasks tool — Phase 11 deliverable.
//!
//! Wraps `claurst_core::task_tracker::TaskTracker::cancel_all`. Exposed to
//! agents only via explicit `tools.allowlist` (per Round 2 spec, item L676).
//! Default agent tool set must NOT include this tool — registration is the
//! caller's choice.

use crate::{PermissionLevel, Tool, ToolContext, ToolResult};
use async_trait::async_trait;
use claurst_core::task_tracker::TaskTracker;
use serde_json::{json, Value};
use tracing::info;

pub struct StopAllTasksTool {
    tracker: TaskTracker,
}

impl StopAllTasksTool {
    pub fn new(tracker: TaskTracker) -> Self {
        Self { tracker }
    }
}

#[async_trait]
impl Tool for StopAllTasksTool {
    fn name(&self) -> &str {
        "StopAllTasks"
    }

    fn description(&self) -> &str {
        "Cancel every tracked background task — agent runs, cron ticks, sub-agents, \
         background loops. Returns the count cancelled. Cancellation is cooperative: \
         tasks observe a CancellationToken and stop at their next yield point."
    }

    fn permission_level(&self) -> PermissionLevel {
        // Treated as Dangerous because it terminates ALL background work.
        // Combined with the spec rule (allowlist-only) this means the user has
        // both granted the tool to the agent AND approved this specific
        // permission decision.
        PermissionLevel::Dangerous
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {},
            "additionalProperties": false
        })
    }

    async fn execute(&self, _input: Value, _ctx: &ToolContext) -> ToolResult {
        let n = self.tracker.cancel_all();
        info!(cancelled = n, "StopAllTasks invoked");
        ToolResult::success(format!("Cancelled {} task(s).", n))
            .with_metadata(json!({ "cancelled": n }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use claurst_core::permissions::TaskSource;
    use claurst_core::task_tracker::{SimpleTrackedTask, TaskKind};
    use tokio_util::sync::CancellationToken;

    #[test]
    fn name_and_permission_level() {
        let tool = StopAllTasksTool::new(TaskTracker::new());
        assert_eq!(tool.name(), "StopAllTasks");
        assert_eq!(tool.permission_level(), PermissionLevel::Dangerous);
    }

    #[test]
    fn empty_tracker_returns_zero() {
        let tracker = TaskTracker::new();
        let tool = StopAllTasksTool::new(tracker);
        let n_before = tool.tracker.list_active().len();
        assert_eq!(n_before, 0);
        let n = tool.tracker.cancel_all();
        assert_eq!(n, 0);
    }

    #[test]
    fn cancels_all_registered_tasks() {
        let tracker = TaskTracker::new();
        let tok1 = CancellationToken::new();
        let tok2 = CancellationToken::new();
        let t1 = SimpleTrackedTask::new(
            "t1",
            TaskKind::Agent,
            TaskSource::Agent("a1".into()),
            "first",
            tok1.clone(),
        );
        let t2 = SimpleTrackedTask::new(
            "t2",
            TaskKind::Cron,
            TaskSource::Cron("c1".into()),
            "second",
            tok2.clone(),
        );
        tracker.register(t1);
        tracker.register(t2);
        assert_eq!(tracker.list_active().len(), 2);

        let tool = StopAllTasksTool::new(tracker.clone());
        let n = tool.tracker.cancel_all();
        assert_eq!(n, 2);
        assert!(tok1.is_cancelled());
        assert!(tok2.is_cancelled());
    }
}
