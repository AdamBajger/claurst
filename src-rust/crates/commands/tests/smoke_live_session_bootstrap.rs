//! Bootstrap smoke tests for the `CommandContext.live_session` field added by
//! task #26. Verifies CommandContext can carry a `SharedLiveSession`, that
//! slash commands can read agent + project state through it, and that
//! pre-session contexts (`live_session: None`) still construct.

use std::path::PathBuf;
use std::sync::Arc;

use claurst_commands::CommandContext;
use claurst_core::cost::CostTracker;
use claurst_core::live_session::LiveSession;
use claurst_core::permissions::PermissionManager;
use claurst_core::project_registry::{ProjectConfig, ProjectRegistry};
use claurst_core::{AgentConfig, Config, PermissionMode, Settings};

fn make_live(settings: Settings, registry: ProjectRegistry) -> Arc<LiveSession> {
    let cwd = std::env::temp_dir();
    let cost = CostTracker::new();
    let perm = PermissionManager::new(PermissionMode::Default, &settings);
    LiveSession::with_projects(settings, cwd, cost, perm, registry)
}

fn empty_ctx_with_live(live: Option<Arc<LiveSession>>) -> CommandContext {
    CommandContext {
        config: Config::default(),
        cost_tracker: CostTracker::new(),
        messages: vec![],
        working_dir: PathBuf::from("."),
        session_id: "test".to_string(),
        session_title: None,
        remote_session_url: None,
        mcp_manager: None,
        live_session: live,
    }
}

#[test]
fn command_context_accepts_none_live_session() {
    let ctx = empty_ctx_with_live(None);
    assert!(ctx.live_session.is_none());
}

#[test]
fn command_context_accepts_bootstrapped_live_session() {
    let live = make_live(Settings::default(), ProjectRegistry::new());
    let ctx = empty_ctx_with_live(Some(live.clone()));
    let live_ref = ctx.live_session.as_ref().expect("live present");
    assert!(live_ref.agent_names().is_empty());
    assert!(live_ref.active_project_name().is_none());
}

#[test]
fn live_session_in_ctx_exposes_agent_management() {
    let mut settings = Settings::default();
    settings
        .agents
        .insert("settings-agent".into(), AgentConfig::default());
    let live = make_live(settings, ProjectRegistry::new());
    live.put_ephemeral_agent("scratch", AgentConfig::default());

    let ctx = empty_ctx_with_live(Some(live));
    let live = ctx.live_session.as_ref().unwrap();
    assert!(live.agent_exists("settings-agent"));
    assert!(live.agent_exists("scratch"));
    assert_eq!(live.agent_names().len(), 2);
}

#[test]
fn live_session_in_ctx_exposes_project_switch() {
    let tmp = tempfile::tempdir().unwrap();
    let mut reg = ProjectRegistry::new();
    reg.insert(ProjectConfig::new("alpha", tmp.path().to_path_buf()));

    let live = make_live(Settings::default(), reg);
    let ctx = empty_ctx_with_live(Some(live));
    let live = ctx.live_session.as_ref().unwrap();

    live.switch_project("alpha").expect("switch ok");
    assert_eq!(live.active_project_name().as_deref(), Some("alpha"));
    assert_eq!(
        *live.runtime.working_directory.read(),
        tmp.path().to_path_buf()
    );
}

#[test]
fn live_session_clones_share_inner_state() {
    let live = make_live(Settings::default(), ProjectRegistry::new());
    let ctx_a = empty_ctx_with_live(Some(live.clone()));
    let ctx_b = empty_ctx_with_live(Some(live));

    ctx_a
        .live_session
        .as_ref()
        .unwrap()
        .put_ephemeral_agent("shared", AgentConfig::default());

    assert!(ctx_b
        .live_session
        .as_ref()
        .unwrap()
        .agent_exists("shared"));
}
