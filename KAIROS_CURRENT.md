# Kairos Mode — Developer Architecture Overview

A guide for developers extending or debugging Kairos. For the implementation
roadmap and phase log, see `KAIROS_FUTURE.md`.

---

## 1. What Kairos Is (One Paragraph)

Kairos is an experimental *assistant mode* that turns Claurst from a strictly
reactive chat tool into one that can also (a) push notifications to the user on
its own initiative (`Brief`), (b) run scheduled prompts (`CronCreate`), (c)
execute slash commands asynchronously without blocking the TUI (`/btw`), (d)
tick autonomously on a timer (proactive mode), and (e) resume the last session
automatically when the user re-opens Claurst in the same working directory
(session bridge). Round 2 added a hierarchical config layer (`AgentConfig`,
`ProjectConfig`), an in-memory live-session aggregate (`LiveSession`), a
process-wide background task tracker (`TaskTracker`), and an event log
(`EventLog`) consumed by `/activity`. Every background unit funnels through
the same gate, runner, tracker, and event log.

---

## 2. Architectural Layers (Responsibility Breakdown)

```
┌─────────────────────────────────────────────────────────────────────────┐
│ TUI loop (crates/cli/src/main.rs)                                       │
│   • startup init, LiveSession bootstrap, tracker + event_log creation   │
│   • cron + ticker spawn, drain loop, JSONL flush on graceful exit       │
└────────────────┬──────────────────────────────────────┬─────────────────┘
                 │                                      │
                 │ bg_task_tx (AgentRunResult)          │ command_queue
                 ▼                                      ▼
┌─────────────────────────────────────────────────────────────────────────┐
│ Background runner (crates/query/src/background_runner.rs)               │
│   execute_agent_run(request, context) → bool(is_error)                  │
│   │ ┌ runs query loop, records to task_history                          │
│   │ ├ registers AgentRunTask in TaskTracker (Phase 9.5)                 │
│   │ ├ emits BackgroundStart/BackgroundFinish to EventLog (Phase 10)     │
│   │ └ sends AgentRunResult to result_tx when present                    │
└─▲────────────────────▲──────────────────────▲─────────────┬─────────────┘
  │                    │                      │             │
  │ Cron               │ Proactive            │ /btw        │ records
  │ scheduler          │ ticker               │ helper      │
  │ (cron_scheduler)   │ (proactive_ticker)   │ (main.rs)   │
  │                    │                      │             ▼
  │                    │                      │   ┌─────────────────────┐
  │                    │                      │   │ task_history (core) │
  │                    │                      │   │   ring buffer 100   │
  │                    │                      │   │   JSONL append      │
  │                    │                      │   └─────────────────────┘
  │                    │                      │
┌─┴────────────────────┴──────────────────────┴───────────────────────────┐
│ Kairos gate (crates/core/src/kairos_gate.rs)                            │
│   resolve_runtime_state → KairosRuntimeState + Diagnostics              │
│   is_kairos_brief_active / channels / proactive                         │
│   proactive_interval_secs, proactive_tick_max_usd, prompts              │
└─▲───────────────────────────────────────────────────────────────────────┘
  │
┌─┴───────────────────────┐ ┌─────────────────────────────────────────────┐
│ Feature flag compile    │ │ Live layer (core::live_session)             │
│ gate (cli/tui/tools/    │ │   LiveSession {                             │
│  core Cargo.toml)       │ │     settings:  Arc<RwLock<Settings>>,       │
└─────────────────────────┘ │     ephemeral: Arc<RwLock<EphemeralState>>, │
                            │     runtime:   RuntimeHandles {             │
┌─────────────────────────┐ │       working_directory, active_project,    │
│ Session bridge          │ │       project_registry, permissions,        │
│ (core/session_bridge.rs)│ │       cost_tracker                          │
└─────────────────────────┘ │     }                                       │
                            │   }                                         │
┌─────────────────────────┐ └─────────────────────────────────────────────┘
│ Project registry        │ ┌─────────────────────────────────────────────┐
│ (~/.claurst/projects/   │ │ TaskTracker (core::task_tracker)            │
│  <name>.json)           │ │   trait TrackedTask                          │
└─────────────────────────┘ │   SimpleTrackedTask + CancellationToken     │
                            │   /tasks list/show/cancel, /stop all        │
┌─────────────────────────┐ └─────────────────────────────────────────────┘
│ Permission manager      │ ┌─────────────────────────────────────────────┐
│ (core::permissions)     │ │ EventLog (core::event_log)                  │
│   evaluate_with_source  │ │   ring buffer (cap 2000) + JSONL flush      │
│   PermissionScope: Once │ │   producers: runner, cron, permissions,     │
│   /Session/Project/Forev│ │              tool dispatch                  │
│   PermissionSubject     │ │   consumers: /activity, status line         │
│   TaskSource attribution│ └─────────────────────────────────────────────┘
└─────────────────────────┘
```

Roughly: each layer below only knows about the layers below it. The gate has
no knowledge of the TUI; the runner has no knowledge of cron vs. proactive vs.
`/btw` beyond an `AgentRunSource` enum. Cross-cutting handles (`LiveSession`,
`TaskTracker`, `EventLog`) attach via `CommandContext` and `ToolContext`
fields populated at TUI bootstrap.

---

## 3. Key Structs and Where They Live

### Round 1 — Kairos plumbing

| Struct / enum | Crate::module | What it does |
|---|---|---|
| `KairosRuntimeState` | `core::kairos_gate` | Frozen snapshot of brief/channels/proactive flags + entitlement + diagnostics. Set once at startup. |
| `KairosGateDiagnostics` | `core::kairos_gate` | Every input that fed the gate decision. Rendered by `/kairos`. |
| `BridgePointer` | `core::session_bridge` | `{session_id, working_dir, started_at, last_active_at}` — file at `~/.claurst/bridge/{session_id}.json`. |
| `TaskRunRecord` / `RunStatus` | `core::task_history` | One record per background run. Ring buffer + JSONL append. |
| `AgentRunRequest` | `query::background_runner` | `{run_id, source, prompt}` — input to the runner. |
| `AgentRunContext` | `query::background_runner` | Shared deps: `query_config`, `agent_config`, `tool_ctx`, `client`, `tools`, `result_tx`, `task_tracker`, `event_log`. |
| `AgentRunResult` | `query::background_runner` | `{run_id, source, output, is_error}` — flows to TUI via `bg_task_tx`. |
| `AgentRunSource` | `query::background_runner` | `SlashCommand{name} | Cron{task_id} | Proactive`. Maps to `TaskSource` via `as_task_source()`. |
| `CommandExecutionPolicy` | `commands::lib` | `BackgroundSafe | ForegroundOnly` — per-slash-command flag. |
| `CronTask` + on-disk store | `tools::cron` | Serde struct for scheduled prompts; gained `agent_name` in Phase 11 (`#[serde(default, skip_serializing_if = "Option::is_none")]`). |

### Round 2 — config + live layer

| Struct / enum | Crate::module | What it does |
|---|---|---|
| `AgentConfig` | `core::config` | Single config for every spawn site. Renames legacy `AgentDefinition` (kept as `pub type` alias). Adds `system_prompt`, `tools`, `mcp`, `project`, `kairos_addendum`, `kairos_policy`, `effort`, `thinking_budget`, `tool_result_budget`, etc. |
| `ToolConfig`, `MCPConfig` | `core::config` | Per-agent allow/deny lists; `MCPConfig.enabled_servers` references the unified runtime registry. |
| `ProjectConfig` | `core::project_registry` | First-class project: `name`, `root_path`, `permission_rules`, `default_agent`, `mcp_servers`. |
| `ProjectRegistry` | `core::project_registry` | `BTreeMap<String, ProjectConfig>`. Loaded from `~/.claurst/projects/<name>.json`. |
| `LiveSession` / `SharedLiveSession` | `core::live_session` | Per-process aggregate: `settings`, `ephemeral`, `runtime: RuntimeHandles`. Cloned shallowly. |
| `EphemeralState` | `core::live_session` | Per-session overlay: `mcp_specs`, `agents`, `tool_allowlist`/`denylist`, `overrides`. Serializable for future named-session-snapshot path. |
| `RuntimeHandles` | `core::live_session` | `working_directory`, `active_project`, `project_registry`, `cost_tracker`, `permissions: Arc<Mutex<PermissionManager>>`. Cross-crate handles (`tools`, `mcp`, `task_tracker`, `event_log`) plumbed alongside via `CommandContext` / `ToolContext` until they migrate onto `RuntimeHandles`. |

### Round 2 — permissions

| Struct / enum | Crate::module | What it does |
|---|---|---|
| `PermissionScope` | `core::permissions` | `Once | Session | Project { name } | Forever`. Persistent renamed to Forever (`#[serde(alias = "Persistent")]`). |
| `PermissionSubject` | `core::permissions` | `Tool` / `ToolInput` / `Path` / `Url` / `Command` / `Composite`. Supersedes legacy `tool_name + path_pattern` when set. |
| `TaskSource` | `core::permissions` | Attribution: `MainSession | SlashCommand | Cron | Proactive | Agent | BgLoop | System`. Lives on `PendingPermission.source`. |
| `KairosPermissionPolicy` | `core::permissions` | `DeferToUser` (default) / `AutoAllowRead` / `Reject`. Per-agent override via `AgentConfig.kairos_policy`; env `KAIROS_PERMISSION_POLICY`. |
| `PermissionRule` | `core::permissions` | Gained `id: Uuid`, `subject`, `created_at`. Legacy `tool_name`/`path_pattern` kept for back-compat. |
| `PermissionManager` | `core::permissions` | Holds `mode`, `session_rules`, `persistent_rules`, `pending`, optional `event_log` handle. Methods: `evaluate`, `evaluate_with_source`, `add_rule`, `remove_rule_by_id`, `list_rules`, `register_pending_with_source`, `pending_snapshot`. |

### Round 2 — observability + tracking

| Struct / enum | Crate::module | What it does |
|---|---|---|
| `TrackedTask` (trait) | `core::task_tracker` | `id`, `kind`, `source`, `started_at`, `status`, `summary`, `details`, `cancel`. |
| `TaskKind` | `core::task_tracker` | `Tool | Agent | Cron | Subagent | BgLoop`. |
| `TaskStatus` | `core::task_tracker` | `Running | Waiting{reason} | Completed | Failed{error} | Cancelled`. |
| `TaskTracker` | `core::task_tracker` | `register`, `deregister`, `list_active`, `get`, `cancel`, `cancel_all`. |
| `SimpleTrackedTask` | `core::task_tracker` | Reference impl backed by interior mutability + `CancellationToken`. |
| `EventLog` | `core::event_log` | Ring buffer (`DEFAULT_RING_CAPACITY = 2000`) + optional JSONL path (`~/.claurst/event_log.jsonl`). `push`, `snapshot`, `most_recent`, `filter_by_source`, `flush_to_jsonl`, `load_from_jsonl`. |
| `Event` / `EventKind` | `core::event_log` | TurnStart/End, ToolCall{tool, status}, BackgroundStart/Finish{is_error}, PermissionRequested/Decided, CronFired{task_id}, AgentSpawned{agent_name}, ConfigChanged, TaskPanicked, SnapshotPartialLoad, Error, Info. |
| `StopAllTasksTool` | `tools::stop_all_tasks` | Cancels every tracked task. `PermissionLevel::Dangerous`. NOT in default tool set — opt-in via `AgentConfig.tools.allowlist`. |

---

## 4. Guardrails — Where Each One Takes Effect

| Guardrail | Fires at | Reference |
|---|---|---|
| Compile-time feature gate | Linker. If `kairos_brief` feature off, tools and gate calls fall back to `false`. | Cargo.toml chain (`cli → tui → tools → core`) |
| Env-var activation | `resolve_runtime_state` on process start. | `core::kairos_gate::resolve_runtime_state` |
| Onboarding trust check | Same. Skipped only when `KAIROS_TRUST_BYPASS=1`. | `core::kairos_gate::resolve_runtime_state` |
| GrowthBook entitlement | Same. Can be forced open (`KAIROS_FORCE=1`) or required hard (`KAIROS_REQUIRE_ENTITLEMENT=1`). | `core::kairos_gate::check_entitlement` |
| Tool exposure gate | Each model turn. Tools list excludes `Brief`/`Cron*` unless `kairos_brief_tools_enabled()` returns true. | `tools::lib::all_tools` |
| Cron job cap | `CronCreate` tool execution. | `tools::cron::max_cron_jobs` (env `KAIROS_MAX_CRON_JOBS`, default 50, clamp [1,1000]) |
| Proactive backoff | After 3 consecutive errored ticks, sleep interval doubles until a successful tick resets the counter. | `query::proactive_ticker` (`MAX_CONSECUTIVE_ERRORS=3`) |
| Proactive per-tick cost ceiling | After 2 consecutive ticks whose `cost_tracker` delta exceeds `KAIROS_TICK_MAX_USD`, the ticker logs and exits. | `query::proactive_ticker` (`MAX_COST_OVERRUNS=2`) |
| Bridge pointer TTL | On startup scan (`find_active_pointer`) and cleanup. Pointers older than `KAIROS_BRIDGE_TTL_SECS` (default 4h, min 60s) are deleted. | `core::session_bridge` |
| Bridge write debounce | TUI drain loop, after each `save_session`. At most one disk write per 30s. | `crates/cli/src/main.rs` |
| MCP settlement wait | Before `/btw` background execution. 5s poll so tools resolve before a fresh query starts. | `crates/cli/src/main.rs` (`spawn_background_slash_command`) |
| Kairos permission policy | `evaluate_with_source` collapses `Ask` → `Deny` (Reject) or `Allow` (AutoAllowRead+read-only) for non-foreground sources. Foreground unaffected. | `core::permissions::evaluate_with_source` |
| Permission scope `Once` | Resolved inline by tool dispatch; never enters `PermissionManager`. | `core::permissions::add_rule` |

---

## 5. Startup Sequence (Entry Points)

```
main()
├─ named-command path (headless)
│   └─ initialize_runtime_state(has_completed_onboarding).await
│                → sets RUNTIME_STATE, returns KairosRuntimeState
│   (no LiveSession; CommandContext.live_session = None)
│
└─ run_interactive()
    ├─ initialize_runtime_state(...).await           ← gate resolved here
    ├─ apply_kairos_bootstrap_to_query_config(...)   ← mutates base QueryConfig
    ├─ LiveSession::with_projects(settings, cwd, …)  ← Phase 8/8.5 bootstrap
    │     • loads ~/.claurst/projects/ into ProjectRegistry
    │     • constructs PermissionManager from settings.permission_rules
    ├─ Build initial CommandContext + ToolContext (live_session = Some(...))
    ├─ bg_task_tx/rx created (unbounded mpsc)
    ├─ TaskTracker::new() + EventLog::new()          ← Phase 9.5 / 10
    ├─ cmd_ctx.task_tracker / event_log = Some(...)  ← propagate to spawn sites
    ├─ tool_ctx.task_tracker / event_log = Some(...) ← Phase 10 ToolCall events
    ├─ live_session.runtime.permissions.lock().set_event_log(...)
    │     • PermissionRequested + PermissionDecided emit on register/resolve
    ├─ start_cron_scheduler(..., live_session, task_tracker, event_log)
    ├─ start_proactive_ticker(..., task_tracker, event_log) if proactive active
    ├─ /btw and other BackgroundSafe slash commands → spawn_background_slash_command
    ├─ apply_kairos_bootstrap_to_query_config re-applied per turn (new qcfg)
    ├─ drain loop — bg_task_rx.try_recv() → command_queue + notification + push_message
    ├─ bridge pointer debounced upsert (30s) after each save_session
    └─ on exit:
        • proactive_cancel.cancel(); cron_cancel.cancel()
        • event_log.flush_to_jsonl()                  ← Phase 10 graceful shutdown
```

**Bootstrap means:** two things. (1) Set `QueryConfig.kairos_enabled = true`.
(2) Append `assistant_system_prompt_addendum(proactive_enabled)` to the system
prompt so the model knows it is in Kairos mode. See
`crates/query/src/lib.rs::apply_kairos_bootstrap_to_query_config`.

---

## 6. TUI Integration

```
User action          TUI behaviour                    Kairos mechanism
─────────────────────────────────────────────────────────────────────────
/btw <prompt>        Immediate toast:                 spawn_background_slash_command
                     "Queued /btw in background."       → wait_for_mcp_settlement
                     Conversation NOT blocked.          → execute_agent_run(SlashCommand)
                                                        → register AgentRunTask
                                                        → emit BackgroundStart/Finish
                                                        → AgentRunResult → bg_task_tx

Cron tick fires      Toast on arrival:                cron_scheduler (tokio task)
(minute granularity)  "Background cron:<id> finished."   → pop_due_tasks
                     System message injected into       → emit CronFired
                     conversation via command_queue.    → resolve_agent_config(task.agent_name)
                                                        → execute_agent_run(Cron{id})

Proactive tick       Toast + system message.           proactive_ticker
                     Label reads "kairos".              → execute_agent_run(Proactive)

Model calls Brief    Toast + message. Model decides   BriefTool (permission_level=None)
                     wording.                           → pushes ToolResult the TUI renders

Tool dispatch        (silent — emits to event_log)    execute_tool
                                                        → ToolCall{Started→Succeeded/Failed}

/kairos [N]          Rendered gate state,             KairosCommand (foreground, sync)
                     diagnostics, cron list, last N     → reads runtime_state, list_tasks,
                     run records.                         last_runs[_by_cron_id]

/permissions [args]  set/allow/deny/reset/list/revoke PermissionsCommand
                                                        → settings + LiveSession.permissions

/project [args]      list/show/switch/create          ProjectCommand
                                                        → LiveSession + ProjectRegistry

/tasks [args]        list/show <id>/cancel <id>       TasksCommand → TaskTracker

/stop all [--yes]    Confirm + cancel_all             StopCommand → TaskTracker.cancel_all

/activity [N] [-s]   Tail of event_log ring           ActivityCommand → EventLog.snapshot
```

**Drain loop pattern**: every frame, drain `bg_task_rx` non-blocking and for
each `AgentRunResult` do three things in order:
1. push `QueuedCommand::InjectSystemMessage(meta)` into `command_queue` so the
   model sees the background output on its next turn;
2. push a toast into `app.notifications`;
3. append the meta string into `app.messages` so it appears in the scrollback.

This is the *only* path background output takes into the TUI. Anything new
that wants to surface a result should go through `AgentRunResult`.

---

## 7. Feature → UX Mapping

### 7.1 Brief (`BriefTool`)
- **Who triggers it:** the model, unprompted.
- **What the user sees:** a tool-result block in the current transcript with
  the model's `message` and optional `attachments`, plus no additional
  approval prompt (permission level `None`).
- **Code:** `crates/tools/src/brief.rs`.

### 7.2 Cron (`CronCreate`, `CronDelete`, `CronList`)
- **Triggers:** model calls a tool; no direct slash command for create/delete.
- **Phase 11:** `CronTask.agent_name: Option<String>` lets a cron task pin a
  named agent. Resolution happens at fire time via
  `live_session.resolve_agent_config(task.agent_name.as_deref())`.
- **What the user sees:** tool result confirming task ID; later, when the
  task fires, a toast `"Background cron:<id> finished."` plus a system
  message injected into the conversation. `/kairos` lists active tasks with
  their last run timestamp and status. `/activity` shows the `CronFired`
  event.
- **Persistence:** `durable=true` writes to `~/.claurst/cron.json`; reload on
  startup.
- **Code:** `crates/tools/src/cron.rs`, `crates/query/src/cron_scheduler.rs`.

### 7.3 Background slash commands (`/btw`, etc.)
- **Trigger:** user types a slash command whose `execution_policy()` returns
  `BackgroundSafe`.
- **Code:** `crates/cli/src/main.rs::spawn_background_slash_command`,
  `crates/commands/src/lib.rs` (policy definition).

### 7.4 Proactive mode (`KAIROS_PROACTIVE=1`)
- **Trigger:** timer in the ticker (every `KAIROS_PROACTIVE_INTERVAL_SECS`).
- **Prompt sent to model:** `proactive_tick_prompt()` (defined in
  `kairos_gate.rs` — only prompt text, static).

### 7.5 Session bridge (auto-resume)
- **Trigger:** startup with no explicit `--resume` flag.
- **Code:** `crates/core/src/session_bridge.rs`.

### 7.6 `/kairos` status
- **Trigger:** user slash command.
- **Code:** `crates/commands/src/lib.rs::KairosCommand`.

### 7.7 `/project` (Phase 8.5)
- `list` — registered projects + active marker.
- `show` — active project's root, default agent, permission rules count, MCP servers.
- `switch <name>` — atomic swap: cwd + active marker. Permission rule + MCP
  swap on switch is partial (see §13 Known Gaps).
- `create <name> --root <path>` — register + persist to `~/.claurst/projects/<name>.json`.

### 7.8 `/permissions` (Phase 9)
- No args — show current mode + allowed/denied lists.
- `set <mode>` / `allow <tool>` / `deny <tool>` / `reset` — settings-level shorthand.
- `list` — full rule list with stable UUIDs and scope (session + persistent + every project bucket).
- `revoke <id>` — remove rule by id (session, project, or persistent + mirror to disk for persistent).
- `grant <subject> <scope> [allow|deny]` — subject-aware rule authoring.
  Subject grammar: `tool:NAME | tool-input:NAME[:CONTAINS] | path:PATH[:read|write|any] | url:GLOB | cmd:bash|powershell|any:GLOB`.
  Scope grammar: `once | session | forever | project[:NAME]` (project falls back to active project name).
  Forever-scoped grants persist into `Settings.permission_rules`; project-scoped grants live in the manager's
  `project_rules` bucket (disk-mirror to `ProjectConfig.permission_rules` is still pending).

### 7.9 `/tasks` and `/stop all` (Phase 9.5)
- `/tasks` — list every tracked unit (agent runs, cron ticks, sub-agents).
- `/tasks show <id>` / `cancel <id>`.
- `/stop all [--yes]` — cancel every tracked task. Confirmation gate unless `--yes`.

### 7.10 `/activity` (Phase 10)
- `/activity [N] [--source <kind>]` — tail of the event-log ring, default 20,
  source filter accepts `main|cron|proactive|agent|bgloop|system|slash`.
- Persistence: graceful exit flushes the ring to `~/.claurst/event_log.jsonl`.
- **Pending:** scrollable modal + status-line wiring (most_recent on avatar line).

### 7.11 `/agent` (Phase 11)
- `/agent` or `/agent list` — list agents from builtins + Settings + EphemeralState (origin tag per entry).
- `/agent show <name>` — full details (description, model, max_turns, project, kairos addendum/policy, prompt overrides).
- `/agent create <name>` — seed an EphemeralState agent from `LiveSession::resolve_agent_config(None)`.
- `/agent delete <name>` — remove (ephemeral first, then settings; mirrors settings removal to disk).
- `/agent persist <name>` — promote EphemeralState → Settings.agents and persist via `save_settings_mutation`.
- `/agent run <name> <prompt>` — spawn the agent in the background via `execute_agent_run` (Phase 11). Mirrors `/btw`. Emits `AgentSpawned` event for /activity attribution.
- `/agent <name>` — sugar for `show <name>`.
- `/agent <name> <prompt>` — sugar for `run <name> <prompt>` when `<name>` resolves to a known agent.

---

## 8. System Prompts — Where They Live

There are exactly two prompt strings under Kairos control, both in
`crates/core/src/kairos_gate.rs`:

| Function | Purpose |
|---|---|
| `assistant_system_prompt_addendum(proactive_enabled: bool)` | Appended to the base system prompt by `apply_kairos_bootstrap_to_query_config`. Tells the model Kairos is on, to be terse, to use `Brief` for notifications, and (when proactive) to pace autonomous work. |
| `proactive_tick_prompt()` | User-turn message sent on each proactive tick. Static. |

Brief, cron, and background slash output are delivered to the model as plain
system/user messages (see drain loop); they do not introduce new prompt
templates.

`AgentConfig.system_prompt` (override) and `AgentConfig.append_system_prompt`
(suffix) thread through `apply_agent_config_to_query_config` — applied on
every spawn after Kairos bootstrap.

---

## 9. Configuration — ENV + Disk

Kairos gate state stays ENV-only (rationale: must resolve before settings
load completes on some paths, and must be auditable from the launching
shell). Round 2 added two on-disk artifacts that ARE part of `~/.claurst`:

- `~/.claurst/projects/<name>.json` — `ProjectConfig` per project. Loaded at
  startup into `ProjectRegistry`. Atomic save via tmpfile + rename.
- `~/.claurst/event_log.jsonl` — append-only event log, flushed once on
  graceful shutdown. Re-loadable via `EventLog::load_from_jsonl` (skips
  malformed lines with warn).

Priority for gate flags: ENV > compile-time feature flag.

| Env var | Default | Range / effect | Read at |
|---|---|---|---|
| `KAIROS` | unset | Umbrella opt-in. Enables brief + channels + proactive. | `resolve_runtime_state` |
| `KAIROS_BRIEF` | unset | Brief-only opt-in (also set by `KAIROS`). | same |
| `KAIROS_CHANNELS` | unset | Channels opt-in. No implementation yet. | same |
| `KAIROS_PROACTIVE` | unset | Proactive loop opt-in. Requires brief also active. | same |
| `KAIROS_TRUST_BYPASS` | unset | Skip onboarding-trust requirement. | same |
| `KAIROS_FORCE` | unset | Bypass GrowthBook entitlement check. | same |
| `KAIROS_REQUIRE_ENTITLEMENT` | unset | Fail closed if entitlement fetch fails or flag is off. | same |
| `KAIROS_PROACTIVE_INTERVAL_SECS` | 900 | Clamped to [60, 3600]. Tick period. | `proactive_interval_secs` |
| `KAIROS_TICK_MAX_USD` | unset | Per-tick cost ceiling. Two consecutive overruns stop the ticker. | `proactive_tick_max_usd` |
| `KAIROS_MAX_CRON_JOBS` | 50 | Clamped to [1, 1000]. | `tools::cron::max_cron_jobs` |
| `KAIROS_BRIDGE_TTL_SECS` | 14400 (4h) | Min 60s. Pointers older than this are stale. | `core::session_bridge::bridge_ttl_secs` |
| `KAIROS_PERMISSION_POLICY` | `defer` | `defer | read | reject`. Default for non-foreground requests; per-agent override via `AgentConfig.kairos_policy`. | `core::permissions::KairosPermissionPolicy::from_env_str` |

`is_env_truthy` accepts `1`, `true`, `yes`, `on` (case-insensitive).

---

## 10. Minimum Activation Recipe

```bash
# Build with feature + local-dev bypasses (no remote entitlement):
KAIROS=1 KAIROS_TRUST_BYPASS=1 \
  cargo run --features kairos_brief

# Enable proactive as well:
KAIROS=1 KAIROS_TRUST_BYPASS=1 KAIROS_PROACTIVE=1 \
KAIROS_PROACTIVE_INTERVAL_SECS=60 KAIROS_TICK_MAX_USD=0.10 \
  cargo run --features kairos_brief
```

Verify in the running TUI:

```
/kairos
/tasks
/activity
/project list
/permissions list
```

---

## 11. Extension Points — Where to Hook New Work

| You want to add… | Extend here |
|---|---|
| A new autonomous driver (new kind of background task) | New caller of `execute_agent_run`, add a variant to `AgentRunSource` + map in `as_task_source()`. |
| A new Kairos-only tool | Add the struct in `crates/tools/src/`, register inside `if kairos_brief_tools_enabled()` in `tools::lib::all_tools`. |
| A new background-safe slash command | Override `execution_policy()` in `crates/commands/src/lib.rs` to return `BackgroundSafe`. |
| A new gate input | Extend `KairosGateDiagnostics` and the logic in `resolve_runtime_state`. Update `format_summary`. |
| A new persisted Kairos artifact | Prefer `~/.claurst/...` and write a round-trip test next to the serde struct. |
| New recorded metrics on a background run | Extend `TaskRunRecord` in `core::task_history`; `execute_agent_run` is the single write site. |
| A new env knob | Add parser in `core::kairos_gate` (or `session_bridge`/`cron`). Document in §9 above. |
| A new `EventKind` variant | Extend `core::event_log::EventKind`. Producers push via `event_log.push(Event::now(...))`. Consumers (`/activity`) match exhaustively. |
| A new `TrackedTask` producer | Implement the trait or use `SimpleTrackedTask`. Register on spawn, deregister on terminal status. |
| A new permission scope or subject variant | Extend `PermissionScope` / `PermissionSubject`. Update `evaluate` + `add_rule` routing. |
| A new named-agent management surface | Wire onto `LiveSession::{put_ephemeral_agent, delete_agent, promote_ephemeral_agent, agent_names, agent_exists, resolve_agent_config}`. |
| A new project field | Extend `ProjectConfig` with `#[serde(default)]` so legacy files keep loading. |

Three rules that have proven load-bearing:

1. **One runner path.** Everything background must go through
   `execute_agent_run`. Do not spawn a second parallel query path — you will
   miss `task_history` recording, tracker registration, event log emissions,
   and Kairos bootstrap.
2. **Gate first, then act.** Any new feature must consult
   `is_kairos_*_active()` at the top of its spawn site, like cron and
   proactive already do. No feature turns itself on unilaterally.
3. **Tracker + log are observable singletons.** Producer sites push
   events; tracker entries surface in `/activity` via lifecycle hooks (when
   wired). Single producer per event — do NOT push from both the spawn site
   and the tracker.

---

## 12. Testing Procedures

### T1 — Gate activation
```bash
KAIROS=1 KAIROS_TRUST_BYPASS=1 cargo run --features kairos_brief
```
Run `/kairos` — `brief_enabled` should be `true`, diagnostics should show
`env KAIROS=yes trust_bypass=yes`.

### T2 — Brief
Ask the model to call `Brief` with a short message. Expect a tool-result
block in the current turn.

### T3 — Recurring cron
Ask the model to `CronCreate { cron: "* * * * *", prompt: "say hi",
recurring: true, durable: false }`. Within ≤60s: toast + injected system
message labelled `cron:<id>`. `/activity --source cron` shows `CronFired`
+ `BackgroundStart`/`Finish`.

### T4 — Durable persistence
Same but with `durable: true`. Exit Claurst, re-enter, run `/kairos`. Task
still listed.

### T5 — BackgroundSafe slash
`/btw what time is it` — immediate "Queued" toast, then a completion toast
and injected message later. `/tasks` shows the in-flight `Agent` task.

### T6 — Proactive cost guardrail
```
KAIROS=1 KAIROS_TRUST_BYPASS=1 KAIROS_PROACTIVE=1 \
KAIROS_PROACTIVE_INTERVAL_SECS=60 KAIROS_TICK_MAX_USD=0.0001 \
cargo run --features kairos_brief
```
After two ticks the ticker exits; grep logs for `"repeated cost overruns,
stopping"`.

### T7 — Bridge auto-resume
Start Claurst in directory X, send one message, exit. Re-enter from X with
no `--resume` flag — prior session loads.

### T8 — Project switch
`/project create alpha --root /tmp/alpha`, `/project switch alpha` — cwd
flips to `/tmp/alpha`; `/project list` shows the active marker.

### T9 — Tasks + stop all
Trigger a `/btw` long task, then `/tasks` to see it, `/stop all --yes` to
cancel.

### T10 — Activity log
Run any background work, then `/activity 10`. Verify mixed entries (Tool
Call, BackgroundStart/Finish, PermissionRequested if any). After exit,
inspect `~/.claurst/event_log.jsonl`.

### T11 — Unit suites
```bash
cargo test --features kairos_brief -p claurst-core --lib
cargo test --features kairos_brief -p claurst-query --tests   # phase 8/8.5/9/9.5/10/11 smoke
cargo test --features kairos_brief -p claurst-commands --tests
cargo test --features kairos_brief -p claurst-tools --lib cron stop_all_tasks
```

---

## 13. Known Gaps (Snapshot)

Round 1 plumbing carry-overs:
- `kairos_channels` feature flag exists but has no implementation.
- No slash command to pause/resume individual cron jobs or the whole scheduler (delete-only).
- No transcript segmentation + lazy history load for long sessions.
- Prompt dedupe across ticks is not implemented (prompt is static — not meaningful yet).

Round 2 deferred items (see KAIROS_FUTURE.md "Deferred Within Round 2" for full status):
- `/project switch` MCP swap (atomic-replace protocol) not yet implemented; cwd + active marker + permission rules done.
- Status-line wiring: welcome-screen "Recent activity" line is now live (App.recent_activity ← event_log.most_recent each tick); continuous below-prompt avatar surfacing still pending.
- Tracker → event-log lifecycle hooks (auto-emit `BackgroundStart`/`Finish` from tracker register/deregister) — currently per-spawn-site emission; safe but could be centralised.
- Tool-dispatch + `AgentTool` sub-agent not yet registered as `TaskTracker` entries (they emit `ToolCall` events but no tracker rows).
- `/activity` is a text-rendering slash command — scrollable modal still pending.

See `KAIROS_FUTURE.md` for the prioritized backlog and phase log.
