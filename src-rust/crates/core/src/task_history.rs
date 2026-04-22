// Kairos background run history.
//
// Ring-buffered (100 records in-memory) + JSONL on disk
// (`~/.claurst/kairos_run_history.jsonl`). Records come from every background
// agent run (cron, proactive, background slash commands) via
// `claurst_query::background_runner::execute_agent_run`. Consumers use
// `last_runs(n)` to surface recent activity (e.g. /kairos status, CronList).

use chrono::{DateTime, Utc};
use once_cell::sync::Lazy;
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use tokio::sync::Mutex;

const MAX_RECORDS: usize = 100;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum RunStatus {
    Success,
    Error,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskRunRecord {
    pub run_id: String,
    /// Human-readable source label: "/btw", "cron:abc123", "proactive"
    pub source_label: String,
    pub prompt_preview: String,
    pub started_at: DateTime<Utc>,
    pub completed_at: DateTime<Utc>,
    pub status: RunStatus,
    pub output_snippet: String,
}

static TASK_HISTORY: Lazy<Mutex<VecDeque<TaskRunRecord>>> =
    Lazy::new(|| Mutex::new(VecDeque::with_capacity(MAX_RECORDS)));

pub async fn record_run(record: TaskRunRecord) {
    let json = serde_json::to_string(&record);

    {
        let mut history = TASK_HISTORY.lock().await;
        if history.len() >= MAX_RECORDS {
            history.pop_front();
        }
        history.push_back(record);
    }

    match json {
        Ok(s) => append_to_disk(format!("{}\n", s)).await,
        Err(e) => tracing::warn!("Failed to serialize task run record: {}", e),
    }
}

pub async fn last_runs(n: usize) -> Vec<TaskRunRecord> {
    let history = TASK_HISTORY.lock().await;
    history.iter().rev().take(n).cloned().collect()
}

/// Most-recent run per cron task id. Scans up to `scan_limit` newest records
/// and buckets them by the `cron:{id}` source label.
pub async fn last_runs_by_cron_id(
    scan_limit: usize,
) -> std::collections::HashMap<String, TaskRunRecord> {
    let history = last_runs(scan_limit).await;
    let mut out: std::collections::HashMap<String, TaskRunRecord> =
        std::collections::HashMap::new();
    for r in history {
        if let Some(id) = r.source_label.strip_prefix("cron:").map(str::to_string) {
            out.entry(id).or_insert(r);
        }
    }
    out
}

fn history_path() -> Option<std::path::PathBuf> {
    dirs::home_dir().map(|h| h.join(".claurst").join("kairos_run_history.jsonl"))
}

async fn append_to_disk(line: String) {
    let Some(path) = history_path() else { return };
    if let Some(dir) = path.parent() {
        if let Err(e) = tokio::fs::create_dir_all(dir).await {
            tracing::warn!("Failed to create .claurst directory for task history: {}", e);
            return;
        }
    }
    use tokio::io::AsyncWriteExt;
    match tokio::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .await
    {
        Ok(mut f) => {
            if let Err(e) = f.write_all(line.as_bytes()).await {
                tracing::warn!("Failed to write task run record to disk: {}", e);
            }
        }
        Err(e) => tracing::warn!("Failed to open kairos_run_history.jsonl: {}", e),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rec(src: &str, now: DateTime<Utc>) -> TaskRunRecord {
        TaskRunRecord {
            run_id: uuid::Uuid::new_v4().to_string(),
            source_label: src.to_string(),
            prompt_preview: "p".to_string(),
            started_at: now - chrono::Duration::seconds(1),
            completed_at: now,
            status: RunStatus::Success,
            output_snippet: "o".to_string(),
        }
    }

    #[test]
    fn task_run_record_json_round_trip() {
        let r = rec("cron:abc", Utc::now());
        let json = serde_json::to_string(&r).unwrap();
        let parsed: TaskRunRecord = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.run_id, r.run_id);
        assert_eq!(parsed.source_label, r.source_label);
        assert!(matches!(parsed.status, RunStatus::Success));
    }

    #[tokio::test]
    async fn last_runs_by_cron_id_filters_and_buckets() {
        // Seed the global ring buffer with a deterministic burst.
        let base = Utc::now();
        record_run(rec("cron:alpha", base - chrono::Duration::seconds(30))).await;
        record_run(rec("cron:alpha", base - chrono::Duration::seconds(10))).await;
        record_run(rec("cron:beta", base - chrono::Duration::seconds(5))).await;
        record_run(rec("/btw", base)).await;

        let by_id = last_runs_by_cron_id(100).await;
        // "/btw" is not a cron source — must be excluded.
        assert!(!by_id.contains_key("btw"));
        // alpha is present; the most recent (smaller time gap to base) wins.
        let alpha = by_id.get("alpha").expect("alpha present");
        assert!(alpha.completed_at >= base - chrono::Duration::seconds(10));
        assert!(by_id.contains_key("beta"));
    }
}
