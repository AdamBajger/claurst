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

### Next Planned Increment

- Promote shared background helper into generic AgentRunRequest-style runner interface.
- Wire cron-triggered execution to the shared runner substrate.
- Add persisted run/session records for inspection and reporting.
