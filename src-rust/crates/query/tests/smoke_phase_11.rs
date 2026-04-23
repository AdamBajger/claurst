//! Phase 11 smoke tests — named-agent management on LiveSession + CronTask
//! `agent_name` serde compat.

use claurst_core::cost::CostTracker;
use claurst_core::live_session::LiveSession;
use claurst_core::permissions::PermissionManager;
use claurst_core::{AgentConfig, PermissionMode, Settings};
use claurst_tools::cron::CronTask;

fn make_live(settings: Settings) -> std::sync::Arc<LiveSession> {
    let cwd = std::env::temp_dir();
    let cost = CostTracker::new();
    let perm = PermissionManager::new(PermissionMode::Default, &settings);
    LiveSession::new(settings, cwd, cost, perm)
}

#[test]
fn agent_names_unions_settings_and_ephemeral() {
    let mut settings = Settings::default();
    settings
        .agents
        .insert("from-settings".into(), AgentConfig::default());
    settings
        .agents
        .insert("shared".into(), AgentConfig::default());
    let live = make_live(settings);

    live.put_ephemeral_agent("from-ephemeral", AgentConfig::default());
    live.put_ephemeral_agent("shared", AgentConfig::default());

    let names = live.agent_names();
    assert!(names.contains(&"from-settings".to_string()));
    assert!(names.contains(&"from-ephemeral".to_string()));
    assert!(names.contains(&"shared".to_string()));
    // Deduplicated.
    let count_shared = names.iter().filter(|n| *n == "shared").count();
    assert_eq!(count_shared, 1);
}

#[test]
fn agent_exists_returns_true_for_either_origin() {
    let mut settings = Settings::default();
    settings.agents.insert("a".into(), AgentConfig::default());
    let live = make_live(settings);
    live.put_ephemeral_agent("b", AgentConfig::default());

    assert!(live.agent_exists("a"));
    assert!(live.agent_exists("b"));
    assert!(!live.agent_exists("ghost"));
}

#[test]
fn ephemeral_shadows_settings_in_resolve() {
    let mut settings = Settings::default();
    settings.agents.insert(
        "research".into(),
        AgentConfig {
            description: Some("from-settings".into()),
            model: Some("settings/model".into()),
            ..Default::default()
        },
    );
    let live = make_live(settings);

    // Before shadow: settings agent resolves.
    let cfg = live.resolve_agent_config(Some("research"));
    assert_eq!(cfg.description.as_deref(), Some("from-settings"));

    // Add ephemeral with same name.
    live.put_ephemeral_agent(
        "research",
        AgentConfig {
            description: Some("shadowed".into()),
            model: Some("ephemeral/model".into()),
            ..Default::default()
        },
    );

    let cfg = live.resolve_agent_config(Some("research"));
    assert_eq!(cfg.description.as_deref(), Some("shadowed"));
    assert_eq!(cfg.model.as_deref(), Some("ephemeral/model"));
}

#[test]
fn delete_agent_removes_ephemeral_first_then_settings() {
    let mut settings = Settings::default();
    settings.agents.insert(
        "x".into(),
        AgentConfig {
            description: Some("settings-x".into()),
            ..Default::default()
        },
    );
    let live = make_live(settings);
    live.put_ephemeral_agent(
        "x",
        AgentConfig {
            description: Some("ephemeral-x".into()),
            ..Default::default()
        },
    );

    // First delete removes ephemeral; settings still has it.
    let removed = live.delete_agent("x").expect("ephemeral removed");
    assert_eq!(removed.description.as_deref(), Some("ephemeral-x"));
    assert!(live.agent_exists("x"));

    // Second delete removes settings entry.
    let removed = live.delete_agent("x").expect("settings removed");
    assert_eq!(removed.description.as_deref(), Some("settings-x"));
    assert!(!live.agent_exists("x"));

    // Third delete is None.
    assert!(live.delete_agent("x").is_none());
}

#[test]
fn promote_ephemeral_moves_into_settings() {
    let live = make_live(Settings::default());
    live.put_ephemeral_agent(
        "scratch",
        AgentConfig {
            description: Some("hand-built".into()),
            ..Default::default()
        },
    );

    assert!(live.ephemeral.read().agents.contains_key("scratch"));
    assert!(!live.settings.read().agents.contains_key("scratch"));

    live.promote_ephemeral_agent("scratch").expect("promote ok");

    assert!(!live.ephemeral.read().agents.contains_key("scratch"));
    assert_eq!(
        live.settings
            .read()
            .agents
            .get("scratch")
            .and_then(|a| a.description.clone())
            .as_deref(),
        Some("hand-built")
    );
}

#[test]
fn promote_unknown_returns_err() {
    let live = make_live(Settings::default());
    let err = live.promote_ephemeral_agent("ghost").unwrap_err();
    assert!(err.contains("ghost"));
}

#[test]
fn cron_task_with_agent_name_serde_round_trip() {
    let task = CronTask {
        id: "t1".into(),
        cron: "* * * * *".into(),
        prompt: "hello".into(),
        recurring: true,
        durable: false,
        created_at: 1234567890,
        agent_name: Some("docs-rag".into()),
    };
    let json = serde_json::to_string(&task).unwrap();
    let back: CronTask = serde_json::from_str(&json).unwrap();
    assert_eq!(back.agent_name.as_deref(), Some("docs-rag"));
}

#[test]
fn legacy_cron_json_without_agent_name_loads_as_none() {
    // Simulate an on-disk cron task written before Phase 11.
    let legacy = r#"{
        "id": "old",
        "cron": "0 9 * * *",
        "prompt": "morning brief",
        "recurring": true,
        "durable": true,
        "created_at": 1700000000
    }"#;
    let task: CronTask = serde_json::from_str(legacy).expect("parse legacy");
    assert_eq!(task.id, "old");
    assert!(task.agent_name.is_none());
}

#[test]
fn cron_task_serialization_omits_none_agent_name() {
    let task = CronTask {
        id: "t1".into(),
        cron: "* * * * *".into(),
        prompt: "x".into(),
        recurring: false,
        durable: false,
        created_at: 0,
        agent_name: None,
    };
    let json = serde_json::to_string(&task).unwrap();
    assert!(!json.contains("agent_name"), "expected agent_name omitted, got: {}", json);
}
