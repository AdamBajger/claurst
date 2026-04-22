// proactive_ticker: periodic autonomous tick for Kairos proactive mode.
//
// Fires every KAIROS_PROACTIVE_INTERVAL_SECS (default 15 min). Each tick sends
// the proactive prompt to a fresh run_query_loop and forwards the result to the
// TUI via the shared bg_task_tx channel.
//
// Runs are sequential: the next tick starts sleeping only after the current run
// finishes, so overlapping proactive tasks are structurally impossible.
//
// Backoff: after MAX_CONSECUTIVE_ERRORS consecutive error outcomes, the sleep
// interval doubles for subsequent ticks. Resets to base interval on success.

use std::sync::Arc;
use tokio::sync::mpsc::UnboundedSender;
use tokio::time::Duration;
use tokio_util::sync::CancellationToken;
use tracing::{debug, info, warn};
use claurst_tools::{Tool, ToolContext};

use crate::background_runner::{AgentRunContext, AgentRunRequest, AgentRunResult, AgentRunSource, execute_agent_run};
use crate::QueryConfig;

const MAX_CONSECUTIVE_ERRORS: u32 = 3;
/// Number of consecutive per-tick cost overruns that shut the ticker down.
/// Small number since each overrun already cost real money.
const MAX_COST_OVERRUNS: u32 = 2;

pub fn start_proactive_ticker(
    query_config: QueryConfig,
    tool_ctx: ToolContext,
    client: Arc<claurst_api::AnthropicClient>,
    tools: Arc<Vec<Box<dyn Tool>>>,
    result_tx: UnboundedSender<AgentRunResult>,
    cancel: CancellationToken,
) {
    let base_interval = Duration::from_secs(claurst_core::kairos_gate::proactive_interval_secs());
    let tick_cost_ceiling = claurst_core::kairos_gate::proactive_tick_max_usd();

    tokio::spawn(async move {
        info!(
            interval_secs = base_interval.as_secs(),
            tick_cost_ceiling_usd = ?tick_cost_ceiling,
            "Proactive ticker started"
        );
        let mut consecutive_errors: u32 = 0;
        let mut cost_overruns: u32 = 0;

        loop {
            let sleep_duration = if consecutive_errors >= MAX_CONSECUTIVE_ERRORS {
                base_interval * 2
            } else {
                base_interval
            };

            tokio::select! {
                _ = tokio::time::sleep(sleep_duration) => {}
                _ = cancel.cancelled() => {
                    info!("Proactive ticker stopped");
                    return;
                }
            }

            let run_id = uuid::Uuid::new_v4().to_string();
            debug!(run_id = %run_id, "Proactive tick firing");

            let cost_before = tool_ctx.cost_tracker.total_cost_usd();

            let is_error = execute_agent_run(
                AgentRunRequest {
                    run_id,
                    source: AgentRunSource::Proactive,
                    prompt: claurst_core::kairos_gate::proactive_tick_prompt(),
                },
                AgentRunContext {
                    query_config: query_config.clone(),
                    tool_ctx: tool_ctx.clone(),
                    client: client.clone(),
                    tools: tools.clone(),
                    result_tx: Some(result_tx.clone()),
                },
            )
            .await;

            let cost_delta = tool_ctx.cost_tracker.total_cost_usd() - cost_before;

            if is_error {
                consecutive_errors = consecutive_errors.saturating_add(1);
                if consecutive_errors == MAX_CONSECUTIVE_ERRORS {
                    warn!(
                        consecutive = consecutive_errors,
                        doubled_interval_secs = (base_interval * 2).as_secs(),
                        "Proactive ticker: repeated failures, doubling sleep interval"
                    );
                }
            } else {
                consecutive_errors = 0;
            }

            if let Some(ceiling) = tick_cost_ceiling {
                if cost_delta > ceiling {
                    cost_overruns = cost_overruns.saturating_add(1);
                    warn!(
                        cost_delta_usd = cost_delta,
                        ceiling_usd = ceiling,
                        strikes = cost_overruns,
                        max_strikes = MAX_COST_OVERRUNS,
                        "Proactive ticker: tick exceeded per-tick cost ceiling"
                    );
                    if cost_overruns >= MAX_COST_OVERRUNS {
                        warn!(
                            strikes = cost_overruns,
                            "Proactive ticker: repeated cost overruns, stopping"
                        );
                        return;
                    }
                } else {
                    cost_overruns = 0;
                }
            }
        }
    });
}
