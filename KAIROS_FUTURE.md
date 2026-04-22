# Kairos Mode — Round 2 Roadmap

Round 1 (gate, runner, cron, proactive ticker, session bridge, task history) is implemented. See [KAIROS_CURRENT.md](KAIROS_CURRENT.md) for the current architecture and module map.

Round 2 transforms Kairos plumbing into a reusable agent-platform foundation: hierarchical configs, projects, scope-aware permissions, background task tracking, observable event log, and named agents.

---

## Design Principles

1. **One config per entity, cloned on use, logged at spawn.** No parallel "snapshot" types. Traceability via spawn-time logging.
2. **Hierarchical configs, flat per-entity.** `AgentConfig`, `ProjectConfig`, `ProviderConfig`, `ToolConfig`, `MCPConfig`. Composition by containment.
3. **Permissions explicit everywhere.** Background, foreground, sub-agent, cron — same prompt queue. Silent denial is a bug.
4. **Status visible by default.** Every spawn / tool call / permission prompt / cron fire emits an event next to the avatar; full log via slash command.
5. **Rebase-friendly.** PR #110 (`feat/permission-manager-tui`) types vendored; our extensions sit on top.
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
| Permission layer (core::permissions, vendored from PR #110)      |
|   PermissionManager: scope-aware (global/project/session/once)   |
|   PendingPermissionStore: dialog queue                           |
+------------------------------------------------------------------+
| Observability (tui::event_log)                                   |
|   EventLog ring buffer + JSONL flush on shutdown                 |
|   Avatar-line current event; /activity full view                 |
+------------------------------------------------------------------+
```

---

## Prerequisites

- **PR #110 vendored.** Module `claurst-core::permissions::vendored` re-exports upstream types (`PermissionManager`, `PendingPermissionStore`, `PendingPermissionRequest`, `PermissionRequest`, `PermissionDecision`). Round 2 code references vendored aliases. If upstream renames during review, only `vendored.rs` changes.
- **Rebase Kairos branch onto post-PR-110 main** before starting Phase 8. Conflict areas: `tools/src/lib.rs` (ToolContext fields), `cli/src/main.rs` (handler construction).

---

## Type Specs (Final)

### AgentConfig

```rust
// claurst-core::config::agent_config
pub struct AgentConfig {
    pub name: Option<String>,                      // None = anonymous (ambient)
    pub provider: ProviderConfig,
    pub tools: ToolConfig,
    pub mcp: MCPConfig,
    pub permission_defaults: Vec<DefaultPermissionRule>,
    pub permission_mode: PermissionMode,
    pub kairos_policy: KairosPermissionPolicy,
    pub system_prompt: Option<String>,
    pub append_system_prompt: Option<String>,
    pub output_style: OutputStyle,
    pub effort: Option<EffortLevel>,
    pub max_turns: Option<usize>,
    pub project: Option<String>,                   // project-name reference
    pub kairos_addendum: bool,
    // NO working_directory — see Working Directory Resolution below.
}

// Defaults are seed-only: no scope (scope = Session of spawned manager),
// no created_at, no created_by. Disambiguates from active rules.
pub struct DefaultPermissionRule {
    pub subject: PermissionSubject,
    pub decision: PermissionDecision,
}
```

`AgentConfig` is the single config type for all spawn sites. **Replaces** the existing `AgentDefinition`. Migration: serde rename / `#[serde(default)]` on new fields; existing `~/.claurst/agents/*.json` (if any) load unchanged.

### ProviderConfig, ToolConfig, MCPConfig

```rust
pub struct ProviderConfig {
    pub provider_id: String,
    pub model: String,
    pub max_tokens: usize,
    pub temperature: Option<f32>,
    pub api_base: Option<String>,
}

pub struct ToolConfig {
    pub allowlist: Option<Vec<String>>,            // None = all enabled
    pub denylist: Vec<String>,
    pub per_tool_overrides: BTreeMap<String, Value>,
}

pub struct MCPConfig {
    pub enabled_servers: Vec<String>,              // explicit per-agent subset
}
```

### ProjectConfig + ProjectRegistry

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

```rust
// claurst-core::permissions
pub enum PermissionScope {
    Once,                                          // never persisted, never listed
    Session,                                       // until process exit
    Project { name: String },                      // persisted in ProjectConfig
    Forever,                                       // persisted in Settings
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
    pub id: Uuid,
    pub subject: PermissionSubject,
    pub scope: PermissionScope,
    pub decision: PermissionDecision,
    pub created_at: DateTime<Utc>,
    #[serde(default = "TaskSource::system")]
    pub created_by: TaskSource,
}

pub enum TaskSource {
    MainSession,
    SlashCommand(String),                          // inline slash commands
    Cron(String),                                  // task id
    Proactive,
    Agent(String),                                 // agent name
    BgLoop(String),                                // /btw etc.
    System,
}
// One enum used by both PendingPermissionRequest.source AND PermissionRule.created_by.
```

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
    pub permissions:       Arc<Mutex<vendored::PermissionManager>>,
    pub cost_tracker:      Arc<CostTracker>,
    pub tasks:             Arc<TaskTracker>,
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

**Goal:** `AgentConfig` plumbed through every spawn site. `AgentDefinition` removed. `QueryConfig` deleted at end of phase.

**Deliverables:**
1. `claurst-core::config::agent_config` module with all structs above.
2. `Settings.agents: HashMap<String, AgentConfig>` (replaces `AgentDefinition`). Serde migration handles old files via `#[serde(default)]` and renames.
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

- **Default:** `AgentConfig.permission_defaults` is converted to `Vec<PermissionRule>` with `scope = Session`, `created_by = Agent(name)` and seeded into the spawned task's manager view.
- **Opt-in inheritance:** caller sets `inherit_live_session_rules: bool` on `AgentRunRequest`. If true, live session's current rules also seeded; live wins on conflict.

### Conflict precedence

When multiple rules match: **most-specific subject wins; on equal specificity, Deny > Allow.** Specificity ordered: Composite > ToolInput > Tool > Path/Url/Command > broader. Documented with examples in unit tests.

### First-prompt flow

1. Tool call → `ToolContext::check_permission(req)` → `PermissionManager::evaluate(&req)`.
2. No rule match → return `Ask`. Push to `PendingPermissionStore`.
3. TUI dialog renders: tool name, subject summary, `request.source` attribution, scope buttons (Once/Session/Project/Forever × Allow/Deny).
4. Decision → if scope ≠ Once, store `PermissionRule` with `created_by = MainSession`; persist if Project/Forever.
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

`TaskSource` enum on both `PendingPermissionRequest.source` (who's asking *now*) and `PermissionRule.created_by` (who created the rule). Lets `/permissions list` filter by source. Clarifies the difference between "who is requesting permission" and "who originally created the rule".

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

- All `RwLock` / `Mutex` use `parking_lot` flavor — no poisoning. **Exception:** vendored PR #110 types use `std::sync::Mutex`; accept upstream as-is, don't wrap.
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
9. **`PermissionRule.created_by` missing in old persisted JSON.** Handled by `#[serde(default)]` defaulting to `TaskSource::System`.
10. **`/stop all` and the Kairos proactive ticker.** Tick-spawn task is tracked; `/stop all` cancels it. Ticker itself (the loop) is **NOT** tracked — it's infrastructure, not a task. Cancel via existing `CancellationToken` on Kairos shutdown path.
11. **`Settings.permission_rules` field already exists** as `Vec<SerializedPermissionRule>` in the existing struct. Migration: rename type to `PermissionRule` (keep serialized form compatible) and treat as Forever scope on load.

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
PR #110 vendored (or upstream merge)
    │
    ▼
Rebase Kairos onto post-PR-110 main
    │
    ▼
Phase 8    — AgentConfig (replaces AgentDefinition)
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

- `AgentConfig` is the single config type; `AgentDefinition` deleted.
- Project registry exists; `/project switch` swaps cwd + permission rules + MCP set atomically.
- `PermissionManager` enforces scope lifecycle (global / project / session / once); `Once` rules never tracked.
- Every background task is tracked, listable, cancellable. `/stop all` works. `StopAllTasks` tool gated.
- Event log replaces "recent activity" line; `/activity` opens full view; JSONL persists across restart.
- Named agents loadable from `Settings.agents` (and `EphemeralState.agents` for ephemeral).
- All locks are `parking_lot`; lock order documented; no panic kills the process.
- Each phase has smoke tests passing in CI.
- Snapshot serialization shape preserved (Future Extension stays viable).
