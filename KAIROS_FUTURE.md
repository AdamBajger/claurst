# Kairos Mode — Round 2 Roadmap

Round 1 (gate, runner, cron, proactive ticker, session bridge, task history) is implemented. See [KAIROS_CURRENT.md](KAIROS_CURRENT.md) for the current architecture and module map.

Round 2 transforms Kairos plumbing into a reusable agent-platform foundation: hierarchical configs, projects, scope-aware permissions, background task tracking, observable event log, and named agents.

---

## Design Principles

1. **One config per entity, cloned on use, logged at spawn.** No parallel "snapshot" types. Traceability via spawn-time logging.
2. **Hierarchical configs, flat per-entity.** `AgentConfig`, `ProjectConfig`, `ProviderConfig`, `ToolConfig`, `MCPConfig`. Composition by containment.
3. **Permissions explicit everywhere.** Background, foreground, sub-agent, cron — same prompt queue. Silent denial is a bug.
4. **Status visible by default.** Every spawn / tool call / permission prompt / cron fire emits an event next to the avatar; full log via slash command.
5. **Extend in place.** Existing `core::permissions` already has the surface we need; Round 2 extends it. PR #110 (`feat/permission-manager-tui`) is ignored — manual merge on top later.
6. **Live session layer.** `Settings` is the persistent root. `LiveSession` (in-memory) wraps it with ephemeral overlay + runtime handles. Spawns resolve effective config at *fire time*.
7. **Every running unit is tracked.** Background agent runs, sub-agents, cron ticks, in-flight tool calls — all implement `TrackedTask`. Listable, inspectable, cancellable via `/tasks` and `/stop all`.

---

## Target Architecture

```text
+------------------------------------------------------------------+
|                       Claurst Agent Runtime                      |
+------------------------------------------------------------------+
| Persistence (~/.claurst/)                                        |
|   settings.json        — Settings (config, agents, providers,    |
|                          global permission_rules, ...)           |
|   projects/<name>.json — ProjectConfig per project               |
|   skills/, ...         — existing                                |
+------------------------------------------------------------------+
| Live layer (core::live_session)                                  |
|   LiveSession {                                                  |
|     settings:  Arc<RwLock<Settings>>,                            |
|     ephemeral: Arc<RwLock<EphemeralState>>,                      |
|     runtime:   RuntimeHandles {                                  |
|       working_directory, active_project,                         |
|       tools, mcp, permissions, cost_tracker, tasks               |
|     }                                                            |
|   }                                                              |
+------------------------------------------------------------------+
| Spawn layer (query::background_runner + query::agent_tool)       |
|   resolve_agent_config(agent_name) -> AgentConfig (frozen clone) |
|   execute_agent_run(request, ctx) registers TrackedTask          |
+------------------------------------------------------------------+
| Permission layer (core::permissions, extended in place)          |
|   PermissionManager: scope-aware (global/project/session/once)   |
|   PendingPermission queue (oneshot-backed)                       |
+------------------------------------------------------------------+
| Observability (tui::event_log)                                   |
|   EventLog ring buffer + JSONL flush on shutdown                 |
|   Avatar-line current event; /activity full view                 |
+------------------------------------------------------------------+
```

---

## Prerequisites

- **None.** Existing `core::permissions` already ships `PermissionManager`, `PermissionRule`, `PermissionScope` (Session/Persistent), `PermissionDecision` (5-variant), `PermissionRequest`, `PendingPermission` (oneshot-backed queue), `PermissionHandler` trait + handlers, and the `Settings.permission_rules` persistence. Round 2 **extends in place**.
- PR #110 (`feat/permission-manager-tui`) is treated as out-of-scope for Round 2. If/when it merges upstream, do a manual merge on top — most of the surface it adds is already present in this repo.

---

## Type Specs (Final)

### AgentConfig

```rust
// claurst-core::config::agent_config
pub struct AgentConfig {
    pub name: Option<String>,                      // None = anonymous (ambient)

    // Model / provider — flattened (matches existing core::Config layout;
    // provider selection still routes through Settings.provider_configs).
    pub model: Option<String>,
    pub max_tokens: u32,
    pub temperature: Option<f64>,
    pub fallback_model: Option<String>,

    // Tools / MCP — per-agent restriction; resolves against runtime registries.
    pub tools: ToolConfig,
    pub mcp: MCPConfig,

    // Permission seeding (see DefaultPermissionRule below).
    pub permission_defaults: Vec<DefaultPermissionRule>,
    pub permission_mode: PermissionMode,
    pub kairos_policy: KairosPermissionPolicy,

    // Prompts / behavior.
    pub system_prompt: Option<String>,
    pub append_system_prompt: Option<String>,
    pub output_style: OutputStyle,
    pub effort: Option<EffortLevel>,
    pub max_turns: Option<u32>,
    pub thinking_budget: Option<u32>,
    pub tool_result_budget: usize,

    // Scope.
    pub project: Option<String>,                   // project-name reference
    pub kairos_addendum: bool,

    // UI-only metadata (preserves legacy AgentDefinition fields).
    pub description: Option<String>,
    pub visible: bool,
    pub color: Option<String>,
    // NO working_directory — see Working Directory Resolution below.
    // NO Arc-backed registries — those live on RuntimeHandles.
}

// Defaults are seed-only: no scope (scope = Session of spawned manager),
// no created_at. Disambiguates from active rules.
pub struct DefaultPermissionRule {
    pub subject: PermissionSubject,
    pub decision: PermissionDecision,
}
```

`AgentConfig` is the single config type for all spawn sites. **Replaces** the existing `AgentDefinition` via in-place rename.

**Migration strategy (decided — A1/B2):**

```rust
// crates/core/src/lib.rs (or extracted module)
pub struct AgentConfig { /* fields above */ }

/// Legacy alias — keeps callers compiling during Phase 8 plumbing.
/// Delete in a later phase once all sites import AgentConfig directly.
pub type AgentDefinition = AgentConfig;
```

The existing 8 fields (`description`, `model`, `temperature`, `prompt`, `access`, `visible`, `max_turns`, `color`) are kept on `AgentConfig` (renaming `prompt` → `append_system_prompt` and dropping `access` in favor of `tools.allowlist` preset bundles). Serde-compat for old `settings.json` via `#[serde(alias = "prompt")]` on `append_system_prompt` and a custom deserializer for `access` that expands to `tools.allowlist`.

No parallel `AgentDefinitionLegacy` type. No `From` conversion. The struct itself moves forward; the type alias buys grace period for callers.

### ToolConfig, MCPConfig

```rust
pub struct ToolConfig {
    pub allowlist: Option<Vec<String>>,            // None = all enabled
    pub denylist: Vec<String>,
    pub per_tool_overrides: BTreeMap<String, Value>,
}

pub struct MCPConfig {
    pub enabled_servers: Vec<String>,              // explicit per-agent subset
}
```

Provider routing reuses the existing `core::ProviderConfig` (in `Settings.provider_configs`); not duplicated on `AgentConfig`.

### ProjectConfig + ProjectRegistry

`ProjectConfig` is a **new** first-class type (decision C2). It lives alongside the existing `Settings.projects: HashMap<String, ProjectSettings>` — the legacy struct stays as visited-directory metadata (allowed_tools / mcp_servers / custom_system_prompt per cwd). Future cleanup: rename `ProjectSettings` → `LegacyProjectVisitMetadata` once all call sites migrated.

```rust
// claurst-core::project_registry
pub struct ProjectConfig {
    pub name: String,
    pub root_path: PathBuf,
    pub permission_rules: Vec<PermissionRule>,     // scope = Project (validated on load)
    pub default_agent: Option<String>,             // points into Settings.agents
    pub mcp_servers: BTreeMap<String, McpServerConfig>,
    // Minimal for Round 2; expandable.
}

pub struct ProjectRegistry {
    projects: BTreeMap<String, ProjectConfig>,     // keyed by name
}
// Disk: ~/.claurst/projects/<name>.json
```

### Permissions

**Strategy (decision A1): extend `core::permissions` in place.** No vendoring, no parallel module. The existing types (`PermissionManager`, `PermissionRule`, `PermissionScope`, `PermissionDecision`, `PermissionRequest`, `PendingPermission`, `PermissionHandler`) stay; we add scopes + subject matching + source attribution.

```rust
// claurst-core::permissions  (existing module — extended)

pub enum PermissionScope {
    Once,                                          // NEW — never persisted, never listed
    Session,                                       // existing
    Project { name: String },                      // NEW — persisted in ProjectConfig
    #[serde(alias = "Persistent")]                 // existing was "Persistent"
    Forever,                                       // renamed; serde alias preserves old JSON
}

pub enum PermissionSubject {
    Tool { name: String },
    ToolInput { name: String, input_match: InputMatcher },
    Path { path: PathBuf, mode: PathMode },        // Read / Write / Any
    Url { pattern: UrlPattern },
    Command { shell: Shell, pattern: CommandPattern },
    Composite(Vec<PermissionSubject>),
}

pub struct PermissionRule {
    pub id: Uuid,                                  // NEW — stable identifier for /permissions revoke
    // Existing fields kept for back-compat:
    pub tool_name: Option<String>,                 // legacy match (kept; None when subject set)
    pub path_pattern: Option<String>,              // legacy match (glob::Pattern)
    pub action: PermissionAction,                  // existing Allow/Deny
    pub scope: PermissionScope,                    // existing
    // Round 2 extension:
    pub subject: Option<PermissionSubject>,        // NEW — when Some, supersedes tool_name+path_pattern
    pub decision: PermissionDecision,              // NEW — richer than action (carries reason)
    pub created_at: DateTime<Utc>,                 // NEW
}

// User is the only rule author for now (decision A1: no created_by).
// `TaskSource` is still defined and used ONLY on PendingPermissionRequest.source
// to attribute "who is asking *now*" in the dialog.
pub enum TaskSource {
    MainSession,
    SlashCommand(String),                          // inline slash commands
    Cron(String),                                  // task id
    Proactive,
    Agent(String),                                 // agent name
    BgLoop(String),                                // /btw etc.
    System,
}
```

**`evaluate()` extension:** the existing 6-step ordering (bypass → deny rules → allow rules → AcceptEdits → Plan → default) stays. New `subject`-based matching slots into the rule-match step: when `rule.subject.is_some()`, dispatch through `PermissionSubject::matches(&request)`; otherwise fall back to existing `tool_name`/`path_pattern` glob check.

### Static Config vs Runtime Handles — explicit split

The existing `QueryConfig` mixes two unrelated concerns: static per-spawn data (model, prompts, effort) AND Arc-backed shared registries (command queue, skill index, provider registry, model registry, managed agents). Round 2 splits them:

| Concern | Lives on | Lifetime |
|---|---|---|
| Static spawn data (model, prompts, effort, max_turns, project, …) | `AgentConfig` | Cloned per spawn; frozen for that spawn |
| Shared command queue (TUI ↔ loop bridge) | `RuntimeHandles.command_queue` | Process-wide |
| Skill index | `RuntimeHandles.skill_index` | Process-wide |
| Provider registry | `RuntimeHandles.provider_registry` | Process-wide |
| Model registry | `RuntimeHandles.model_registry` | Process-wide |
| Managed agents (multi-agent orchestration) | `RuntimeHandles.managed_agents` | Process-wide; mutable through `LiveSession` |
| Tool instances | `RuntimeHandles.tools` | Process-wide |
| MCP handles | `RuntimeHandles.mcp` | Per active scope |
| Cost tracker | `RuntimeHandles.cost_tracker` | Process-wide |
| Permission manager | `RuntimeHandles.permissions` | Per process; rules per scope |
| Task tracker | `RuntimeHandles.tasks` | Process-wide |
| Working directory, active project | `RuntimeHandles.working_directory` / `active_project` | Process-wide; mutable |

`AgentRunContext` (passed to `execute_agent_run`) bundles `AgentConfig` + `Arc<LiveSession>` + per-call channels (`result_tx`, `cancel`). The runtime side is reached through `live_session.runtime.*`; nothing on `AgentConfig` requires `Arc`.

This is the canonical interpretation of Principle 6. `QueryConfig` is deleted in Phase 8.

### MCP Server Resolution

MCP architecture (clarified):

- **Configs live at scope** — never duplicated:
  - Global → `Settings.config.mcp_servers`
  - Project → `ProjectConfig.mcp_servers`
  - Session (ephemeral) → `EphemeralState.mcp_specs`
- **`RuntimeHandles.mcp` is the unified live registry** — `HashMap<String, McpServerHandle>` keyed by server name. Single source of truth for connected servers.
- **`AgentConfig.mcp.enabled_servers: Vec<String>` references by name only** — never carries config.

**Population order at scope load:** global → active project → ephemeral additions. Each step populates / replaces entries in `RuntimeHandles.mcp`.

**Name collision precedence:** session > project > global. A session-added `docs-rag` shadows a project's `docs-rag`; if session removed, project's re-emerges (re-resolved on `/project switch` or session save/load).

**Agent's effective MCP set at spawn:** intersection of `AgentConfig.mcp.enabled_servers` with currently-connected names in `RuntimeHandles.mcp`. If `enabled_servers` is empty → all available.

### LiveSession + EphemeralState + RuntimeHandles

```rust
// claurst-core::live_session
pub struct LiveSession {
    pub settings:  Arc<RwLock<Settings>>,
    pub ephemeral: Arc<RwLock<EphemeralState>>,
    pub runtime:   RuntimeHandles,
}
pub type SharedLiveSession = Arc<LiveSession>;

#[derive(Serialize, Deserialize)]
// Every field MUST stay (de)serializable — preserves Future Extension session-snapshot path.
pub struct EphemeralState {
    pub mcp_specs: BTreeMap<String, McpServerConfig>,
    pub skills: BTreeMap<String, SkillDefinition>,
    pub agents: BTreeMap<String, AgentConfig>,
    pub tool_allowlist: Option<HashSet<String>>,
    pub tool_denylist: HashSet<String>,
    pub overrides: EphemeralOverrides,
}

pub struct EphemeralOverrides {
    pub model: Option<String>,
    pub effort: Option<EffortLevel>,
    pub output_style: Option<OutputStyle>,
    pub provider: Option<String>,
}

pub struct RuntimeHandles {
    pub working_directory: Arc<RwLock<PathBuf>>,
    pub active_project:    Arc<RwLock<Option<String>>>,
    pub tools:             Arc<RwLock<Vec<Arc<dyn Tool>>>>,    // Arc<dyn>, not Box
    pub mcp:               Arc<RwLock<HashMap<String, McpServerHandle>>>,
    pub permissions:       Arc<Mutex<PermissionManager>>,        // core::permissions, extended in place
    pub cost_tracker:      Arc<CostTracker>,
    pub tasks:             Arc<TaskTracker>,
    // Migrated out of QueryConfig — see "Static Config vs Runtime Handles" above.
    pub command_queue:     Option<CommandQueue>,
    pub skill_index:       Option<SharedSkillIndex>,
    pub provider_registry: Option<Arc<ProviderRegistry>>,
    pub model_registry:    Option<Arc<ModelRegistry>>,
    pub managed_agents:    Option<Arc<RwLock<ManagedAgentConfig>>>,
}
```

### Working Directory Resolution

`AgentConfig` has **no `working_directory` field**. Resolution at spawn:

```rust
impl LiveSession {
    pub fn resolve_cwd(
        &self,
        explicit: Option<&Path>,        // caller override
        project: Option<&str>,          // AgentConfig.project
    ) -> PathBuf {
        if let Some(p) = explicit { return p.to_path_buf(); }
        if let Some(name) = project {
            if let Some(cfg) = self.lookup_project(name) {
                return cfg.root_path.clone();
            }
            // Project named but missing on disk: warn via event log, fall through.
        }
        self.runtime.working_directory.read().clone()
    }
}
```

Order: explicit override > project root > live session cwd.

### Resolution Rule (spawn-time)

```rust
impl LiveSession {
    pub fn resolve_agent_config(&self, agent_name: Option<&str>) -> AgentConfig {
        let settings  = self.settings.read();
        let ephemeral = self.ephemeral.read();
        let mut cfg = AgentConfig::from_settings(&settings);
        cfg.apply_ephemeral(&ephemeral);
        if let Some(name) = agent_name {
            if let Some(def) = settings.agents.get(name)
                .or_else(|| ephemeral.agents.get(name))
            {
                cfg.apply_agent(def);
            }
        }
        // Caller applies its own overrides outside.
        cfg
    }
}
```

Merge order: persistent → ephemeral → named-agent → caller-override. Same for every spawn.

---

## Phase 8 — Hierarchical Config Refactor

**Goal:** `AgentConfig` plumbed through every spawn site. `AgentDefinition` survives as a type alias only. `QueryConfig` deleted at end of phase.

**Deliverables:**
1. `claurst-core::config::agent_config` module with all structs above. `pub type AgentDefinition = AgentConfig;` for caller back-compat.
2. `Settings.agents: HashMap<String, AgentConfig>` (the existing field, type swapped). Serde migration via `#[serde(alias = "prompt")]` on `append_system_prompt` and a custom `access` deserializer that expands to `tools.allowlist` preset bundles.
3. `AgentRunContext.agent_config: AgentConfig` (replaces `query_config`).
4. All spawn sites (`/btw`, cron scheduler, proactive ticker, `AgentTool::execute`) call `LiveSession::resolve_agent_config(name)` then optionally mutate the clone before passing to `execute_agent_run`.
5. Delete `apply_kairos_bootstrap_to_query_config`. Kairos addendum becomes a field set by `from_settings` when `kairos_gate::is_kairos_brief_active()`.
6. Delete `resolve_subagent_model`. Replaced by `AgentConfig.provider` with optional override.
7. `tracing::info!(config = ?cfg, "Spawning agent run")` at every spawn site.

**Smoke tests** (`crates/query/tests/smoke_phase_8.rs`):
- `/btw` after `/model X` uses model X.
- Round-trip serde of `AgentConfig` (and migration from old `AgentDefinition` JSON).
- `live_session.resolve_agent_config(Some("foo"))` returns merged config from settings + ephemeral.

---

## Phase 8.5 — Project Registry

**Goal:** introduce explicit project concept. WD and permission rules scoped by project, not raw paths.

**Deliverables:**
1. `ProjectConfig` + `ProjectRegistry` types.
2. Disk layout `~/.claurst/projects/<name>.json`. Loaded at startup into `ProjectRegistry`.
3. Migration: existing `Settings.projects: HashMap<String, ProjectSettings>` extended into `ProjectConfig` with `serde(default)` on new fields. `permission_rules` field starts empty for migrated entries.
4. `LiveSession.runtime.active_project: Arc<RwLock<Option<String>>>`.
5. `LiveSession::resolve_cwd(explicit, project)` helper.
6. Slash commands:
   - `/project list` — known projects + active marker.
   - `/project switch <name>` — atomic swap: cwd, permission rules, MCP set (uses Atomic Replace Protocol).
   - `/project create <name> --root <path>` — register.
   - `/project show` — print active.
7. On `/project switch`: `PermissionManager` drops old project's rules, loads new project's. MCP servers swapped via the atomic-replace protocol.

**`/project switch` mid-task semantics:** in-flight tasks **freeze their permission rule set at spawn time**. They keep finishing under their original ruleset. Only new spawns see the new project. Same for MCP — in-flight tasks hold their `Arc<McpServerHandle>`; old handles drop when last task releases.

**Smoke tests:** `/project switch` swaps rules + MCP; cron task scheduled with `project = "foo"` resolves cwd from foo; in-flight task keeps old rules after switch.

---

## Phase 9 — Permission System Extension

**Goal:** scope-aware first-prompt approval with TUI management.

### Storage by scope

| Scope | In-memory home | Persistent home |
|---|---|---|
| `Once` | transient (allow/deny resolution only) | — never |
| `Session` | `PermissionManager` | — (lost on exit unless session saved) |
| `Project` | `PermissionManager` (active project only) | `~/.claurst/projects/<name>.json` |
| `Forever` | `PermissionManager` | `~/.claurst/settings.json` |

### `PermissionManager` lifecycle

- **Startup:** load global (`Forever`) rules from `Settings.permission_rules`.
- **Project switch:** drop old project rules, load new project rules.
- **Session switch / resume:** drop old session rules, load new (when sessions saveable).
- **Process exit:** in-memory state lost; only persisted scopes survive.
- **Once rules:** never enter the manager; resolved inline by tool dispatch and dropped after.

### Rule seeding at agent spawn

- **Default:** `AgentConfig.permission_defaults` is converted to `Vec<PermissionRule>` with `scope = Session` and seeded into the spawned task's manager view.
- **Opt-in inheritance:** caller sets `inherit_live_session_rules: bool` on `AgentRunRequest`. If true, live session's current rules also seeded; live wins on conflict.

### Conflict precedence

When multiple rules match: **most-specific subject wins; on equal specificity, Deny > Allow.** Specificity ordered: Composite > ToolInput > Tool > Path/Url/Command > broader. Documented with examples in unit tests.

### First-prompt flow

1. Tool call → `ToolContext::check_permission(req)` → `PermissionManager::evaluate(&req)`.
2. No rule match → return `Ask`. Push to `PendingPermissionStore`.
3. TUI dialog renders: tool name, subject summary, `request.source` attribution, scope buttons (Once/Session/Project/Forever × Allow/Deny).
4. Decision → if scope ≠ Once, store `PermissionRule`; persist if Project/Forever. (User is the only rule author for Round 2 — no `created_by` field.)
5. Send decision on oneshot.

### Kairos policy

```rust
pub enum KairosPermissionPolicy {
    DeferToUser,    // default: queue prompt for cron/proactive
    AutoAllowRead,  // read-only auto-allow; writes prompt
    Reject,         // backend tools refuse; prompt never shown
}
```

Env: `KAIROS_PERMISSION_POLICY=defer|read|reject` (default `defer`). Per-agent override via `AgentConfig.kairos_policy`.

### Source attribution

`TaskSource` enum lives on `PendingPermissionRequest.source` only — answers "who is asking *now*" so the TUI dialog can display attribution (e.g. `Cron(<id>)`, `Proactive`, `Agent("docs-rag")`). Rule authorship is implicitly the user (Round 2 decision A1: only the user authors rules), so no `created_by` field is stored. Future extension can add it back if/when programmatic rule creation is allowed.

### TUI management

- `/permissions` — list active rules grouped by scope. Once-rules excluded.
- Row actions: Revoke, Change scope, Show subject details.
- `/permissions grant <subject> <scope>` for power users.

### Timeout, throttle, drain

- `KAIROS_PERMISSION_TIMEOUT_SECS` (default 300) → auto-deny + event log entry.
- Per-source max 1 pending request; second from same `Cron(id)` waits.
- Shutdown: drain pending with deny reason `"session ending"`.

### Smoke tests
- Rule evaluation precedence (Deny > Allow on equal specificity).
- Persistence round-trip for Session/Project/Forever.
- Background `FileWrite` with no rule → dialog shows `Cron(<id>)` source.
- Timeout → auto-deny + event entry.
- Shutdown with pending → no hang.

---

## Phase 9.5 — Background Task Tracking

**Goal:** uniform tracking of every running unit. Single trait, single registry.

```rust
pub trait TrackedTask: Send + Sync {
    fn id(&self) -> &str;
    fn kind(&self) -> TaskKind;             // Tool / Agent / Cron / Subagent / BgLoop
    fn source(&self) -> &TaskSource;
    fn started_at(&self) -> DateTime<Utc>;
    fn status(&self) -> TaskStatus;         // Running / Waiting(reason) / Completed / Failed / Cancelled
    fn summary(&self) -> String;
    fn details(&self) -> String;
    fn cancel(&self) -> Result<()>;         // graceful via CancellationToken
}

pub struct TaskTracker {
    tasks: Arc<RwLock<HashMap<String, Arc<dyn TrackedTask>>>>,
}
```

**Producer sites:**
- `execute_agent_run` registers `AgentRunTask`.
- Tool dispatch registers `ToolCallTask` per call.
- Cron tick wraps fire in `CronTickTask`.
- `AgentTool` registers `SubagentTask`.

**Slash commands:**
- `/tasks` — active list (id, kind, source, age, status, summary).
- `/tasks show <id>` — full details.
- `/tasks cancel <id>` — graceful cancel.
- `/stop all` — cancel every tracked task (including main turn's in-flight tool call). Confirmation prompt unless `--yes`.

**`StopAllTasks` tool:** exposed to agents but **NOT in default tool set**. Must be explicitly enabled per-agent via `AgentConfig.tools.allowlist`. Reasoning: too powerful to grant by default; cron/proactive should not nuke main session work.

**Lifecycle:** register on spawn, deregister on completion/failure/cancel. Tracker entries surface in `/activity` event log as start/end events (single producer; tracker is observable).

**Smoke tests:**
- `/tasks` shows running cron + tool call.
- `/tasks cancel <id>` propagates cancel.
- `/stop all` clears tracker (with timeout fallback).

---

## Phase 10 — Event Log + Status Line

**Goal:** replace inert "recent activity" line with live event log. Persisted across restarts via JSONL flush on graceful shutdown.

```rust
pub struct EventLog {
    buffer: VecDeque<Event>,                    // ring, cap 2000
    jsonl_path: PathBuf,                        // ~/.claurst/event_log.jsonl
}

pub struct Event {
    pub at: DateTime<Utc>,
    pub kind: EventKind,
    pub source: TaskSource,
    pub summary: String,
    pub details: Option<String>,
}

pub enum EventKind {
    TurnStart, TurnEnd,
    ToolCall { tool: String, status: ToolStatus },
    BackgroundStart, BackgroundFinish { is_error: bool },
    PermissionRequested, PermissionDecided(PermissionDecision),
    CronFired { task_id: String },
    AgentSpawned { agent_name: Option<String> },
    ConfigChanged { entity: String, action: String, scope: String },
    TaskPanicked { msg: String },
    SnapshotPartialLoad { failed: Vec<String> },
    Error(String),
    Info(String),
}
```

**Producer sites:** TUI turn boundary, tool dispatch, `execute_agent_run` start/finish, `PermissionManager::evaluate`, cron tick, `AgentTool::execute`, `TaskTracker` lifecycle, snapshot loader, panic boundary.

**TUI integration:**
- Avatar-line shows `event_log.most_recent()` summary; icon per `EventKind`; fade after 2s.
- `/activity` opens scrollable modal: filter by source (all/main/cron/agent/proactive), j/k scroll, `f` filter, `d` expand, `Esc` close.
- Reader snapshots ring into render state once per tick (no per-frame lock contention).

**Persistence:** JSONL append on graceful shutdown only; ring stays in-memory during run. TUI panic handler flushes log before exiting.

**Smoke tests:** event ordering under concurrent producers, ring eviction, filter correctness, `/activity` open/close non-disruptive, JSONL written on graceful shutdown.

---

## Phase 11 — Named Agents

**Goal:** named agents loadable, invokable, registered as cron defaults.

**Implementation:**
- Lookup order: `Settings.agents[name]` first, then `EphemeralState.agents[name]` (ephemeral wins for shadow override).
- Slash commands:
  - `/agent list`
  - `/agent create <name>` — wizard from current `LiveSession::resolve_agent_config(None)`.
  - `/agent <name> <prompt>` — spawn background run with that agent.
  - `/agent delete <name>`.
  - `/agent persist <name>` — promote ephemeral agent to `Settings.agents`.
- `CronTask` gains `agent_name: Option<String>`. Scheduler resolves at fire time via `live_session.resolve_agent_config(task.agent_name.as_deref())`.
- `AgentTool` input schema gains optional `agent_name`.

**Project namespace:** agents stay **global** in `Settings.agents`. Per-project agents pushed to deferred (see Nice-to-Have).

**Cron rule inheritance:** cron tasks freeze their config (including default rules from chosen agent) at schedule time. They do **not** capture caller's live session rules unless caller explicitly opted into inheritance via `inherit_from_caller_at_schedule = true` on the schedule call. Default = false.

**Smoke tests:** registry round-trip, `/agent` spawn picks up named config, cron with `agent_name` resolves correctly, `/agent delete` cleanly invalidates.

---

## Future Extension — Named Session Persistence (Deferred)

Design-only. Not implemented in Round 2. Phase 8 must preserve serialization shape so this stays viable.

**Storage:** `~/.claurst/managed-sessions/<name>/{settings.json, ephemeral.json, metadata.json}`.

**Verbs:** `/session save <name>`, `/session load <name>`, `/session list`, `/session delete <name>`, `/session persist`, `claurst --session <name>`.

**Schema versioning:** `metadata.json.schema_version: u32` (start at 1). Loader rejects unknown major; migration via `core::live_session::snapshot::migrate(snap, from_v, to_v)` chain of pure functions. Missing migration = explicit user-facing error, never silent.

**Constraints Round 2 must preserve:**
1. `EphemeralState` stays serializable (compile-time enforced via derive + doc comment).
2. `RuntimeHandles` rebuildable from `(Settings, EphemeralState)` — runtime holds artifacts; specs live in data.
3. Atomic replace via the protocol below.
4. Kairos gate not snapshotted; advisory `kairos_gate_hash` warns on mismatch.

**Action item Phase 8:** every `EphemeralState` field carries `#[derive(Serialize, Deserialize)]` and a doc comment flagging "must stay serializable for snapshot path."

---

## Atomic Replace Protocol

For `LiveSession::replace_from_snapshot` AND `/project switch`. Resolves "sync locks + async MCP I/O = no holding lock across await" tension.

**Six steps:**

1. **Acquire all sync write locks in fixed order:** `settings` → `ephemeral` → `runtime.working_directory` → `runtime.active_project` → `runtime.tools` → `runtime.mcp` → `runtime.permissions`. Order constant: `LIVE_SESSION_LOCK_ORDER`.
2. **Snapshot old runtime artifacts** to local Vec/HashMap. Move out old MCP handles (not clone). Tools list cloned (Arc-counted).
3. **Swap data fields in place** — `*settings.write() = snap.settings`, `*ephemeral.write() = snap.ephemeral`. Empty `runtime.mcp` map. Reset `runtime.tools` to built-ins only.
4. **Release all sync locks.**
5. **Outside locks:** drop old MCP handles (triggers async cleanup). Spawn concurrent rebuild of new handles from new specs.
6. **Insert new handles via short write-lock per server** as each becomes ready. Re-register MCP-backed tools in `runtime.tools` similarly.

**Window invariant:** tool calls during step 5 see empty MCP map → return `"MCP server not yet connected, retry"`. Documented as expected behavior.

**Partial failure:** any rebuild error logged as `EventKind::SnapshotPartialLoad { failed }`. No rollback. User can `/mcp add` manually.

---

## Concurrency / Robustness Decisions

- All `RwLock` / `Mutex` use `parking_lot` flavor — no poisoning. The existing `core::permissions::PermissionManager` is migrated to `parking_lot` as part of Phase 9 (it's small surface; no upstream concern since PR #110 is ignored).
- Lock acquisition order documented as `LIVE_SESSION_LOCK_ORDER` constant.
- Background task bodies wrapped in `futures::FutureExt::catch_unwind`. Panic → log to event log as `EventKind::TaskPanicked`, mark task failed in tracker, **process survives**.
- TUI render-path panics remain fatal; panic handler flushes event log JSONL before exit.
- `EventLog` writer single push site under `Mutex`; reader snapshots once per tick.

---

## Architecture Consistency Notes

Issues spotted during finalization. Resolved in this revision; flagged so reviewers can sanity-check before implementing:

1. **`AgentConfig.permission_rules` was over-loaded.** Earlier draft mixed seed defaults with active rule list, breaking session-snapshot semantics. Resolved by introducing `DefaultPermissionRule` (no scope, no provenance). Defaults are seed-only data; active state lives only in `PermissionManager`.
2. **`ProjectConfig.permission_rules: Vec<PermissionRule>` carries `scope` field redundantly** (storage location implies project scope). Kept the field for type uniformity; loader validates `scope == Project { name: self.name }` and refuses mismatches.
3. **`TaskSource` overlap risk.** `SlashCommand("/btw")` and `BgLoop("/btw")` both possible. **Resolved:** `/btw` always emits `BgLoop("/btw")`; `SlashCommand` reserved for inline slash commands (e.g. `/permissions grant`).
4. **Tool registry vs tool policy vs frozen agent config — three places.** `runtime.tools` (live instances), `ephemeral.tool_denylist` (live policy), `AgentConfig.tools` (frozen at spawn). Precedence: ephemeral defines policy; runtime owns instances; agent config = frozen snapshot. Spawn freezes policy; live changes do **not** propagate into in-flight task.
5. **Hot-tool-add vs hot-skill-add asymmetry.** Skills loaded mid-session are visible to the agent next turn (live re-resolve via `runtime.tools` for skills-as-tools). Tool enable/disable mid-session is **NOT** visible to in-flight spawned agents (frozen at spawn). Justification: tools have state (open connections, caches); skills are stateless content. Documented in `AgentConfig` rustdoc.
6. **`Arc<Box<dyn Tool>>` vs `Arc<dyn Tool>`.** Updated to `Arc<dyn Tool>` everywhere — cheaper, no double indirection. Migration from `Arc<Vec<Box<dyn Tool>>>` is the wide-blast-radius change in Phase 8.
7. **Event log double-write risk.** TaskTracker registration AND log push at every spawn site = duplicates. Resolved: tracker is observable; log push happens via tracker's lifecycle events, not spawn-site directly. Single producer per event.
8. **MCP spec sources triple.** `Settings.config.mcp_servers` (global), `ProjectConfig.mcp_servers` (project), `EphemeralState.mcp_specs` (session). Resolution at startup: global → active project → ephemeral added later. On `/project switch`: drop old project specs from `runtime.mcp`, add new project specs (atomic replace), leave ephemeral untouched.
9. **`PermissionRule` JSON migration.** Old `SerializedPermissionRule` lacks `id`, `subject`, `decision`, `created_at`. Handled by `#[serde(default)]` on each new field; `id` defaults to a fresh `Uuid::new_v4()` on load (deterministic enough for revoke), `subject` defaults to `None` (legacy `tool_name`/`path_pattern` stays authoritative for that rule), `decision` defaults from `action`, `created_at` defaults to `Utc::now()`. No `created_by` field — never was, isn't being added.
10. **`/stop all` and the Kairos proactive ticker.** Tick-spawn task is tracked; `/stop all` cancels it. Ticker itself (the loop) is **NOT** tracked — it's infrastructure, not a task. Cancel via existing `CancellationToken` on Kairos shutdown path.
11. **`Settings.permission_rules` field already exists** as `Vec<SerializedPermissionRule>` in the existing struct. Migration: rename serialized form's struct to merge into `PermissionRule` (with the serde defaults from note 9) and treat all loaded rules as `Forever` scope.
12. **`PermissionScope::Persistent` → `Forever` rename.** `#[serde(alias = "Persistent")]` keeps old JSON loadable; emit-side writes `Forever`. No data loss.

---

## Deferred Within Round 2

Items deferred mid-implementation; tracked here so they aren't forgotten.

- **LiveSession CLI bootstrap.** No production code path constructs a `LiveSession` yet — Phase 8/8.5 ships the type + helpers + smoke tests. `cli/main.rs` has multiple init paths (interactive, headless, ACP bridge, kairos-only); plumbing `SharedLiveSession` through `CommandContext` is invasive and lands as a dedicated task before `/project` slash commands. Until then, spawn sites pass `AgentConfig::default()`.
- **`/project list|switch|create|show` slash commands.** Underlying ops exist (`ProjectRegistry::load_from_dir`, `LiveSession::switch_project`); UI surface waits on the LiveSession bootstrap task. ETA: pre-Phase 9 if Phase 9 needs project-scoped permission rules at the slash-command level.
- **`AgentConfig.kairos_policy` field.** `KairosPermissionPolicy` enum lives on `core::permissions` (Phase 9 task #28); per-agent override field on `AgentConfig` is wired alongside Phase 11 (Named Agents) when agent serde shape gets its next pass. Until then, `KAIROS_PERMISSION_POLICY` env governs.
- **`TaskSource` on `PendingPermission`.** Enum exists; `PendingPermission` struct still carries no `source` field — added when Phase 9.5 introduces `PendingPermissionRequest` as the spawn-aware wrapper. Dialog attribution waits on that.
- **`evaluate()` consults `KairosPermissionPolicy`.** Policy enum is parseable + per-agent-overridable in shape, but `PermissionManager::evaluate()` does not yet route requests through it. Lands when `TaskSource` reaches `PendingPermission` (above) so background-source detection has a stable hook.
- **`PermissionRule.decision: PermissionDecision` field.** Spec listed it; deferred — `action: PermissionAction` still serves the rule-storage role unambiguously, and dual-storing a richer `decision` invites drift. Reconsider when a use case (e.g. attaching `Ask{reason}` to a stored rule) actually emerges.
- **`/permissions` slash command (list/revoke/grant).** Rule `id` + `created_at` exist on `PermissionRule`; UI surface waits on LiveSession CLI bootstrap (same blocker as `/project`).
- **Phase 9.5 producer sites beyond `execute_agent_run`.** `TaskTracker` ships and `execute_agent_run` registers `AgentRunTask`. Tool-dispatch `ToolCallTask`, cron-tick `CronTickTask` wrapper, and `AgentTool` `SubagentTask` not yet wired — landed when each crate's spawn site gets touched for unrelated work, or proactively if `/tasks` UX needs them populated.
- **`/tasks` + `/stop all` slash commands.** `TaskTracker::list_active`, `cancel`, `cancel_all` all exist; UI surface waits on LiveSession CLI bootstrap (same blocker).
- **`StopAllTasks` tool.** Phase 11 deliverable per spec — exposed to agents via explicit `tools.allowlist` only.
- **Tracker → event-log lifecycle hooks.** Tracker is observable; `/activity` integration lands with Phase 10.
- **Phase 10 producer sites beyond `execute_agent_run`.** `EventLog` ships and `BackgroundStart`/`BackgroundFinish` emit from `execute_agent_run`. TUI turn boundary, tool dispatch (`ToolCall`/`PermissionRequested`/`PermissionDecided`), cron `CronFired`, `AgentSpawned`, snapshot loader, panic boundary not yet wired — landed when each crate's site gets touched.
- **`/activity` modal + status-line wiring.** `EventLog::most_recent` and `snapshot`/`filter_by_source` exist; UI surface waits on LiveSession CLI bootstrap (same blocker).
- **Graceful-shutdown JSONL flush.** `EventLog::flush_to_jsonl` exists; CLI shutdown/panic handler wiring deferred to LiveSession bootstrap pass.

---

## Open / Underspecified

Items not yet explicitly resolved. Defaults applied where listed; await user override.

1. **`/tool disable <name>` retry semantics on tool-call failure.** Default applied: retry after failure also denied (denylist checked at every dispatch attempt, including retries).
2. **Project root changed on disk between save/load.** Default applied: warn via event log + fall back to live cwd at load time. No hard error.
3. **Sub-agent rule inheritance default.** Default applied: live session inheritance. Caller must explicitly opt out.
4. **Cron job rule inheritance default.** Default applied: false. Cron carries only its agent's defaults at schedule time; does not capture live session rules unless caller sets `inherit_from_caller_at_schedule = true`.
5. **`/project switch` mid-task behavior.** Default applied: in-flight tasks freeze rule set + MCP handles at spawn time; new spawns see new project.
6. **Rule conflict bias.** Default applied: most-specific subject wins; on equal specificity, Deny > Allow.
7. **`/stop all` blast radius.** Default applied: cancels everything tracked, including main turn's in-flight tool call.
8. **Hot-tool-add visibility for in-flight agent.** Default applied: tools frozen at spawn (asymmetric with skills — see Architecture Note 5).
9. **`StopAllTasks` default exposure.** Default applied: not in default agent tool set; explicit allowlist only.
10. **Project as namespace for agents.** Default applied: agents stay global; per-project agents in deferred list.

If any default is wrong, override before implementation starts on the affected phase.

---

## Nice-to-Have / Deferred

Not Round 2.

- **Connect to background agent session mid-execution.** `/attach <task-id>` swaps TUI focus to that task's I/O; on detach, task resumes background. Requires generic session management across foreground/background. Defer to Round 3.
- **Settings undo / `/settings diff` / `/settings revert <n>`.** Last-N writes log of `--persist` mutations with revert action. Mitigates misclick on `/model X --persist`.
- **`/permissions list --by-source cron` filter view.** Enabled by `TaskSource` enum; UI work deferred.
- **Read-mostly `Arc<Settings>` clone-cost mitigation.** Wrap large sub-fields in `Arc`. Deferred until profiling shows hot path.
- **Project-scoped agent definitions.** Per-project agents possible later via `ProjectConfig.agents` field.
- **Snapshot portability across machines.** `ProjectConfig.root_path` is absolute = non-portable. Add path-translation table to snapshot when needed.
- **Rule conflict UX.** Surface diff before `/permissions grant` overwrites existing rule.
- **Per-agent token budgets.** Out of scope (cost ceiling stays global).
- **Standalone agent files at `~/.claurst/agents/<name>.json`.** Currently agents live in `Settings.agents`. Standalone files would let users version-control agents independently.
- **`/agent <name> <prompt>` running in foreground vs background.** Phase 11 spawns background; future: explicit foreground variant for interactive agent take-over.

---

## Final Sequencing

```
Phase 8    — AgentConfig (renames AgentDefinition; type alias for back-compat)
    │
    ▼
Phase 8.5  — Project Registry
    │
    ▼
Phase 9    — Permission System (uses Project rules)
    │
    ▼
Phase 9.5  — Background Task Tracking
    │
    ▼
Phase 10   — Event Log + Status Line (consumes Tracker + Permission events)
    │
    ▼
Phase 11   — Named Agents (uses unified AgentConfig in Settings.agents)
```

Smoke tests live in `crates/<crate>/tests/smoke_phase_<n>.rs`. Each phase ships with passing smoke before merge.

---

## Definition of Done — Round 2

- `AgentConfig` is the single config type; `AgentDefinition` is a type alias.
- Project registry exists; `/project switch` swaps cwd + permission rules + MCP set atomically.
- `PermissionManager` enforces scope lifecycle (global / project / session / once); `Once` rules never tracked.
- Every background task is tracked, listable, cancellable. `/stop all` works. `StopAllTasks` tool gated.
- Event log replaces "recent activity" line; `/activity` opens full view; JSONL persists across restart.
- Named agents loadable from `Settings.agents` (and `EphemeralState.agents` for ephemeral).
- All locks are `parking_lot`; lock order documented; no panic kills the process.
- Each phase has smoke tests passing in CI.
- Snapshot serialization shape preserved (Future Extension stays viable).
