//! Live in-memory session layer.
//!
//! `Settings` is the persistent root. `LiveSession` wraps it with an ephemeral
//! overlay (per-session model overrides, scratch agents, ad-hoc MCP specs) plus
//! `RuntimeHandles` (Arc-shared registries, cwd, active project).
//!
//! Round 2 scaffold: only core-owned handles live here. Cross-crate handles
//! (tools, MCP, command queue, skill index, provider/model registries, managed
//! agents) attach in task #17 when spawn sites are migrated.

use std::collections::{BTreeMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;

use parking_lot::{Mutex, RwLock};
use serde::{Deserialize, Serialize};

use crate::config::{AgentConfig, McpServerConfig, Settings};
use crate::cost::CostTracker;
use crate::permissions::PermissionManager;
use crate::project_registry::{ProjectConfig, ProjectRegistry};

/// Top-level live session aggregate. Cloned shallowly via `SharedLiveSession`.
pub struct LiveSession {
    pub settings: Arc<RwLock<Settings>>,
    pub ephemeral: Arc<RwLock<EphemeralState>>,
    pub runtime: RuntimeHandles,
}

pub type SharedLiveSession = Arc<LiveSession>;

/// Per-session ephemeral overlay. Every field MUST stay (de)serializable —
/// preserves the future named-session-snapshot path. See KAIROS_FUTURE.md.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct EphemeralState {
    /// Ad-hoc MCP server specs added this session (not persisted unless promoted).
    #[serde(default)]
    pub mcp_specs: BTreeMap<String, McpServerConfig>,

    /// Scratch named agents (not promoted to `Settings.agents` yet).
    #[serde(default)]
    pub agents: BTreeMap<String, AgentConfig>,

    /// Per-session tool allowlist override. `None` = inherit from settings.
    #[serde(default)]
    pub tool_allowlist: Option<HashSet<String>>,

    /// Per-session tool denylist (added on top of settings denylist).
    #[serde(default)]
    pub tool_denylist: HashSet<String>,

    /// Lightweight overrides applied at spawn-time merge.
    #[serde(default)]
    pub overrides: EphemeralOverrides,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct EphemeralOverrides {
    pub model: Option<String>,
    pub provider: Option<String>,
    pub output_style: Option<String>,
    /// Effort level as raw string; parsed at apply-time.
    pub effort: Option<String>,
}

/// Process-wide runtime artifacts. Rebuilt from `(Settings, EphemeralState)`
/// on snapshot load — never serialized.
#[derive(Clone)]
pub struct RuntimeHandles {
    pub working_directory: Arc<RwLock<PathBuf>>,
    pub active_project: Arc<RwLock<Option<String>>>,
    pub project_registry: Arc<RwLock<ProjectRegistry>>,
    pub cost_tracker: Arc<CostTracker>,
    pub permissions: Arc<Mutex<PermissionManager>>,
    // tools / mcp / tasks / command_queue / skill_index / provider_registry /
    // model_registry / managed_agents — attached in later phases.
}

impl LiveSession {
    pub fn new(
        settings: Settings,
        working_directory: PathBuf,
        cost_tracker: Arc<CostTracker>,
        permissions: PermissionManager,
    ) -> SharedLiveSession {
        Self::with_projects(
            settings,
            working_directory,
            cost_tracker,
            permissions,
            ProjectRegistry::new(),
        )
    }

    pub fn with_projects(
        settings: Settings,
        working_directory: PathBuf,
        cost_tracker: Arc<CostTracker>,
        permissions: PermissionManager,
        project_registry: ProjectRegistry,
    ) -> SharedLiveSession {
        Arc::new(Self {
            settings: Arc::new(RwLock::new(settings)),
            ephemeral: Arc::new(RwLock::new(EphemeralState::default())),
            runtime: RuntimeHandles {
                working_directory: Arc::new(RwLock::new(working_directory)),
                active_project: Arc::new(RwLock::new(None)),
                project_registry: Arc::new(RwLock::new(project_registry)),
                cost_tracker,
                permissions: Arc::new(Mutex::new(permissions)),
            },
        })
    }

    /// Look up a project config by name. Returns a clone to avoid holding the
    /// registry lock across the caller's work.
    pub fn lookup_project(&self, name: &str) -> Option<ProjectConfig> {
        self.runtime.project_registry.read().get(name).cloned()
    }

    /// Resolve effective working directory at spawn time.
    /// Order: explicit override > project root (when present and known) > live cwd.
    /// A named project that doesn't exist in the registry falls through to live
    /// cwd with a warn-level trace (no hard error).
    pub fn resolve_cwd(&self, explicit: Option<&std::path::Path>, project: Option<&str>) -> PathBuf {
        if let Some(p) = explicit {
            return p.to_path_buf();
        }
        if let Some(name) = project {
            if let Some(cfg) = self.lookup_project(name) {
                return cfg.root_path;
            }
            tracing::warn!(project = %name, "Project name not found in registry; falling back to live cwd");
        }
        self.runtime.working_directory.read().clone()
    }

    /// Switch the active project: update `active_project` slot and live cwd.
    /// Phase 8.5 baseline: updates cwd + active_project marker. Permission rule
    /// + MCP atomic swap arrives in Phase 9 (PermissionScope::Project) and the
    /// MCP unified registry wiring (task #24+).
    ///
    /// Returns `Err(name)` if the named project is not registered.
    pub fn switch_project(&self, name: &str) -> Result<(), String> {
        let cfg = self.lookup_project(name).ok_or_else(|| name.to_string())?;
        *self.runtime.working_directory.write() = cfg.root_path.clone();
        *self.runtime.active_project.write() = Some(cfg.name.clone());
        tracing::info!(project = %cfg.name, root = %cfg.root_path.display(), "Active project switched");
        Ok(())
    }

    /// Clear active project; cwd stays as-is.
    pub fn clear_active_project(&self) {
        *self.runtime.active_project.write() = None;
    }

    pub fn active_project_name(&self) -> Option<String> {
        self.runtime.active_project.read().clone()
    }

    /// Resolve effective `AgentConfig` at spawn time.
    ///
    /// Lookup order: `EphemeralState.agents[name]` first (shadow override),
    /// then `Settings.agents[name]`, else default. Ephemeral spawn-time
    /// overrides (`overrides.model` etc.) layered on top.
    pub fn resolve_agent_config(&self, agent_name: Option<&str>) -> AgentConfig {
        let settings = self.settings.read();
        let ephemeral = self.ephemeral.read();

        let mut cfg = if let Some(name) = agent_name {
            ephemeral
                .agents
                .get(name)
                .cloned()
                .or_else(|| settings.agents.get(name).cloned())
                .unwrap_or_default()
        } else {
            AgentConfig::default()
        };

        if let Some(model) = ephemeral.overrides.model.as_ref() {
            cfg.model = Some(model.clone());
        }

        cfg
    }

    // ----------------------------------------------------------------------
    // Phase 11 — named agent management
    // ----------------------------------------------------------------------

    /// Union of agent names across `Settings.agents` and
    /// `EphemeralState.agents`. Sorted, deduplicated.
    pub fn agent_names(&self) -> Vec<String> {
        let mut names: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
        for k in self.settings.read().agents.keys() {
            names.insert(k.clone());
        }
        for k in self.ephemeral.read().agents.keys() {
            names.insert(k.clone());
        }
        names.into_iter().collect()
    }

    pub fn agent_exists(&self, name: &str) -> bool {
        self.ephemeral.read().agents.contains_key(name)
            || self.settings.read().agents.contains_key(name)
    }

    /// Insert a scratch agent into `EphemeralState.agents`. Replaces any
    /// existing ephemeral entry with the same name. Does not touch settings.
    pub fn put_ephemeral_agent(&self, name: impl Into<String>, cfg: AgentConfig) {
        let name = name.into();
        self.ephemeral.write().agents.insert(name, cfg);
    }

    /// Remove an agent. Looks in ephemeral first, then settings (so shadowed
    /// settings agents survive ephemeral deletion). Returns the deleted
    /// config when found.
    pub fn delete_agent(&self, name: &str) -> Option<AgentConfig> {
        if let Some(cfg) = self.ephemeral.write().agents.remove(name) {
            return Some(cfg);
        }
        self.settings.write().agents.remove(name)
    }

    /// Promote an ephemeral agent into `Settings.agents`. Idempotent: a
    /// second call after promotion returns `Ok(false)` (nothing to promote).
    /// Does not auto-persist settings to disk — caller decides.
    pub fn promote_ephemeral_agent(&self, name: &str) -> Result<bool, String> {
        let cfg = self
            .ephemeral
            .write()
            .agents
            .remove(name)
            .ok_or_else(|| format!("no ephemeral agent named '{}'", name))?;
        self.settings.write().agents.insert(name.to_string(), cfg);
        Ok(true)
    }
}
