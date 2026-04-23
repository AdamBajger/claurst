use std::sync::Arc;
use tokio_util::sync::CancellationToken;
use tracing::info;
use claurst_tools::{Tool, ToolContext};
use claurst_core::AgentConfig;
use claurst_core::event_log::{Event, EventKind, EventLog};
use claurst_core::permissions::TaskSource;
use claurst_core::task_tracker::{SimpleTrackedTask, TaskKind, TaskStatus, TaskTracker, TrackedTask};

use crate::{QueryConfig, QueryOutcome, apply_agent_config_to_query_config, run_query_loop};
use claurst_core::task_history::{RunStatus, TaskRunRecord, record_run};

#[derive(Debug)]
pub enum AgentRunSource {
    Cron { task_id: String },
    SlashCommand { name: String },
    Proactive,
}

impl AgentRunSource {
    /// Map to the canonical Phase 9 `TaskSource` enum used by the tracker
    /// and (later) the permission dialog.
    pub fn as_task_source(&self) -> TaskSource {
        match self {
            AgentRunSource::Cron { task_id } => TaskSource::Cron(task_id.clone()),
            AgentRunSource::SlashCommand { name } => TaskSource::SlashCommand(name.clone()),
            AgentRunSource::Proactive => TaskSource::Proactive,
        }
    }
}

pub struct AgentRunRequest {
    pub run_id: String,
    pub source: AgentRunSource,
    pub prompt: String,
}

pub struct AgentRunContext {
    /// Base query-loop config (model, registries, budgets). Round 2 transition:
    /// stays as the carrier through `run_query_loop`; spawn-time agent overlay
    /// is applied via `apply_agent_config_to_query_config` before the loop runs.
    pub query_config: QueryConfig,
    /// Static per-spawn agent config. Round 2 canonical spawn input.
    pub agent_config: AgentConfig,
    pub tool_ctx: ToolContext,
    pub client: Arc<claurst_api::AnthropicClient>,
    pub tools: Arc<Vec<Box<dyn Tool>>>,
    pub result_tx: Option<tokio::sync::mpsc::UnboundedSender<AgentRunResult>>,
    /// Phase 9.5: optional registry. When `Some`, the run registers itself on
    /// spawn and deregisters on completion. Cancellation propagates via the
    /// returned task's `cancel_token`.
    pub task_tracker: Option<TaskTracker>,
    /// Phase 10: optional event log. When `Some`, the run emits
    /// `BackgroundStart` + `BackgroundFinish` events so `/activity` and the
    /// status line surface it.
    pub event_log: Option<EventLog>,
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

    // Apply agent-config overlay onto a per-spawn clone of the base query config.
    let mut effective_qcfg = ctx.query_config.clone();
    apply_agent_config_to_query_config(&ctx.agent_config, &mut effective_qcfg);

    info!(
        run_id = %req.run_id,
        source = %label,
        agent_model = ?ctx.agent_config.model,
        agent_max_turns = ?ctx.agent_config.max_turns,
        agent_project = ?ctx.agent_config.project,
        kairos_addendum = ctx.agent_config.kairos_addendum,
        "Spawning agent run",
    );

    // Phase 9.5: register with TaskTracker so /tasks + /stop all see this run.
    let cancel_token = CancellationToken::new();
    let task_source = req.source.as_task_source();
    let tracked = ctx.task_tracker.as_ref().map(|tracker| {
        let task = SimpleTrackedTask::new(
            req.run_id.clone(),
            TaskKind::Agent,
            task_source.clone(),
            format!("agent run {} ({})", req.run_id, label),
            cancel_token.clone(),
        );
        task.set_details(format!("source={}\nprompt: {}", label, prompt_preview));
        tracker.register(task.clone());
        (tracker.clone(), task)
    });

    // Phase 10: emit BackgroundStart.
    if let Some(log) = ctx.event_log.as_ref() {
        log.push(
            Event::now(
                EventKind::BackgroundStart,
                task_source.clone(),
                format!("agent run {} started ({})", req.run_id, label),
            )
            .with_details(format!("prompt: {}", prompt_preview)),
        );
    }

    let mut messages = vec![claurst_core::types::Message::user(req.prompt)];
    let cost_tracker = ctx.tool_ctx.cost_tracker.clone();
    let outcome = run_query_loop(
        ctx.client.as_ref(),
        &mut messages,
        &ctx.tools,
        &ctx.tool_ctx,
        &effective_qcfg,
        cost_tracker,
        None,
        cancel_token.clone(),
        None,
    )
    .await;

    let (output, is_error) = format_outcome(outcome);

    if let Some((tracker, task)) = tracked.as_ref() {
        let final_status = if cancel_token.is_cancelled() {
            TaskStatus::Cancelled
        } else if is_error {
            TaskStatus::Failed { error: output.chars().take(200).collect() }
        } else {
            TaskStatus::Completed
        };
        task.set_status(final_status);
        tracker.deregister(task.id());
    }

    // Phase 10: emit BackgroundFinish.
    if let Some(log) = ctx.event_log.as_ref() {
        log.push(
            Event::now(
                EventKind::BackgroundFinish { is_error },
                task_source,
                format!(
                    "agent run {} {} ({})",
                    req.run_id,
                    if is_error { "failed" } else { "finished" },
                    label
                ),
            )
            .with_details(output.chars().take(300).collect::<String>()),
        );
    }

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
