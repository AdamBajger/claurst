# Kairos Mode — Comprehensive Guide

## What is Kairos?

Kairos is an **experimental/advanced mode** in Claurst that enables:
- **Proactive communication** via the `Brief` tool
- **Scheduled tasks** via `CronCreate`, `CronDelete`, `CronList` tools
- **Channel-based messaging** (planned, not implemented)

---

## Current Implementation Status

### ✅ Implemented
| Component | Status | Location |
|-----------|--------|----------|
| `Brief` tool | Fully implemented | `crates/tools/src/brief.rs` |
| `CronCreate` tool | Fully implemented | `crates/tools/src/cron.rs` |
| `CronDelete` tool | Fully implemented | `crates/tools/src/cron.rs` |
| `CronList` tool | Fully implemented | `crates/tools/src/cron.rs` |
| Cron scheduler (background) | Fully implemented | `crates/query/src/cron_scheduler.rs` |
| Feature flags (`kairos_brief`, `kairos_channels`) | Defined | `crates/core/Cargo.toml` |

### ❌ Missing / Not Implemented
| Component | Status | Notes |
|-----------|--------|-------|
| `kairos_brief` feature gate enforcement | **NOT ENFORCED** | Brief tool is always available regardless of feature flag |
| `kairos_channels` feature | **EMPTY** | Flag exists but no implementation |
| TUI integration for Kairos | **MISSING** | No special UI rendering for Kairos mode |
| Environment variable detection (`KAIROS`, `KAIROS_BRIEF`) | **MISSING** | No runtime feature detection |
| GrowthBook/feature gate integration | **MISSING** | No external feature flag system |
| `/brief` command (CLI toggle) | **MISSING** | Spec mentions but not implemented |
| `assistant` command | **MISSING** | Spec mentions but not implemented |
| `subscribe-pr` command (GitHub webhooks) | **MISSING** | Spec mentions but not implemented |
| `proactive` command | **MISSING** | Spec mentions but not implemented |

---

## Build Configuration

### Feature Flags Defined
```toml
# crates/core/Cargo.toml
[kairos_brief]  # Enables Brief tool (but not enforced)
[kairos_channels]  # Planned, no implementation
```

### Dependency Chain
```
claurst-tui
  └── kairos_brief → claurst-core/kairos_brief
  └── kairos_channels → claurst-core/kairos_channels

claurst-tools
  └── Always includes BriefTool, Cron*Tool (no feature gating)
```

---

## The Problem: `--all-features` is NOT Enough

Building with `cargo build --all-features` **does compile** the code, but:

1. **No runtime activation**: There's no code path that checks `cfg!(feature = "kairos_brief")` to conditionally enable/disable functionality
2. **Tools always registered**: `BriefTool` and `Cron*Tool` are registered in `crates/tools/src/lib.rs` unconditionally
3. **No CLI commands**: No `/brief`, `/kairos`, or similar commands exist in the TUI
4. **No environment hooks**: No `std::env::var("KAIROS")` checks anywhere

**Result**: Kairos features are **always on** once compiled, regardless of build flags or environment variables.

---

## How to Enable & Use (Current State)

### Step 1: Build
```bash
cd src-rust
cargo build --all-features
# OR explicitly:
cargo build --features "kairos_brief,kairos_channels"
```

### Step 2: Run
```bash
./target/debug/claurst
# or
./target/release/claurst
```

### Step 3: Use Available Tools
Once running, these tools are **always available**:

#### Brief Tool
```rust
// Called by the model to notify the user
Brief {
    message: "Task complete",
    status: "proactive",  // or "normal"
    attachments: ["file.txt"]  // optional
}
```

#### Cron Tools
```rust
// Schedule a recurring task
CronCreate {
    cron: "*/5 * * * *",
    prompt: "Check for new emails",
    recurring: true,
    durable: true  // persists across sessions
}

// List scheduled tasks
CronList {}

// Delete a task
CronDelete { id: "abc123" }
```

---

## Cron Expression Format

```
M H DoM Mon DoW  (5 fields, local time)

Examples:
*/5 * * * *     → Every 5 minutes
30 14 * * 1     → Every Monday at 14:30
0 9 15 * *      → 15th of each month at 09:00
* * * * *       → Every minute
```

---

## Known Issues & Design Flaws

### 1. Feature Flags Are Decorative
**Problem**: `kairos_brief` and `kairos_channels` flags have no effect on runtime behavior.

**Location**: `crates/tools/src/lib.rs` lines 61, 385-387

```rust
// Unconditional registration - feature flag ignored
pub use brief::BriefTool;
// ...
Box::new(BriefTool),
Box::new(CronCreateTool),
Box::new(CronDeleteTool),
Box::new(CronListTool),
```

**Fix needed**: Wrap registration in `#[cfg(feature = "kairos_brief")]`.

### 2. No Channel Implementation
**Problem**: `kairos_channels` feature flag exists but zero code references it.

**Impact**: Flag is useless; no channels, subscriptions, or multi-channel routing exist.

### 3. Cron Scheduler Lifecycle Not Exposed
**Problem**: Cron scheduler starts in `main.rs` but there's no way to view/manage it from within the session.

**Current flow**:
```rust
// main.rs lines 730-738
let cron_cancel = CancellationToken::new();
start_cron_scheduler(&query_state, cron_cancel.clone());
```

**Missing**: Commands to pause/resume/reload cron scheduler at runtime.

### 4. Persistent Storage Race Condition
**Problem**: `ensure_store_loaded()` uses a mutex but `CRON_STORE` is a separate `Lazy` static.

**Risk**: If `CronList` runs before the scheduler loads disk state, it may miss durable tasks.

**Current mitigation**: Each tool calls `ensure_store_loaded()` before accessing the store.

### 5. No Task Output Visibility
**Problem**: Cron tasks fire in the background but their output is logged only (no UI display).

**Location**: `cron_scheduler.rs` lines 94-113

```rust
// Output goes to logs, not user-visible
info!(id = %task_id, "Cron task completed");
```

**Missing**: A way to view cron task results/history in the TUI.

### 6. Hard-Coded Limits
**Problem**: Max 50 jobs hardcoded in `CronCreate`.

**Location**: `cron.rs` line 319

```rust
if store.len() >= 50 {
    return ToolResult::error("Too many scheduled jobs (max 50).");
}
```

**Fix**: Make configurable via `.claurst/config.json`.

---

## Testing Procedures

### Test 1: Basic Brief
```bash
# In an active session
# Model should be able to call:
Brief { message: "Test", status: "normal" }
```

### Test 2: Recurring Cron
```bash
# Schedule every minute
CronCreate { cron: "* * * * *", prompt: "echo hello", recurring: true, durable: false }
# Wait 60 seconds — should see log output
```

### Test 3: Durable Persistence
```bash
# Create durable task
CronCreate { cron: "*/5 * * * *", prompt: "check status", durable: true }
# Exit claurst
# Restart claurst
CronList  # Task should still appear
```

### Test 4: One-Shot Task
```bash
CronCreate { cron: "* * * * *", prompt: "run once", recurring: false }
# Wait for next minute — fires once then auto-deletes
CronList  # Task should be gone
```

---

## Steps to Fully Enable Kairos Mode

### Option A: Enforce Feature Gates (Recommended)
1. Modify `crates/tools/src/lib.rs`:
   ```rust
   #[cfg(feature = "kairos_brief")]
   pub use brief::BriefTool;

   #[cfg(feature = "kairos_brief")]
   Box::new(BriefTool),
   ```

2. Add runtime check in TUI initialization:
   ```rust
   if cfg!(feature = "kairos_brief") {
       // Register Kairos-specific commands
   }
   ```

3. Add environment variable override:
   ```rust
   let kairos_enabled = cfg!(feature = "kairos_brief")
       || std::env::var("KAIROS").is_ok();
   ```

### Option B: Remove Feature Gates (Simpler)
Since `--all-features` already compiles everything:
1. Delete `kairos_brief` and `kairos_channels` from `Cargo.toml`
2. Document that Brief/Cron tools are always available
3. Focus on implementing missing pieces (commands, channels)

### Option C: Implement Full Kairos (Complete)
1. Enforce feature gates (Option A)
2. Implement `/brief` command to toggle brief-only mode
3. Implement `assistant` command for agent communication
4. Implement `subscribe-pr` for GitHub webhooks
5. Implement channel routing for `kairos_channels`
6. Add GrowthBook integration for remote feature flags

---

## File Reference Map

| File | Purpose |
|------|---------|
| `crates/core/Cargo.toml` | Feature flag definitions |
| `crates/tui/Cargo.toml` | Feature pass-through |
| `crates/tools/src/brief.rs` | BriefTool implementation |
| `crates/tools/src/cron.rs` | CronCreate/Delete/List tools |
| `crates/tools/src/lib.rs` | Tool registration |
| `crates/query/src/cron_scheduler.rs` | Background scheduler |
| `crates/cli/src/main.rs` | Scheduler startup (lines 730-738) |

---

## Summary

| Aspect | Status |
|--------|--------|
| **Build** | Works with `--all-features` |
| **Runtime activation** | Always on (no gating) |
| **Core tools** | Fully functional |
| **Commands/UI** | Missing |
| **Channels** | Not implemented |
| **Production ready** | No (experimental only) |

**Bottom line**: Kairos mode compiles and works, but the feature flags don't actually gate anything. The tools are always available once built. To make them truly optional, add `#[cfg(feature = "...")]` guards around tool registration.

---

## Progress Addendum (feature/kairos-mode)

This document started as a baseline snapshot. The items below track code changes applied after that snapshot.

### 2026-04-16 — Applied Changes

### 2026-04-16 — Unified cron/agent background execution substrate

- Refactored the cron scheduler to use the same background runner as slash commands (`spawn_background_agent_task`).
- Exposed a new function in CLI for background agent/cron tasks, reusing the slash command substrate.
- All background agent/cron execution now flows through a single substrate, simplifying future agent management and session/report tracking.

- Introduced strict runtime Kairos gating and initialization in `src-rust/crates/core/src/kairos_gate.rs`.
- Wired startup initialization order in `src-rust/crates/cli/src/main.rs` (both normal and named-command paths).
- Gated Kairos tool exposure in `src-rust/crates/tools/src/lib.rs`.
- Added assistant bootstrap behavior and query-config propagation in `src-rust/crates/cli/src/main.rs` and `src-rust/crates/query/src/lib.rs`.
- Added async background command path for `/btw`, including MCP settlement wait and completion requeue.
- Added command execution policy metadata in `src-rust/crates/commands/src/lib.rs`.
- Replaced hardcoded `/btw` route with policy-based background routing in `src-rust/crates/cli/src/main.rs`.
- Extracted reusable helper `spawn_background_slash_command(...)` in `src-rust/crates/cli/src/main.rs` as shared background-runner foundation.

### Validation

- `cargo check -p claurst` passed.
- `cargo check -p claurst --features kairos_brief` passed.
