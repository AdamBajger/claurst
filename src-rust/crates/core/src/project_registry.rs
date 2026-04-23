//! Project registry — first-class project concept.
//!
//! `ProjectConfig` is the new Round 2 type, distinct from the legacy
//! `Settings.projects: HashMap<String, ProjectSettings>` (which stays as
//! visited-directory metadata; rename to `LegacyProjectVisitMetadata` deferred).
//!
//! Disk shape: `~/.claurst/projects/<name>.json` — one file per project. Loaded
//! at startup into `ProjectRegistry`. Phase 8.5 ships the type + load/save +
//! lookup; `LiveSession.runtime.active_project` and `/project switch` wire in
//! the next task.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::config::McpServerConfig;
use crate::permissions::SerializedPermissionRule;

/// First-class project description. Phase 8.5 baseline: minimal but extensible.
///
/// `permission_rules` uses the existing `SerializedPermissionRule` for now;
/// Phase 9 extends scope variants and validates `scope == Project { name }` on
/// load (Architecture Note 2 in the roadmap).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectConfig {
    pub name: String,
    pub root_path: PathBuf,
    #[serde(default)]
    pub permission_rules: Vec<SerializedPermissionRule>,
    /// Default agent name to spawn when no `--agent` flag is provided.
    /// Resolves against `Settings.agents` (or `EphemeralState.agents`).
    #[serde(default)]
    pub default_agent: Option<String>,
    /// Per-project MCP server specs. Merged into the unified runtime registry
    /// at `/project switch`; precedence is session > project > global.
    #[serde(default)]
    pub mcp_servers: BTreeMap<String, McpServerConfig>,
}

impl ProjectConfig {
    pub fn new(name: impl Into<String>, root_path: impl Into<PathBuf>) -> Self {
        Self {
            name: name.into(),
            root_path: root_path.into(),
            permission_rules: Vec::new(),
            default_agent: None,
            mcp_servers: BTreeMap::new(),
        }
    }
}

/// In-memory registry of known projects. Authoritative copy is on disk.
#[derive(Debug, Clone, Default)]
pub struct ProjectRegistry {
    projects: BTreeMap<String, ProjectConfig>,
}

impl ProjectRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn get(&self, name: &str) -> Option<&ProjectConfig> {
        self.projects.get(name)
    }

    pub fn insert(&mut self, cfg: ProjectConfig) {
        self.projects.insert(cfg.name.clone(), cfg);
    }

    pub fn remove(&mut self, name: &str) -> Option<ProjectConfig> {
        self.projects.remove(name)
    }

    pub fn names(&self) -> impl Iterator<Item = &str> {
        self.projects.keys().map(String::as_str)
    }

    pub fn iter(&self) -> impl Iterator<Item = (&str, &ProjectConfig)> {
        self.projects.iter().map(|(k, v)| (k.as_str(), v))
    }

    pub fn len(&self) -> usize {
        self.projects.len()
    }

    pub fn is_empty(&self) -> bool {
        self.projects.is_empty()
    }

    /// Load every `<name>.json` file from `dir` into a fresh registry.
    /// Missing directory is treated as "empty registry" — not an error. Files
    /// that fail to parse are skipped with a warning; the rest still load.
    pub fn load_from_dir(dir: &Path) -> std::io::Result<Self> {
        let mut reg = Self::new();
        let entries = match std::fs::read_dir(dir) {
            Ok(it) => it,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(reg),
            Err(e) => return Err(e),
        };

        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|s| s.to_str()) != Some("json") {
                continue;
            }
            let bytes = match std::fs::read(&path) {
                Ok(b) => b,
                Err(e) => {
                    tracing::warn!(path = %path.display(), error = %e, "Skipping project file: read failed");
                    continue;
                }
            };
            match serde_json::from_slice::<ProjectConfig>(&bytes) {
                Ok(cfg) => {
                    reg.projects.insert(cfg.name.clone(), cfg);
                }
                Err(e) => {
                    tracing::warn!(path = %path.display(), error = %e, "Skipping project file: parse failed");
                }
            }
        }

        Ok(reg)
    }

    /// Persist a single project to `<dir>/<name>.json`. Parent dir created if
    /// missing. Atomic via tmpfile + rename to avoid torn writes.
    pub fn save_one(dir: &Path, cfg: &ProjectConfig) -> std::io::Result<()> {
        std::fs::create_dir_all(dir)?;
        let target = dir.join(format!("{}.json", cfg.name));
        let tmp = dir.join(format!(".{}.json.tmp", cfg.name));
        let json = serde_json::to_vec_pretty(cfg)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
        std::fs::write(&tmp, json)?;
        std::fs::rename(&tmp, &target)?;
        Ok(())
    }

    /// Delete `<dir>/<name>.json`. Returns `Ok(false)` if the file was absent.
    pub fn delete_one(dir: &Path, name: &str) -> std::io::Result<bool> {
        let target = dir.join(format!("{}.json", name));
        match std::fs::remove_file(&target) {
            Ok(()) => Ok(true),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
            Err(e) => Err(e),
        }
    }
}

/// Default disk location for project configs: `~/.claurst/projects`.
/// Returns `None` if the home dir cannot be resolved.
pub fn default_projects_dir() -> Option<PathBuf> {
    dirs::home_dir().map(|h| h.join(".claurst").join("projects"))
}
