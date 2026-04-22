# Kairos Mode Future Implementation Plan

## High-Level System Design

Kairos should evolve from a set of always-on tools into a gated assistant mode that can:
- activate only for entitled and trusted contexts,
- run proactive and background work safely,
- persist and resume sessions across restarts,
- surface useful results to users in the terminal UI.

The target model follows a layered design.

```text
+-------------------------------------------------------------+
|                    Kairos Assistant Mode                    |
+-------------------------------------------------------------+
| 1) Activation and Entitlement Layer                         |
|    - Build-time features                                    |
|    - Runtime env/config checks                              |
|    - Entitlement gate (GrowthBook-backed)                   |
|    - Workspace trust checks                                 |
+-------------------------------------------------------------+
| 2) Assistant Bootstrap Layer                                |
|    - Brief mode enforcement                                 |
|    - Team pre-seeding                                       |
|    - Kairos system prompt addendum                          |
+-------------------------------------------------------------+
| 3) Execution Layer                                          |
|    - Background slash-command execution                     |
|    - MCP settlement wait                                    |
|    - Result re-queue into message stream                    |
+-------------------------------------------------------------+
| 4) Proactive Autonomy Layer                                 |
|    - Tick wakeups                                            |
|    - Focus-aware behavior                                   |
|    - Sleep pacing                                            |
+-------------------------------------------------------------+
| 5) Session Continuity Layer                                 |
|    - Perpetual bridge sessions                              |
|    - Pointer-based recovery                                 |
|    - Session discovery and resume                           |
+-------------------------------------------------------------+
| 6) Persistence and History Layer                            |
|    - Transcript segment writes                              |
|    - Lazy history load                                      |
|    - Durable cron and session metadata                      |
+-------------------------------------------------------------+
| 7) UX and Operations Layer                                  |
|    - TUI visibility for background results                  |
|    - Health/status commands                                 |
|    - Configurable limits and retention                      |
+-------------------------------------------------------------+
```

## Current Baseline (Verified)

Status after code and web cross-check:
- Implemented: Brief tool, CronCreate, CronDelete, CronList, cron scheduler startup.
- Missing behavior: runtime Kairos activation flow, assistant-mode bootstrap, async slash background flow, proactive tick loop, session bridge persistence and resume UX.
- Important correction: GrowthBook integration code exists in core, but Kairos activation currently does not wire it in.

## Feature List to Make Kairos Fully Functional

The following list is ordered by dependency and implementation priority.

### 1. Activation and Gate Enforcement

Goal: make Kairos optional, explicit, and safe.

- Enforce compile-time feature gates around Kairos tools and related wiring.
- Add runtime activation policy that combines:
  - build feature availability,
  - environment or config opt-in,
  - entitlement check,
  - workspace trust acceptance.
- Add single app-state source of truth for `kairos_enabled`.
- Ensure all downstream behavior checks app-state instead of ad hoc flags.

Implementation notes:
- Use existing core feature flag infrastructure (including GrowthBook manager) as entitlement backend.
- Add deterministic fallback behavior when remote flag checks are unavailable.

### 2. Assistant Mode Bootstrap

Goal: normalize assistant behavior immediately after activation.

- Force brief-mode conversational style while Kairos is active.
- Pre-seed assistant/team context to allow agent workflows without manual setup friction.
- Inject Kairos-specific prompt addendum for autonomous and background behavior.
- Ensure bootstrap runs only once per session startup path.

### 3. Background Task Execution Pipeline

Goal: allow slash-command style operations to run asynchronously without blocking input.

- Add fire-and-forget execution path under Kairos mode.
- Wait for MCP/tool-settlement before starting background task execution.
- Re-queue completion output as message-queue notifications.
- Preserve ordering metadata (start time, completion time, source command).
- Add retries or bounded failure handling for transient tool-unavailable cases.

### 4. Proactive Autonomous Loop

Goal: support controlled autonomy between user turns.

- Add periodic tick prompts when proactive mode is enabled.
- Add focus-aware policy:
  - terminal focused: concise, collaborative behavior,
  - terminal unfocused: more autonomous execution.
- Support Sleep tool pacing in all modes, with stricter proactive-mode usage guidance.
- Prevent unnecessary wakeups when no useful work is available.

### 5. Perpetual Session Bridge and Resume

Goal: preserve assistant continuity across restarts.

- Add bridge pointer file writes on active session updates.
- On startup, discover valid pointer(s) and offer resume flow.
- Support worktree-aware discovery and recovery.
- Keep bridge session alive on clean exit when Kairos is active.
- Handle stale pointers and corrupted metadata safely.

### 6. Enhanced Session Persistence

Goal: durable and recoverable context with responsive UX.

- Write transcript segments during compaction/checkpoints.
- Implement lazy history loading to avoid startup blocking.
- Keep metadata indexes for fast lookup and partial restore.
- Configure transcript retention and cleanup policy.

### 7. Cron System Completion

Goal: upgrade cron from background-only execution to observable, configurable operations.

- Keep durable task persistence behavior.
- Add visible task run history and last result in TUI.
- Add controls to pause/resume scheduler and inspect health.
- Replace hard-coded job cap with config-driven limit.
- Keep strict cron validation and bounded task execution policy.

### 8. CLI and TUI UX Surface

Goal: make Kairos operationally usable from terminal workflows.

- Add user-facing commands for:
  - mode status,
  - scheduler status,
  - session resume/discovery,
  - proactive mode state.
- Add clear event surfaces for background completion notices.
- Add diagnostics output for gate decisions (why enabled or disabled).

### 9. Safety, Policy, and Failure Handling

Goal: keep autonomy predictable and debuggable.

- Add explicit fail-safe behavior when entitlement checks fail.
- Add telemetry/logging fields for activation path and background lifecycle.
- Add guardrails on autonomous loop frequency and concurrency.
- Add bounded queue sizes and backpressure behavior.

### 10. Testing and Verification

Goal: prove feature parity and avoid regressions.

- Unit tests:
  - gate evaluation,
  - pointer serialization/deserialization,
  - cron validation and limits,
  - background queue transitions.
- Integration tests:
  - restart and session resume,
  - parallel background commands,
  - proactive tick and sleep pacing,
  - entitlement on/off transitions.
- Manual tests:
  - Kairos disabled path remains unchanged,
  - Kairos enabled path exposes expected capabilities,
  - TUI surfaces asynchronous outcomes clearly.

## Phased Implementation Roadmap

### Phase 1 - Gate Foundation
- Deliver activation policy and app-state propagation.
- Wire existing entitlement manager into startup flow.
- Gate tool registration and runtime usage.

Exit criteria:
- Kairos is disabled by default.
- Kairos can be enabled only when policy conditions pass.

### Phase 2 - Assistant Bootstrap
- Add brief enforcement, team pre-seeding, prompt addendum.

Exit criteria:
- Kairos sessions always start in consistent assistant profile.

### Phase 3 - Async Background Execution
- Add detached command execution, settlement wait, result re-queue.

Exit criteria:
- long-running slash-like tasks do not block user input.

### Phase 4 - Proactive Loop
- Add tick wakeups, focus policy, sleep pacing behavior.

Exit criteria:
- proactive mode performs useful autonomous cycles safely.

### Phase 5 - Session Continuity
- Add bridge pointer persistence, discovery, and resume UX.
- Add worktree-aware pointer scanning.

Exit criteria:
- active assistant sessions can survive and resume across restarts.

### Phase 6 - Persistence and Cron UX
- Add transcript segments and lazy history load.
- Add visible cron run history and scheduler controls.
- Make limits configurable.

Exit criteria:
- durable context and scheduler behavior are user-visible and operable.

### Phase 7 - Hardening and Docs
- Add complete test matrix and operational diagnostics.
- Finalize docs and migration notes.

Exit criteria:
- feature-complete Kairos with clear operator workflow.

## Suggested Initial File Targets

- `src-rust/crates/tools/src/lib.rs`
- `src-rust/crates/tools/src/cron.rs`
- `src-rust/crates/query/src/cron_scheduler.rs`
- `src-rust/crates/cli/src/main.rs`
- `src-rust/crates/core/src/feature_flags.rs`
- `src-rust/crates/core/src/session_storage.rs`

## Definition of Done

Kairos is considered fully functional when all conditions below are true:
- Activation is gated by compile-time and runtime policy checks.
- Assistant bootstrap behavior is consistent and repeatable.
- Background tasks run asynchronously and report back in UI.
- Proactive loop and sleep pacing operate under clear constraints.
- Sessions resume across restarts using pointer-based recovery.
- Transcript/history persistence is durable and lazily retrievable.
- Cron scheduler is observable, controllable, and configurable.
- Non-Kairos user workflows remain unchanged.

## Implementation Progress Log

### 2026-04-16 — Phase 1 (Gate Foundation) Completed

- Added centralized Kairos runtime gate state in `src-rust/crates/core/src/kairos_gate.rs`.
- Added strict runtime-state initialization contract (no fallback reads after startup).
- Initialized runtime state in both startup paths in `src-rust/crates/cli/src/main.rs`.
- Propagated compile-time Kairos feature flags through Cargo manifests.

### 2026-04-16 — Phase 2 (Assistant Bootstrap) Completed

- Added `apply_kairos_bootstrap_to_query_config(...)` in `src-rust/crates/cli/src/main.rs`.
- Enforced concise output mode when Kairos brief is active.
- Appended assistant-mode addendum from `claurst_core::kairos_gate::assistant_system_prompt_addendum(...)`.
- Added `kairos_enabled` session flag to query config in `src-rust/crates/query/src/lib.rs`.

### 2026-04-16 — Phase 3.0 (Async Background Slice) Completed

- Added detached background execution path for `/btw` in interactive CLI loop.
- Added MCP settlement wait prior to detached execution.
- Added background completion reinjection via command queue and TUI notification.

### 2026-04-16 — Phase 3.1 (Policy + Shared Runner Foundation) Completed

- Added `CommandExecutionPolicy` in `src-rust/crates/commands/src/lib.rs`.
- Added default `SlashCommand::execution_policy()` with foreground default.
- Marked `/btw` as `BackgroundSafe` and added policy lookup test coverage.
- Replaced hardcoded `/btw` branch in CLI with policy-based background routing.
- Extracted reusable shared helper `spawn_background_slash_command(...)` in `src-rust/crates/cli/src/main.rs`.

### 2026-04-20 — Phase 3.2 (Unified Background Runner) Completed

- Created `src-rust/crates/query/src/background_runner.rs` with `AgentRunSource`, `AgentRunRequest`, `AgentRunContext`, `AgentRunResult`, `execute_agent_run`, `spawn_agent_run`.
- Removed `spawn_background_agent_task` from `query/src/lib.rs` (was a duplicate with different parameter bags).
- Removed redundant query-config field reassignment from the slash-command background path (`model`, `max_tokens`, `output_style`, `output_style_prompt` were already set in the main `query_config`).
- Fixed double-bootstrap bug: Kairos system prompt addendum was being appended twice for background tasks. Now applied once at startup; `execute_agent_run` does not re-apply.
- Inlined `run_scheduler_loop` into `start_cron_scheduler` (private passthrough function with one caller).
- Channel type changed from `(String, String, bool)` tuple to `AgentRunResult` (structured, source-tagged).
- Renamed `bg_slash_tx/rx` → `bg_task_tx/rx` to reflect that cron and proactive results will flow through the same channel.
- Silent background commands no longer generate a spurious "completed with no output" notification.
- Removed stale phase-tracking comment from interactive loop.

### 2026-04-21 — Phase 3.3 (Task Run Records) Completed

- Created `src-rust/crates/query/src/task_history.rs`: `TaskRunRecord`, `RunStatus`, global in-memory ring buffer (100 records, `Lazy<Mutex<VecDeque>>`), `record_run`, `last_runs`.
- Disk persistence: JSONL append to `~/.claurst/kairos_run_history.jsonl` on each completion.
- Wired `record_run` into `execute_agent_run` in `background_runner.rs`: records start time, source label, prompt preview, output snippet, status.
- In-memory buffer updated even when disk write fails (errors logged, not swallowed).
- `last_runs(n)` exposed from `claurst-query` crate for future use in `/kairos status` command.
- Simplification pass: removed unnecessary `Arc` wrapper from global history static; separated disk append into `append_to_disk` helper; silent `create_dir_all` error now logged and exits early.

### 2026-04-21 — Phase 4 (Proactive Autonomous Loop) Completed

- Added `proactive_interval_secs()` (reads `KAIROS_PROACTIVE_INTERVAL_SECS`, default 900, clamped [60, 3600]) and `proactive_tick_prompt()` to `crates/core/src/kairos_gate.rs`.
- Created `crates/query/src/proactive_ticker.rs`: `start_proactive_ticker()` runs a sequential tick loop — sleep → execute → record → repeat. No concurrent proactive tasks possible by design.
- Backoff: `MAX_CONSECUTIVE_ERRORS` (3) consecutive errors doubles the sleep interval; resets to base on success. Warns once at threshold.
- `execute_agent_run` return type changed `() → bool` (is_error) so ticker can track outcomes.
- Proactive ticker started in `run_interactive` after `bg_task_tx` creation; results flow through same `AgentRunResult` channel as slash commands and cron. Cancelled at session end via `proactive_cancel`.
- Existing drain loop in `run_interactive` already handles `AgentRunSource::Proactive` label — no TUI changes needed.
- Simplification pass: removed redundant startup log (ticker logs itself with interval).

### 2026-04-21 — Phase 5: Session Bridge + Resume Completed

- `core/src/session_bridge.rs`: `BridgePointer`, `upsert_bridge_pointer`, `find_active_pointer`, `cleanup_stale_pointers`.
- Bridge files at `~/.claurst/bridge/{session_id}.json`, TTL-gated (env `KAIROS_BRIDGE_TTL_SECS`, default 4h).
- Auto-resume on startup: scans CWD-matching live pointers, merges with `--resume` flag (explicit wins).
- Pointer written after each session save with 30s debounce. Stale cleanup runs concurrently at startup.

### 2026-04-21 — Phase 6: Cron UX + Config Completed

- `cron_scheduler::start_cron_scheduler` now accepts `result_tx: Option<mpsc::UnboundedSender<AgentRunResult>>` and passes it into every `AgentRunContext`. Cron output merges into the same TUI drain loop as proactive ticks and background slash commands.
- Cron scheduler start moved from outer `main()` into `run_interactive` so it shares the interactive `bg_task_tx` directly; headless/--print mode no longer starts a minute-ticker it would never use. Cancelled via a local `CancellationToken` on loop exit.
- Promoted `task_history` from `claurst-query` to `claurst-core`. `claurst-tools` now depends on it without pulling a cycle; `claurst-query` keeps `pub use claurst_core::task_history::*` so existing call sites stay working.
- Added `claurst_core::task_history::last_runs_by_cron_id(scan_limit)` — returns `HashMap<task_id, TaskRunRecord>` of most recent run per cron id. Single helper used by `CronList` and `/kairos`.
- `CronListTool` output now shows `last_run=<UTC ts> (ok|err)` or `last_run=never` per row. `execute` body collapsed to call new `cron::list_tasks()` snapshot helper instead of inlining the read/sort.
- Cron job cap now reads `KAIROS_MAX_CRON_JOBS` (default 50, clamped [1, 1000]); error message names the env var so users can raise it.
- `/kairos` slash command added (`commands/src/lib.rs`): prints gate state, active cron jobs with last_run, and N most recent background run records. Optional numeric arg (default 10, clamped [1, 100]). Registered in `all_commands()` and under "System" category.
- Simplification pass: extracted `cron::list_tasks()` so `CronListTool` and `/kairos` share the snapshot/sort logic; extracted `last_runs_by_cron_id` so both call sites share the cron-id indexing (removed two near-identical loops).

### 2026-04-21 — Phase 7: Hardening Completed

- **Unit tests** (16 new, all green):
  - `crates/tools/src/cron.rs` — 9 tests covering `validate_cron` (accept/reject paths) and `cron_matches` (every-minute, step, specific, range, list, Sunday alias 7≡0, wrong field count).
  - `crates/core/src/session_bridge.rs` — 5 tests: `BridgePointer` JSON round-trip, `is_stale` past vs. recent, `bridge_ttl_secs` clamps below 60s, `matches_dir` exact equality.
  - `crates/core/src/task_history.rs` — 2 tests: `TaskRunRecord` JSON round-trip, `last_runs_by_cron_id` filters non-cron sources and buckets by id (newest wins per id).
- **Gate decision diagnostics**: introduced `KairosGateDiagnostics` stored on `KairosRuntimeState` at `resolve_runtime_state` time. Captures compile features, each env var value, trust + bypass, forced / require_entitlement / entitlement_ok. Struct carries its own `format_summary()` so `/kairos` renders inputs verbatim without recomputing state mid-session.
- **Proactive loop cost guardrail**: added `kairos_gate::proactive_tick_max_usd()` (reads `KAIROS_TICK_MAX_USD`; unset/non-positive = `None` = no ceiling). `proactive_ticker` now snapshots `total_cost_usd` around each tick; a single-tick delta over the ceiling counts as an overrun. After `MAX_COST_OVERRUNS = 2` consecutive overruns the ticker logs a warning and exits. Successful under-ceiling tick resets the strike counter.
- **Scope cuts** (not implemented, documented as deferred):
  - Integration tests (restart+resume, parallel background, entitlement transitions) — require a full E2E harness; separate initiative.
  - Prompt dedupe across ticks — the proactive prompt is static, so dedupe would suppress every tick; not meaningful.
  - Cron scheduler pause/resume/inspect slash commands — `/kairos` + `CronDelete` cover current needs; can be added on demand.
  - Transcript segment writes + lazy history load — still a carry-over candidate but distinct from Kairos hardening; track separately.
- **Simplification pass**: diagnostics rendering lives on the struct (`format_summary`) so `/kairos` gains one call, not a reimplementation. Cost ceiling returns `Option<f64>` so "unset" and "invalid" collapse to the same branch.

### Kairos Status

All roadmap phases (1 through 7) are complete for the "brief + proactive + cron + bridge" slice. Remaining work is outside the Phase 1–7 scope:
- `kairos_channels` feature — flag only, no implementation. Distinct feature.
- Integration test harness — infra task.
- Focus-aware behavior — deferred product decision.
- Transcript segment writes / lazy history — persistence layer work, orthogonal to Kairos.

Kairos meets the Definition of Done criteria for: gated activation, consistent assistant bootstrap, async background tasks with TUI visibility, proactive loop with pacing and cost guardrails, pointer-based session recovery, observable + configurable cron, and non-Kairos paths unchanged.
