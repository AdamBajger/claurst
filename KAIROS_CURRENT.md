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
(session bridge). Every one of these features sits behind the same gate and
funnels through the same background runner.

---

## 2. Architectural Layers (Responsibility Breakdown)

```
┌─────────────────────────────────────────────────────────────────────────┐
│ TUI loop (crates/cli/src/main.rs)                                       │
│   • startup initialization, bootstrap, cron+ticker spawn, drain loop    │
│   • bridge pointer upsert, cron/proactive cancel on exit                │
└────────────────┬──────────────────────────────────────┬─────────────────┘
                 │                                      │
                 │ bg_task_tx (AgentRunResult)          │ command_queue
                 ▼                                      ▼
┌─────────────────────────────────────────────────────────────────────────┐
│ Background runner (crates/query/src/background_runner.rs)               │
│   execute_agent_run(request, context) → bool(is_error)                  │
│   │ ┌ runs query loop, records to task_history                          │
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
┌─┴───────────────────────┐   ┌────────────────────────────────────────┐
│ Feature flag compile    │   │ Session bridge (core/session_bridge.rs)│
│ gate (core/tools/tui    │   │   BridgePointer, TTL, upsert/find/clean│
│ /cli Cargo.toml)        │   └────────────────────────────────────────┘
└─────────────────────────┘
```

Roughly: each layer below only knows about the layers below it. The gate has
no knowledge of the TUI; the runner has no knowledge of cron vs. proactive vs.
`/btw` beyond an `AgentRunSource` enum.

---

## 3. Key Structs and Where They Live

| Struct / enum | Crate::module | What it does |
|---|---|---|
| `KairosRuntimeState` | `core::kairos_gate` | Frozen snapshot of brief/channels/proactive flags + entitlement + diagnostics. Set once at startup. |
| `KairosGateDiagnostics` | `core::kairos_gate` | Every input that fed the gate decision (compile flag, env vars, trust, bypass, entitlement). Rendered by `/kairos`. |
| `BridgePointer` | `core::session_bridge` | `{session_id, working_dir, started_at, last_active_at}` — file at `~/.claurst/bridge/{session_id}.json`. |
| `TaskRunRecord` / `RunStatus` | `core::task_history` | One record per background run. Stored in ring buffer + appended to JSONL. |
| `AgentRunRequest` | `query::background_runner` | `{run_id, source, prompt}` — input to the runner. |
| `AgentRunContext` | `query::background_runner` | Shared deps: `query_config`, `tool_ctx`, `client`, `tools`, `result_tx`. |
| `AgentRunResult` | `query::background_runner` | `{run_id, source, output, is_error}` — flows to TUI via `bg_task_tx`. |
| `AgentRunSource` | `query::background_runner` | `SlashCommand{name} | Cron{task_id} | Proactive`. Label-only; runner treats all sources identically. |
| `QueryConfig` (`.kairos_enabled`) | `query::lib` | Carries Kairos system-prompt addendum + concise-mode flag into each turn. |
| `CommandExecutionPolicy` | `commands::lib` | `BackgroundSafe | ForegroundOnly` — per-slash-command flag consulted by the TUI loop. |
| `CronTask` + on-disk store | `tools::cron` | Serde struct for scheduled prompts; persisted to `~/.claurst/cron.json` when `durable=true`. |

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
| Bridge write debounce | TUI drain loop, after each `save_session`. At most one disk write per 30s. | `crates/cli/src/main.rs:3020` |
| MCP settlement wait | Before `/btw` background execution. 5s poll so tools resolve before a fresh query starts. | `crates/cli/src/main.rs` (`spawn_background_slash_command`) |

---

## 5. Startup Sequence (Entry Points)

All line numbers reference `crates/cli/src/main.rs` at the time of writing.

```
main()
├─ named-command path (headless)
│   └─ line 484: initialize_runtime_state(has_completed_onboarding).await
│                → sets RUNTIME_STATE, returns KairosRuntimeState
│
└─ run_interactive()
    ├─ line 631: initialize_runtime_state(...).await           ← gate resolved here
    ├─ line 820: apply_kairos_bootstrap_to_query_config(...)   ← mutates base QueryConfig
    ├─ line 1642: bg_task_tx/rx created (unbounded mpsc)
    ├─ line 1650–1662: cron_cancel token + start_cron_scheduler(..., Some(bg_task_tx))
    ├─ line 1664–1674: proactive_cancel token + start_proactive_ticker(...) if proactive active
    ├─ line 1850–1860: /btw & other BackgroundSafe slash commands routed to spawn_background_slash_command
    ├─ line 2249: apply_kairos_bootstrap_to_query_config re-applied per turn (new qcfg)
    ├─ line 2968–2986: drain loop — bg_task_rx.try_recv() → command_queue + notification + push_message
    ├─ line 3018–3029: bridge pointer debounced upsert (30s) after each save_session
    └─ line 3089–3090: proactive_cancel.cancel(); cron_cancel.cancel() on exit
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
                                                        → AgentRunResult → bg_task_tx

Cron tick fires      Toast on arrival:                cron_scheduler (tokio task)
(minute granularity)  "Background cron:<id> finished."   → pop_due_tasks
                     System message injected into       → execute_agent_run(Cron{id})
                     conversation via command_queue.    → AgentRunResult → bg_task_tx

Proactive tick       Toast + system message.           proactive_ticker
                     Label reads "kairos".              → execute_agent_run(Proactive)
                                                        → AgentRunResult → bg_task_tx

Model calls Brief    Toast + message. Model decides   BriefTool (permission_level=None)
                     wording.                           → pushes ToolResult the TUI renders

/kairos [N]          Rendered gate state,             KairosCommand (foreground, synchronous)
                     diagnostics, cron list, last N     → reads runtime_state, list_tasks,
                     run records.                         last_runs[_by_cron_id]
```

**Drain loop pattern** (`crates/cli/src/main.rs:2968`): every frame, the loop
drains `bg_task_rx` non-blocking and for each `AgentRunResult` does three
things in order:
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
- **Triggers:** model calls a tool; user has no direct slash command for
  create/delete (by design — they go through the model turn).
- **What the user sees:** tool result confirming the task ID; later, when the
  task fires, a toast `"Background cron:<id> finished."` plus a system
  message injected into the conversation. `/kairos` lists active tasks with
  their last run timestamp and status.
- **Persistence:** `durable=true` writes to `~/.claurst/cron.json`; reload on
  startup.
- **Code:** `crates/tools/src/cron.rs`, `crates/query/src/cron_scheduler.rs`.

### 7.3 Background slash commands (`/btw`, etc.)
- **Trigger:** user types a slash command whose `execution_policy()` returns
  `BackgroundSafe`.
- **What the user sees:** immediate toast, TUI free to accept new input, later
  toast + system message when complete.
- **Code:** `crates/cli/src/main.rs::spawn_background_slash_command`,
  `crates/commands/src/lib.rs` (policy definition).

### 7.4 Proactive mode (`KAIROS_PROACTIVE=1`)
- **Trigger:** timer in the ticker (every `KAIROS_PROACTIVE_INTERVAL_SECS`).
- **Prompt sent to model:** `proactive_tick_prompt()` (defined in
  `kairos_gate.rs` — this is the *only* prompt text and it is static).
- **What the user sees:** same pattern as cron — toast + injected system
  message, label `kairos`.
- **Guardrails visible to user:** if repeated failures double the interval, a
  single warn! log appears. If `KAIROS_TICK_MAX_USD` overruns twice, the
  ticker exits silently to the log; no new Kairos ticks until restart.

### 7.5 Session bridge (auto-resume)
- **Trigger:** startup with no explicit `--resume` flag.
- **What the user sees:** the last active session for the current working
  directory is loaded automatically. Explicit `--resume` wins over
  auto-discovered pointer (`cli.resume.or(bridge_resume_id)`).
- **Code:** `crates/core/src/session_bridge.rs`, invoked in `run_interactive`.

### 7.6 `/kairos` status
- **Trigger:** user slash command.
- **What the user sees:** multi-line status block with:
  - Gate flags (brief/channels/proactive/entitled)
  - Frozen diagnostics (compile flags, env vars, trust/bypass, entitlement)
  - Cron jobs with last_run per task
  - Last N run records (default 10, clamp [1, 100] via `/kairos <N>`)
- **Code:** `crates/commands/src/lib.rs::KairosCommand`.

---

## 8. System Prompts — Where They Live

There are exactly two prompt strings under Kairos control, both in
`crates/core/src/kairos_gate.rs`:

| Function | Purpose |
|---|---|
| `assistant_system_prompt_addendum(proactive_enabled: bool)` | Appended to the base system prompt by `apply_kairos_bootstrap_to_query_config`. Tells the model Kairos is on, to be terse, to use `Brief` for notifications, and (when proactive) to pace autonomous work. |
| `proactive_tick_prompt()` | User-turn message sent on each proactive tick. Static. |

That's it. Brief, cron, and background slash output are delivered to the
model as plain system/user messages (see drain loop); they do not introduce
new prompt templates.

---

## 9. Configuration — ENV Only (Design Choice)

Kairos deliberately does **not** use `settings.json`. Rationale: gate state
must be resolvable at the earliest possible startup point (before settings
loading completes in some code paths) and must be auditable from the shell
that launched the binary. Treat this as intentional — if you add a knob,
add it as an env var and document it below. Settings.json integration is
out of scope until a concrete need arises.

Priority: ENV > compile-time feature flag. If the feature is not compiled,
env vars do nothing.

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
| `KAIROS_TICK_MAX_USD` | unset | Per-tick cost ceiling. `None` if unset / invalid / non-positive (no ceiling). Two consecutive overruns stop the ticker. | `proactive_tick_max_usd` |
| `KAIROS_MAX_CRON_JOBS` | 50 | Clamped to [1, 1000]. | `tools::cron::max_cron_jobs` |
| `KAIROS_BRIDGE_TTL_SECS` | 14400 (4h) | Min 60s. Pointers older than this are stale. | `core::session_bridge::bridge_ttl_secs` |

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
```

---

## 11. Extension Points — Where to Hook New Work

| You want to add… | Extend here |
|---|---|
| A new autonomous driver (new kind of background task) | New caller of `execute_agent_run`, add a variant to `AgentRunSource`. |
| A new Kairos-only tool | Add the struct in `crates/tools/src/`, register it inside the `if kairos_brief_tools_enabled()` block in `tools::lib::all_tools`. |
| A new background-safe slash command | Override `execution_policy()` in `crates/commands/src/lib.rs` to return `BackgroundSafe`. |
| A new gate input | Extend `KairosGateDiagnostics` and the logic in `resolve_runtime_state`. Update `format_summary`. |
| A new persisted Kairos artifact | Prefer `~/.claurst/...` and write a round-trip test next to the serde struct (see `session_bridge.rs` tests as template). |
| New recorded metrics on a background run | Extend `TaskRunRecord` in `core::task_history`; `execute_agent_run` is the single write site. |
| A new env knob | Add parser in `core::kairos_gate` (or `session_bridge`/`cron` for domain-specific knobs). Document in §9 above. |

Two rules that have proven load-bearing:

1. **One runner path.** Everything background must go through
   `execute_agent_run`. Do not spawn a second parallel query path — you will
   miss `task_history` recording, `AgentRunResult` plumbing, and Kairos
   bootstrap.
2. **Gate first, then act.** Any new feature must consult
   `is_kairos_*_active()` at the top of its spawn site, like cron and
   proactive already do. No feature turns itself on unilaterally.

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
block in the current turn; no additional approval prompt.

### T3 — Recurring cron
Ask the model to `CronCreate { cron: "* * * * *", prompt: "say hi",
recurring: true, durable: false }`. Within ≤60s: toast + injected system
message labelled `cron:<id>`.

### T4 — Durable persistence
Same but with `durable: true`. Exit Claurst, re-enter, run `/kairos`. Task
still listed.

### T5 — BackgroundSafe slash
`/btw what time is it` — immediate "Queued" toast, then a completion toast
and injected message later.

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

### T8 — Unit suites
```bash
cargo test -p claurst-core  --features kairos_brief --lib
cargo test -p claurst-tools --features kairos_brief --lib cron
```

---

## 13. Known Gaps (Snapshot)

- `kairos_channels` feature flag exists but has no implementation.
- No slash command to pause/resume individual cron jobs or the whole scheduler (delete-only).
- No transcript segmentation + lazy history load for long sessions.
- No integration-test harness exercising restart+resume or parallel background fan-out.
- Prompt dedupe across ticks is not implemented (prompt is static — not meaningful yet).

See `KAIROS_FUTURE.md` for the prioritized backlog and phase log.
