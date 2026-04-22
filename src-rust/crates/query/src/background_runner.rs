use std::sync::Arc;
use tokio_util::sync::CancellationToken;
use claurst_tools::{Tool, ToolContext};

use crate::{QueryConfig, QueryOutcome, run_query_loop};
use claurst_core::task_history::{RunStatus, TaskRunRecord, record_run};

#[derive(Debug)]
pub enum AgentRunSource {
    Cron { task_id: String },
    SlashCommand { name: String },
    Proactive,
}

pub struct AgentRunRequest {
    pub run_id: String,
    pub source: AgentRunSource,
    pub prompt: String,
}

pub struct AgentRunContext {
    pub query_config: QueryConfig,
    pub tool_ctx: ToolContext,
    pub client: Arc<claurst_api::AnthropicClient>,
    pub tools: Arc<Vec<Box<dyn Tool>>>,
    pub result_tx: Option<tokio::sync::mpsc::UnboundedSender<AgentRunResult>>,
}

#[derive(Debug)]
pub struct AgentRunResult {
    pub run_id: String,
    pub source: AgentRunSource,
    pub output: String,
    pub is_error: bool,
}

fn source_label(source: &AgentRunSource) -> String {
    match source {
        AgentRunSource::Cron { task_id } => format!("cron:{}", task_id),
        AgentRunSource::SlashCommand { name } => format!("/{}", name),
        AgentRunSource::Proactive => "proactive".to_string(),
    }
}

fn format_outcome(outcome: QueryOutcome) -> (String, bool) {
    match outcome {
        QueryOutcome::EndTurn { message, .. } => (message.get_all_text(), false),
        QueryOutcome::MaxTokens { partial_message, .. } => (
            format!(
                "Response hit max tokens. Partial output:\n{}",
                partial_message.get_all_text()
            ),
            false,
        ),
        QueryOutcome::BudgetExceeded { cost_usd, limit_usd } => (
            format!(
                "Background run stopped: budget limit ${:.4} reached (spent ${:.4}).",
                limit_usd, cost_usd
            ),
            true,
        ),
        QueryOutcome::Cancelled => ("Background run was cancelled.".to_string(), true),
        QueryOutcome::Error(e) => (format!("Background run failed: {}", e), true),
    }
}

/// Executes an agent task to completion. Must be called from within a spawned async task.
/// Returns true if the run ended in an error condition.
pub async fn execute_agent_run(req: AgentRunRequest, ctx: AgentRunContext) -> bool {
    let started_at = chrono::Utc::now();
    let label = source_label(&req.source);
    let prompt_preview = req.prompt.chars().take(120).collect::<String>();

    let mut messages = vec![claurst_core::types::Message::user(req.prompt)];
    let cost_tracker = ctx.tool_ctx.cost_tracker.clone();
    let outcome = run_query_loop(
        ctx.client.as_ref(),
        &mut messages,
        &ctx.tools,
        &ctx.tool_ctx,
        &ctx.query_config,
        cost_tracker,
        None,
        CancellationToken::new(),
        None,
    )
    .await;

    let (output, is_error) = format_outcome(outcome);

    record_run(TaskRunRecord {
        run_id: req.run_id.clone(),
        source_label: label,
        prompt_preview,
        started_at,
        completed_at: chrono::Utc::now(),
        status: if is_error { RunStatus::Error } else { RunStatus::Success },
        output_snippet: output.chars().take(300).collect(),
    })
    .await;

    if let Some(tx) = ctx.result_tx {
        let _ = tx.send(AgentRunResult {
            run_id: req.run_id,
            source: req.source,
            output,
            is_error,
        });
    }

    is_error
}

/// Spawns a direct-prompt agent task in the background (cron, proactive).
/// The query config must already have Kairos bootstrap applied before calling this.
pub fn spawn_agent_run(req: AgentRunRequest, ctx: AgentRunContext) {
    tokio::spawn(execute_agent_run(req, ctx));
}
