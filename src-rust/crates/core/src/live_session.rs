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

use crate::config::{AgentConfig, ConfigResolver, GlobalScope, McpServerConfig, OnceScope, ProjectScope, SessionScope, Settings};
use crate::cost::CostTracker;
use crate::permissions::PermissionManager;
use crate::project_registry::{ProjectConfig, ProjectRegistry};

/// Documented lock-acquisition order for the live session aggregate. When
/// multiple write-guards are needed (e.g. atomic snapshot replace, project
/// switch), acquire them in this order to avoid deadlocks. The order matches
/// the field declaration order on `LiveSession` + `RuntimeHandles`.
///
/// Per Round 2 roadmap §"Atomic Replace Protocol":
/// > Acquire all sync write locks in fixed order: settings → ephemeral →
/// > runtime.working_directory → runtime.active_project → runtime.tools →
/// > runtime.mcp → runtime.permissions.
///
/// This constant is documentation-only; Rust can't statically enforce
/// acquisition order at the `parking_lot` API level. Reviewers and `unsafe`
/// concurrent code paths must consult this list.
pub const LIVE_SESSION_LOCK_ORDER: &[&str] = &[
    "settings",
    "ephemeral",
    "runtime.working_directory",
    "runtime.active_project",
    "runtime.project_registry",
    "runtime.tools",       // attached in later phases
    "runtime.mcp",         // attached in later phases
    "runtime.permissions",
];

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
    /// Phase 8.5 partial: signals that the CLI's MCP runtime needs to
    /// reconnect (e.g. after `/project switch` so the active project's
    /// `mcp_servers` participate). The CLI loop polls this each tick and
    /// resets it after wiring `App.pending_mcp_reconnect`. Full unified
    /// `runtime.mcp` registry + atomic-replace lands later.
    pub mcp_reconnect_pending: Arc<std::sync::atomic::AtomicBool>,
    // tools / tasks / command_queue / skill_index / provider_registry /
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
                mcp_reconnect_pending: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            },
        })
    }

    /// Build a [`ConfigResolver`] from the current live session state.
    ///
    /// Cascades: once (ephemeral overrides) → session → project → global.
    /// The resolver is a snapshot; it does not track live changes after creation.
    pub fn resolver(&self) -> ConfigResolver {
        let settings = self.settings.read();
        let global = GlobalScope { config: settings.clone() };

        let project = self.runtime.active_project.read().as_ref().and_then(|name| {
            self.runtime.project_registry.read().get(name).map(|cfg| {
                ProjectScope {
                    config: crate::config::ProjectSettings {
                        allowed_tools: Vec::new(),
                        mcp_servers: cfg.mcp_servers.values().cloned().collect(),
                        custom_system_prompt: None,
                        append_system_prompt: None,
                        provider: None,
                        model: None,
                        agent: cfg.default_agent.clone(),
                        env: std::collections::HashMap::new(),
                        permission_rules: cfg.permission_rules.clone(),
                        default_agent: cfg.default_agent.clone(),
                    },
                }
            })
        });

        let session: Option<SessionScope> = None; // TODO: wire when session persistence lands

        let once = {
            let ephem = self.ephemeral.read();
            OnceScope { config: ephem.overrides.clone() }
        };

        ConfigResolver {
            global,
            project,
            session,
            once,
        }
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

    /// Switch the active project: update `active_project` slot, live cwd, and
    /// permission-manager's active-project bucket. Loads the new project's
    /// `permission_rules` from its `ProjectConfig` into the manager's
    /// in-memory bucket so `evaluate` consults them. Sets the MCP reconnect
    /// flag so the CLI loop reconnects the MCP runtime against the merged
    /// global + project specs (full unified `runtime.mcp` registry +
    /// atomic-replace protocol still pending).
    ///
    /// Returns `Err(name)` if the named project is not registered.
    pub fn switch_project(&self, name: &str) -> Result<(), String> {
        use std::sync::atomic::Ordering;
        let cfg = self.lookup_project(name).ok_or_else(|| name.to_string())?;
        
        // 1. Acquire all sync write locks in fixed order (LIVE_SESSION_LOCK_ORDER)
        let _settings_lock = self.settings.write();
        let _ephemeral_lock = self.ephemeral.write();
        let mut wd_lock = self.runtime.working_directory.write();
        let mut ap_lock = self.runtime.active_project.write();
        let _reg_lock = self.runtime.project_registry.write();
        // Note: tools and mcp are not yet in RuntimeHandles as per current code,
        // but we follow the order for permissions.
        let mut perm_lock = self.runtime.permissions.lock();

        // 2. Snapshot/Move old artifacts (Not applicable for simple project switch,
        // but we are now inside the lock window).

        // 3. Swap data fields in place
        *wd_lock = cfg.root_path.clone();
        *ap_lock = Some(cfg.name.clone());

        let rules: Vec<crate::permissions::PermissionRule> = cfg
            .permission_rules
            .iter()
            .map(|s| {
                let mut r = crate::permissions::PermissionRule::from(s);
                r.scope = crate::permissions::PermissionScope::Project {
                    name: cfg.name.clone(),
                };
                r
            })
            .collect();
        
        perm_lock.set_project_rules(&cfg.name, rules);
        perm_lock.set_active_project(Some(cfg.name.clone()));

        // 4. Release all sync locks (happens automatically as guards go out of scope)
        drop(perm_lock);
        drop(ap_lock);
        drop(wd_lock);
        drop(_ephemeral_lock);
        drop(_settings_lock);

        // 5. Outside locks: handle async cleanup/rebuild
        // Currently handled by the CLI loop polling mcp_reconnect_pending.
        self.runtime
            .mcp_reconnect_pending
            .store(true, Ordering::Release);

        tracing::info!(project = %cfg.name, root = %cfg.root_path.display(), "Active project switched via Atomic Replace Protocol");
        Ok(())
    }

    /// Clear active project; cwd stays as-is. Permission-manager's active
    /// bucket cleared so its rules stop affecting evaluation. Also flags MCP
    /// reconnect so the runtime drops project-only servers.
    pub fn clear_active_project(&self) {
        use std::sync::atomic::Ordering;
        *self.runtime.active_project.write() = None;
        self.runtime.permissions.lock().set_active_project(None);
        self.runtime
            .mcp_reconnect_pending
            .store(true, Ordering::Release);
    }

    /// Take the MCP reconnect flag (returns `true` once and resets). Called
    /// by the CLI loop each tick. After taking it, the loop forwards into
    /// `App.pending_mcp_reconnect = true` so the existing reconnect path
    /// runs at the next idle tick.
    pub fn take_mcp_reconnect_pending(&self) -> bool {
        use std::sync::atomic::Ordering;
        self.runtime
            .mcp_reconnect_pending
            .swap(false, Ordering::AcqRel)
    }

    /// Active project's MCP server specs, or empty when no project is active
    /// or the project carries no MCP entries. Used by the CLI's reconnect
    /// path to merge into the global spec list (precedence: project > global
    /// on name collision; matches roadmap §"Name collision precedence").
    pub fn active_project_mcp_specs(&self) -> Vec<crate::config::McpServerConfig> {
        let Some(name) = self.active_project_name() else {
            return Vec::new();
        };
        let Some(cfg) = self.lookup_project(&name) else {
            return Vec::new();
        };
        cfg.mcp_servers.values().cloned().collect()
    }

    pub fn active_project_name(&self) -> Option<String> {
        self.runtime.active_project.read().clone()
    }

    /// Append a `Project { name }`-scoped rule to the project's on-disk
    /// `ProjectConfig.permission_rules` list and rewrite the file. Caller is
    /// responsible for already inserting the rule into the in-memory
    /// `PermissionManager` bucket; this helper only mirrors to disk so the
    /// rule survives restart.
    ///
    /// Path: `<dir>/<project>.json`. `dir` defaults to `default_projects_dir`
    /// when `None` is passed.
    ///
    /// Returns `Err` when the project is not registered, or on I/O / JSON
    /// failure.
    pub fn persist_project_rule(
        &self,
        project: &str,
        rule: &crate::permissions::PermissionRule,
        dir: Option<&std::path::Path>,
    ) -> std::io::Result<()> {
        let resolved_dir = match dir {
            Some(d) => d.to_path_buf(),
            None => crate::project_registry::default_projects_dir().ok_or_else(|| {
                std::io::Error::new(
                    std::io::ErrorKind::NotFound,
                    "Cannot resolve ~/.claurst/projects",
                )
            })?,
        };
        let mut reg = self.runtime.project_registry.write();
        let cfg = reg.get(project).cloned().ok_or_else(|| {
            std::io::Error::new(
                std::io::ErrorKind::NotFound,
                format!("Project '{}' not registered", project),
            )
        })?;
        let mut updated = cfg.clone();
        updated
            .permission_rules
            .push(crate::permissions::SerializedPermissionRule::from(rule));
        crate::project_registry::ProjectRegistry::save_one(&resolved_dir, &updated)?;
        reg.insert(updated);
        Ok(())
    }

    /// Mirror removal of a project-scoped rule (matched by stable id) to the
    /// project's on-disk `ProjectConfig.permission_rules`. No-op when the rule
    /// isn't found in the on-disk list (e.g. the rule was added during the
    /// current session and never persisted).
    ///
    /// Returns `Err` when the project is not registered, or on I/O failure.
    pub fn persist_remove_project_rule(
        &self,
        project: &str,
        rule_id: uuid::Uuid,
        dir: Option<&std::path::Path>,
    ) -> std::io::Result<bool> {
        let resolved_dir = match dir {
            Some(d) => d.to_path_buf(),
            None => crate::project_registry::default_projects_dir().ok_or_else(|| {
                std::io::Error::new(
                    std::io::ErrorKind::NotFound,
                    "Cannot resolve ~/.claurst/projects",
                )
            })?,
        };
        let mut reg = self.runtime.project_registry.write();
        let cfg = reg.get(project).cloned().ok_or_else(|| {
            std::io::Error::new(
                std::io::ErrorKind::NotFound,
                format!("Project '{}' not registered", project),
            )
        })?;
        let mut updated = cfg.clone();
        let before = updated.permission_rules.len();
        updated
            .permission_rules
            .retain(|s| s.id != Some(rule_id));
        if updated.permission_rules.len() == before {
            return Ok(false);
        }
        crate::project_registry::ProjectRegistry::save_one(&resolved_dir, &updated)?;
        reg.insert(updated);
        Ok(true)
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
