// session_bridge: bridge pointer files for Kairos session continuity.
//
// When Kairos is active, a pointer file is written after each completed query
// turn. On next startup in the same working directory, the pointer is discovered
// and the session is auto-resumed.
//
// Each session gets one file: ~/.claurst/bridge/{session_id}.json
// Pointers older than KAIROS_BRIDGE_TTL_SECS (default 4h) are stale and ignored.

use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use tracing::{debug, warn};

const DEFAULT_TTL_SECS: i64 = 4 * 3600;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BridgePointer {
    pub session_id: String,
    pub working_dir: PathBuf,
    pub started_at: DateTime<Utc>,
    pub last_active_at: DateTime<Utc>,
}

impl BridgePointer {
    fn is_stale(&self) -> bool {
        let ttl = Duration::seconds(bridge_ttl_secs());
        Utc::now() - self.last_active_at > ttl
    }

    fn matches_dir(&self, dir: &Path) -> bool {
        self.working_dir == dir
    }
}

fn bridge_ttl_secs() -> i64 {
    std::env::var("KAIROS_BRIDGE_TTL_SECS")
        .ok()
        .and_then(|s| s.parse::<i64>().ok())
        .unwrap_or(DEFAULT_TTL_SECS)
        .max(60)
}

fn bridge_dir() -> Option<PathBuf> {
    dirs::home_dir().map(|h| h.join(".claurst").join("bridge"))
}

fn pointer_path(session_id: &str) -> Option<PathBuf> {
    bridge_dir().map(|d| d.join(format!("{}.json", session_id)))
}

/// Write or overwrite the bridge pointer for this session.
/// Called after each completed query turn (debounced by the caller).
pub async fn upsert_bridge_pointer(
    session_id: &str,
    working_dir: &Path,
    started_at: DateTime<Utc>,
) {
    let pointer = BridgePointer {
        session_id: session_id.to_string(),
        working_dir: working_dir.to_path_buf(),
        started_at,
        last_active_at: Utc::now(),
    };

    let json = match serde_json::to_string_pretty(&pointer) {
        Ok(s) => s,
        Err(e) => {
            warn!("Failed to serialize bridge pointer: {}", e);
            return;
        }
    };

    let Some(path) = pointer_path(session_id) else { return };
    if let Some(dir) = path.parent() {
        if let Err(e) = tokio::fs::create_dir_all(dir).await {
            warn!("Failed to create bridge directory: {}", e);
            return;
        }
    }

    if let Err(e) = tokio::fs::write(&path, json).await {
        warn!("Failed to write bridge pointer {}: {}", path.display(), e);
    }
}

/// Find the most recently active bridge pointer for the given working directory.
/// Returns None if no non-stale pointer matches.
pub async fn find_active_pointer(working_dir: &Path) -> Option<BridgePointer> {
    let dir = bridge_dir()?;

    let mut entries = match tokio::fs::read_dir(&dir).await {
        Ok(e) => e,
        Err(_) => return None, // bridge dir doesn't exist yet — normal on first run
    };

    let mut best: Option<BridgePointer> = None;

    while let Ok(Some(entry)) = entries.next_entry().await {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }

        let data = match tokio::fs::read_to_string(&path).await {
            Ok(d) => d,
            Err(e) => {
                debug!("Skipping unreadable bridge pointer {}: {}", path.display(), e);
                continue;
            }
        };

        let pointer: BridgePointer = match serde_json::from_str(&data) {
            Ok(p) => p,
            Err(e) => {
                debug!("Skipping malformed bridge pointer {}: {}", path.display(), e);
                continue;
            }
        };

        if pointer.is_stale() || !pointer.matches_dir(working_dir) {
            continue;
        }

        let is_newer = best
            .as_ref()
            .map(|b| pointer.last_active_at > b.last_active_at)
            .unwrap_or(true);

        if is_newer {
            best = Some(pointer);
        }
    }

    best
}

/// Delete all stale bridge pointer files. Returns the number deleted.
pub async fn cleanup_stale_pointers() -> usize {
    let Some(dir) = bridge_dir() else { return 0 };

    let mut entries = match tokio::fs::read_dir(&dir).await {
        Ok(e) => e,
        Err(_) => return 0,
    };

    let mut deleted = 0usize;

    while let Ok(Some(entry)) = entries.next_entry().await {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }

        let data = match tokio::fs::read_to_string(&path).await {
            Ok(d) => d,
            Err(_) => continue,
        };

        let pointer: BridgePointer = match serde_json::from_str(&data) {
            Ok(p) => p,
            Err(_) => {
                if let Err(e) = tokio::fs::remove_file(&path).await {
                    warn!("Failed to remove malformed bridge pointer {}: {}", path.display(), e);
                } else {
                    debug!("Removed malformed bridge pointer {}", path.display());
                    deleted += 1;
                }
                continue;
            }
        };

        if pointer.is_stale() {
            if let Err(e) = tokio::fs::remove_file(&path).await {
                warn!("Failed to remove stale bridge pointer {}: {}", path.display(), e);
            } else {
                debug!(session_id = %pointer.session_id, "Removed stale bridge pointer");
                deleted += 1;
            }
        }
    }

    deleted
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pointer(last_active: DateTime<Utc>, dir: &str) -> BridgePointer {
        BridgePointer {
            session_id: "sess-1".to_string(),
            working_dir: PathBuf::from(dir),
            started_at: last_active - Duration::minutes(5),
            last_active_at: last_active,
        }
    }

    #[test]
    fn bridge_pointer_round_trip_json() {
        let p = pointer(Utc::now(), "/tmp/work");
        let json = serde_json::to_string(&p).expect("serialize");
        let parsed: BridgePointer = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(parsed.session_id, p.session_id);
        assert_eq!(parsed.working_dir, p.working_dir);
        assert_eq!(parsed.last_active_at, p.last_active_at);
    }

    #[test]
    fn is_stale_true_past_ttl() {
        // Far past: 1 day ago with default 4h TTL ⇒ stale.
        let p = pointer(Utc::now() - Duration::days(1), "/tmp/x");
        assert!(p.is_stale());
    }

    #[test]
    fn is_stale_false_recent() {
        let p = pointer(Utc::now() - Duration::seconds(30), "/tmp/x");
        assert!(!p.is_stale());
    }

    #[test]
    fn bridge_ttl_respects_env_minimum() {
        // Setting TTL below 60s should clamp to 60. Test helper directly;
        // env mutation is process-global, so each assertion sets and checks
        // back-to-back to minimise interleaving with other env-reading tests.
        std::env::set_var("KAIROS_BRIDGE_TTL_SECS", "5");
        assert_eq!(bridge_ttl_secs(), 60);
        std::env::set_var("KAIROS_BRIDGE_TTL_SECS", "7200");
        assert_eq!(bridge_ttl_secs(), 7200);
        std::env::remove_var("KAIROS_BRIDGE_TTL_SECS");
        assert_eq!(bridge_ttl_secs(), 4 * 3600);
    }

    #[test]
    fn matches_dir_exact() {
        let p = pointer(Utc::now(), "/tmp/work");
        assert!(p.matches_dir(Path::new("/tmp/work")));
        assert!(!p.matches_dir(Path::new("/tmp/other")));
    }
}
