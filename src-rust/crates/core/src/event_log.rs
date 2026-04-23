//! Phase 10 — process-wide event log.
//!
//! Replaces the inert "recent activity" line with a live, queryable stream of
//! everything happening: turn boundaries, tool calls, background runs,
//! permission requests/decisions, cron fires, agent spawns, snapshot loads,
//! errors. Single producer trait, single consumer (TUI status line +
//! `/activity` modal), persisted as JSONL on graceful shutdown.
//!
//! Phase 10 baseline ships the types + ring + JSONL flush. Producer wiring
//! lands incrementally per phase deliverables.

use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use chrono::{DateTime, Utc};
use parking_lot::RwLock;
use serde::{Deserialize, Serialize};

use crate::permissions::{PermissionDecision, TaskSource};

/// Default ring capacity. 2000 ≈ a couple hours of moderate activity.
pub const DEFAULT_RING_CAPACITY: usize = 2000;

/// Default on-disk JSONL location: `~/.claurst/event_log.jsonl`.
pub fn default_jsonl_path() -> Option<PathBuf> {
    dirs::home_dir().map(|h| h.join(".claurst").join("event_log.jsonl"))
}

/// One observable event in the system.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Event {
    pub at: DateTime<Utc>,
    pub kind: EventKind,
    pub source: TaskSource,
    pub summary: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub details: Option<String>,
}

impl Event {
    pub fn now(kind: EventKind, source: TaskSource, summary: impl Into<String>) -> Self {
        Self {
            at: Utc::now(),
            kind,
            source,
            summary: summary.into(),
            details: None,
        }
    }

    pub fn with_details(mut self, details: impl Into<String>) -> Self {
        self.details = Some(details.into());
        self
    }
}

/// Outcome of a tool dispatch event.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ToolStatus {
    Started,
    Succeeded,
    Failed,
    Denied,
}

/// All event variants. Categorized so the TUI can pick icons and the
/// `/activity` modal can filter.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum EventKind {
    TurnStart,
    TurnEnd,
    ToolCall { tool: String, status: ToolStatus },
    BackgroundStart,
    BackgroundFinish { is_error: bool },
    PermissionRequested,
    PermissionDecided(PermissionDecision),
    CronFired { task_id: String },
    AgentSpawned { agent_name: Option<String> },
    ConfigChanged { entity: String, action: String, scope: String },
    TaskPanicked { msg: String },
    SnapshotPartialLoad { failed: Vec<String> },
    Error(String),
    Info(String),
}

/// Process-wide event log. Cheap to clone (`Arc` inside).
///
/// Writes are non-blocking: producers `push` without ever waiting on the
/// consumer. The TUI snapshots the ring once per render tick; `/activity`
/// reads against a frozen copy.
#[derive(Clone)]
pub struct EventLog {
    inner: Arc<EventLogInner>,
}

struct EventLogInner {
    buffer: RwLock<VecDeque<Event>>,
    capacity: usize,
    jsonl_path: Option<PathBuf>,
}

impl EventLog {
    /// New ring with `DEFAULT_RING_CAPACITY` and the default disk location
    /// (when the home dir is resolvable).
    pub fn new() -> Self {
        Self::with_capacity_and_path(DEFAULT_RING_CAPACITY, default_jsonl_path())
    }

    pub fn with_capacity_and_path(capacity: usize, jsonl_path: Option<PathBuf>) -> Self {
        Self {
            inner: Arc::new(EventLogInner {
                buffer: RwLock::new(VecDeque::with_capacity(capacity.min(4096))),
                capacity: capacity.max(1),
                jsonl_path,
            }),
        }
    }

    pub fn capacity(&self) -> usize {
        self.inner.capacity
    }

    pub fn jsonl_path(&self) -> Option<&Path> {
        self.inner.jsonl_path.as_deref()
    }

    /// Append an event. Evicts oldest when at capacity.
    pub fn push(&self, event: Event) {
        let mut buf = self.inner.buffer.write();
        if buf.len() == self.inner.capacity {
            buf.pop_front();
        }
        buf.push_back(event);
    }

    pub fn len(&self) -> usize {
        self.inner.buffer.read().len()
    }

    pub fn is_empty(&self) -> bool {
        self.inner.buffer.read().is_empty()
    }

    /// Snapshot of the current ring contents, oldest first.
    pub fn snapshot(&self) -> Vec<Event> {
        self.inner.buffer.read().iter().cloned().collect()
    }

    /// Most-recent event. Used by the avatar status line.
    pub fn most_recent(&self) -> Option<Event> {
        self.inner.buffer.read().back().cloned()
    }

    /// Snapshot filtered by predicate over `TaskSource`.
    pub fn filter_by_source<F>(&self, mut f: F) -> Vec<Event>
    where
        F: FnMut(&TaskSource) -> bool,
    {
        self.inner
            .buffer
            .read()
            .iter()
            .filter(|e| f(&e.source))
            .cloned()
            .collect()
    }

    /// Append the current ring contents to the configured JSONL path. Each
    /// event becomes one line. Parent dir created if missing. Atomic per
    /// line: standard append; partial writes possible only on hard crash.
    ///
    /// Returns `Ok(0)` (no-op) when no path is configured.
    pub fn flush_to_jsonl(&self) -> std::io::Result<usize> {
        let Some(path) = self.inner.jsonl_path.as_ref() else {
            return Ok(0);
        };
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        use std::io::Write;
        let snap: Vec<Event> = self.inner.buffer.read().iter().cloned().collect();
        let mut f = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)?;
        for e in &snap {
            let line = serde_json::to_string(e)
                .map_err(|err| std::io::Error::new(std::io::ErrorKind::InvalidData, err))?;
            writeln!(f, "{}", line)?;
        }
        Ok(snap.len())
    }

    /// Best-effort load from JSONL into a fresh ring. Skips malformed lines
    /// with a warn-level trace. Used at startup if continuity across
    /// restarts is desired.
    pub fn load_from_jsonl(path: &Path, capacity: usize) -> std::io::Result<Self> {
        let log = Self::with_capacity_and_path(capacity, Some(path.to_path_buf()));
        let bytes = match std::fs::read(path) {
            Ok(b) => b,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(log),
            Err(e) => return Err(e),
        };
        let s = String::from_utf8_lossy(&bytes);
        for line in s.lines() {
            if line.trim().is_empty() {
                continue;
            }
            match serde_json::from_str::<Event>(line) {
                Ok(ev) => log.push(ev),
                Err(e) => tracing::warn!(error = %e, "Skipping malformed event_log line"),
            }
        }
        Ok(log)
    }
}

impl Default for EventLog {
    fn default() -> Self {
        Self::new()
    }
}
