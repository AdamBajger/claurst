//! Phase 9.5 — uniform background-task tracking.
//!
//! Single trait (`TrackedTask`) implemented by every long-running unit:
//! agent runs, tool calls, cron ticks, sub-agents, background loops. Single
//! registry (`TaskTracker`) holds them keyed by stable id.
//!
//! Producer wiring lands incrementally per task #33+. This module ships the
//! types + registry + cancellation contract.

use std::collections::HashMap;
use std::sync::Arc;

use chrono::{DateTime, Utc};
use parking_lot::RwLock;
use tokio_util::sync::CancellationToken;

use crate::permissions::TaskSource;

/// Coarse classification of a tracked unit. Drives `/tasks` grouping and icon
/// selection in the event log.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum TaskKind {
    /// Single tool call.
    Tool,
    /// Top-level agent run (foreground turn or `execute_agent_run`).
    Agent,
    /// One cron-fired execution.
    Cron,
    /// Sub-agent invocation via `AgentTool`.
    Subagent,
    /// Background loop (`/btw`, etc.).
    BgLoop,
}

/// Lifecycle state of a tracked task. `Waiting` carries an optional reason
/// surfaced in `/tasks` (e.g. "permission prompt", "MCP settle").
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum TaskStatus {
    Running,
    Waiting { reason: String },
    Completed,
    Failed { error: String },
    Cancelled,
}

impl TaskStatus {
    pub fn is_terminal(&self) -> bool {
        matches!(self, Self::Completed | Self::Failed { .. } | Self::Cancelled)
    }
}

/// Implemented by every kind of background work.
///
/// The registry holds `Arc<dyn TrackedTask>`. Implementations own their own
/// state (typically behind interior mutability) so `status()` and `cancel()`
/// can be called concurrently with the running work.
pub trait TrackedTask: Send + Sync {
    fn id(&self) -> &str;
    fn kind(&self) -> TaskKind;
    fn source(&self) -> &TaskSource;
    fn started_at(&self) -> DateTime<Utc>;
    fn status(&self) -> TaskStatus;
    /// One-line label suitable for a list view.
    fn summary(&self) -> String;
    /// Multi-line block for `/tasks show <id>`.
    fn details(&self) -> String;
    /// Request graceful cancellation. Implementations typically signal a
    /// `CancellationToken`; the actual stop happens at the next await point.
    fn cancel(&self) -> Result<(), String>;
}

/// Process-wide registry of in-flight tracked tasks.
///
/// Cheap to clone (`Arc` inside). Lives on `RuntimeHandles.tasks`.
#[derive(Clone, Default)]
pub struct TaskTracker {
    tasks: Arc<RwLock<HashMap<String, Arc<dyn TrackedTask>>>>,
}

impl TaskTracker {
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a freshly-spawned task. Replaces any prior entry with the same
    /// id (caller's responsibility to avoid id collisions).
    pub fn register(&self, task: Arc<dyn TrackedTask>) {
        self.tasks.write().insert(task.id().to_string(), task);
    }

    /// Remove an entry. Idempotent. Call after a task reaches a terminal
    /// state.
    pub fn deregister(&self, id: &str) -> Option<Arc<dyn TrackedTask>> {
        self.tasks.write().remove(id)
    }

    /// Snapshot of currently-tracked tasks. Returned vec is independent of the
    /// registry — safe to iterate without holding the lock.
    pub fn list_active(&self) -> Vec<Arc<dyn TrackedTask>> {
        self.tasks.read().values().cloned().collect()
    }

    /// Look up a task by id.
    pub fn get(&self, id: &str) -> Option<Arc<dyn TrackedTask>> {
        self.tasks.read().get(id).cloned()
    }

    pub fn len(&self) -> usize {
        self.tasks.read().len()
    }

    pub fn is_empty(&self) -> bool {
        self.tasks.read().is_empty()
    }

    /// Cancel one task by id. Returns the task if found (so caller can await
    /// terminal state). Does **not** deregister — the producer site does that
    /// on terminal status.
    pub fn cancel(&self, id: &str) -> Option<Arc<dyn TrackedTask>> {
        let task = self.tasks.read().get(id).cloned()?;
        let _ = task.cancel();
        Some(task)
    }

    /// Cancel every tracked task. Returns the count cancelled. Used by
    /// `/stop all`.
    pub fn cancel_all(&self) -> usize {
        let snapshot: Vec<_> = self.tasks.read().values().cloned().collect();
        let n = snapshot.len();
        for t in &snapshot {
            let _ = t.cancel();
        }
        n
    }
}

// ---------------------------------------------------------------------------
// Helper: simple TrackedTask backed by atomic state + CancellationToken.
// Producer sites can use this directly or implement TrackedTask themselves.
// ---------------------------------------------------------------------------

/// Reference implementation of `TrackedTask` driven by interior mutability.
///
/// Producers update `status` via `set_status` as work progresses; consumers
/// read it without blocking the producer.
pub struct SimpleTrackedTask {
    id: String,
    kind: TaskKind,
    source: TaskSource,
    started_at: DateTime<Utc>,
    summary: parking_lot::RwLock<String>,
    details: parking_lot::RwLock<String>,
    status: parking_lot::RwLock<TaskStatus>,
    cancel_token: CancellationToken,
}

impl SimpleTrackedTask {
    pub fn new(
        id: impl Into<String>,
        kind: TaskKind,
        source: TaskSource,
        summary: impl Into<String>,
        cancel_token: CancellationToken,
    ) -> Arc<Self> {
        Arc::new(Self {
            id: id.into(),
            kind,
            source,
            started_at: Utc::now(),
            summary: parking_lot::RwLock::new(summary.into()),
            details: parking_lot::RwLock::new(String::new()),
            status: parking_lot::RwLock::new(TaskStatus::Running),
            cancel_token,
        })
    }

    pub fn set_status(&self, status: TaskStatus) {
        *self.status.write() = status;
    }

    pub fn set_summary(&self, s: impl Into<String>) {
        *self.summary.write() = s.into();
    }

    pub fn set_details(&self, s: impl Into<String>) {
        *self.details.write() = s.into();
    }

    pub fn cancel_token(&self) -> &CancellationToken {
        &self.cancel_token
    }
}

impl TrackedTask for SimpleTrackedTask {
    fn id(&self) -> &str {
        &self.id
    }
    fn kind(&self) -> TaskKind {
        self.kind
    }
    fn source(&self) -> &TaskSource {
        &self.source
    }
    fn started_at(&self) -> DateTime<Utc> {
        self.started_at
    }
    fn status(&self) -> TaskStatus {
        self.status.read().clone()
    }
    fn summary(&self) -> String {
        self.summary.read().clone()
    }
    fn details(&self) -> String {
        self.details.read().clone()
    }
    fn cancel(&self) -> Result<(), String> {
        self.cancel_token.cancel();
        Ok(())
    }
}
