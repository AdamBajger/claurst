// cron_scheduler: background task that fires cron-scheduled prompts.
//
// Runs as a long-lived tokio task. Every minute it checks the global CRON_STORE
// (in cc-tools) for tasks whose cron expression matches the current wall-clock
// minute. Matching tasks are fired by spawning a sub-query loop, exactly like
// the AgentTool does for sub-agents.
//
// One-shot tasks (recurring=false) are automatically removed from the store
// by `pop_due_tasks` after they are returned.

use crate::background_runner::{
    AgentRunContext, AgentRunRequest, AgentRunResult, AgentRunSource, spawn_agent_run,
};
use crate::QueryConfig;
use claurst_core::AgentConfig;
use claurst_core::live_session::SharedLiveSession;
use claurst_tools::{Tool, ToolContext};
use chrono::Timelike;
use std::sync::Arc;
use tokio::sync::mpsc;
use tokio::time::{Duration, sleep};
use tokio_util::sync::CancellationToken;
use tracing::{debug, info};

/// Starts the background cron scheduler as a detached tokio task.
/// Call `cancel.cancel()` to stop it gracefully.
/// `result_tx` is forwarded to each fired task so its output reaches the TUI
/// drain loop; pass `None` for headless contexts with no drain.
///
/// Phase 11: when `live_session` is `Some`, each fired task whose
/// `CronTask.agent_name` is set resolves through
/// `LiveSession::resolve_agent_config(name)`. With `None` (back-compat),
/// every fire uses `AgentConfig::default()`.
pub fn start_cron_scheduler(
    client: Arc<claurst_api::AnthropicClient>,
    tools: Arc<Vec<Box<dyn Tool>>>,
    tool_ctx: ToolContext,
    query_config: QueryConfig,
    result_tx: Option<mpsc::UnboundedSender<AgentRunResult>>,
    cancel: CancellationToken,
    live_session: Option<SharedLiveSession>,
) {
    tokio::spawn(async move {
        info!("Cron scheduler started");

        loop {
            let now = chrono::Local::now();
            let secs_into_minute = now.second() as u64;
            let nanos_ms = now.nanosecond() as u64 / 1_000_000;
            let ms_to_next_minute = (60u64.saturating_sub(secs_into_minute))
                .saturating_mul(1_000)
                .saturating_sub(nanos_ms)
                .max(1);

            tokio::select! {
                _ = sleep(Duration::from_millis(ms_to_next_minute)) => {}
                _ = cancel.cancelled() => {
                    info!("Cron scheduler stopped");
                    return;
                }
            }

            let tick_time = chrono::Local::now();
            debug!(time = %tick_time.format("%H:%M"), "Cron scheduler tick");

            for task in claurst_tools::cron::pop_due_tasks(&tick_time).await {
                info!(
                    id = %task.id,
                    cron = %task.cron,
                    agent = ?task.agent_name,
                    "Firing cron task",
                );
                let run_id = task.id.clone();
                let agent_config = match (live_session.as_ref(), task.agent_name.as_deref()) {
                    (Some(live), Some(name)) => live.resolve_agent_config(Some(name)),
                    _ => AgentConfig::default(),
                };
                spawn_agent_run(
                    AgentRunRequest {
                        run_id,
                        source: AgentRunSource::Cron { task_id: task.id },
                        prompt: task.prompt,
                    },
                    AgentRunContext {
                        query_config: query_config.clone(),
                        agent_config,
                        tool_ctx: tool_ctx.clone(),
                        client: client.clone(),
                        tools: tools.clone(),
                        result_tx: result_tx.clone(),
                        task_tracker: None,
                        event_log: None,
                    },
                );
            }
        }
    });
}
