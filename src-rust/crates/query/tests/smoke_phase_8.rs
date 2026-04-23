//! Phase 8 smoke tests.
//!
//! Verifies:
//! 1. Round-trip serde of `AgentConfig` with new fields and legacy alias.
//! 2. Old `prompt` JSON deserializes into `append_system_prompt` via serde alias.
//! 3. `apply_agent_config_to_query_config` overlays expected fields.
//! 4. `LiveSession::resolve_agent_config` returns ephemeral / settings agents.
//! 5. `default_agents()` builds without panicking under the new shape.

use claurst_core::live_session::{EphemeralOverrides, EphemeralState, LiveSession};
use claurst_core::permissions::PermissionManager;
use claurst_core::{AgentConfig, AgentDefinition, PermissionMode, Settings, default_agents};
use claurst_core::cost::CostTracker;
use claurst_query::{QueryConfig, apply_agent_config_to_query_config};

#[test]
fn agent_config_legacy_prompt_alias_loads() {
    // Legacy on-disk shape: `prompt` field, no new fields. Round-trips into
    // `append_system_prompt` via serde alias and survives a re-serialize.
    let legacy = r#"{
        "description": "legacy agent",
        "model": "anthropic/claude-haiku-4-5",
        "temperature": 0.7,
        "prompt": "Be concise.",
        "access": "read-only",
        "visible": true,
        "max_turns": 10,
        "color": "yellow"
    }"#;

    let cfg: AgentConfig = serde_json::from_str(legacy).expect("legacy AgentConfig parses");
    assert_eq!(cfg.append_system_prompt.as_deref(), Some("Be concise."));
    assert_eq!(cfg.access, "read-only");
    assert_eq!(cfg.max_turns, Some(10));
    // New fields take their defaults.
    assert!(cfg.tools.allowlist.is_none());
    assert!(cfg.mcp.enabled_servers.is_empty());
    assert!(!cfg.kairos_addendum);
    assert!(cfg.project.is_none());

    // Round-trip the new shape.
    let json = serde_json::to_string(&cfg).expect("serialize");
    let cfg2: AgentConfig = serde_json::from_str(&json).expect("re-parse");
    assert_eq!(cfg.append_system_prompt, cfg2.append_system_prompt);
    assert_eq!(cfg.color, cfg2.color);
}

#[test]
fn agent_definition_alias_compiles() {
    // The back-compat type alias must point at AgentConfig so existing code
    // referencing `AgentDefinition` continues working.
    let _: AgentDefinition = AgentConfig::default();
}

#[test]
fn default_agents_build_under_new_shape() {
    // Phase 8 changed default_agents() to use ..Default::default() with the
    // renamed `append_system_prompt`. Make sure the three presets construct
    // and carry the right access preset.
    let m = default_agents();
    assert_eq!(m.len(), 3);
    assert_eq!(m["build"].access, "full");
    assert_eq!(m["plan"].access, "read-only");
    assert_eq!(m["explore"].access, "search-only");
    assert!(m["build"].append_system_prompt.is_some());
    assert!(m["plan"].append_system_prompt.is_some());
}

#[test]
fn apply_agent_config_overlays_fields() {
    let mut qcfg = QueryConfig::default();
    let baseline_max_tokens = qcfg.max_tokens;

    let agent = AgentConfig {
        model: Some("anthropic/claude-haiku-4-5".to_string()),
        max_tokens: Some(baseline_max_tokens + 1234),
        max_turns: Some(7),
        temperature: Some(0.42),
        thinking_budget: Some(2048),
        tool_result_budget: Some(99_999),
        append_system_prompt: Some("Suffix.".to_string()),
        system_prompt: Some("Override system prompt.".to_string()),
        effort: Some("high".to_string()),
        fallback_model: Some("anthropic/claude-sonnet-4-6".to_string()),
        ..Default::default()
    };

    apply_agent_config_to_query_config(&agent, &mut qcfg);

    assert_eq!(qcfg.model, "anthropic/claude-haiku-4-5");
    assert_eq!(qcfg.max_tokens, baseline_max_tokens + 1234);
    assert_eq!(qcfg.max_turns, 7);
    assert_eq!(qcfg.temperature, Some(0.42_f32));
    assert_eq!(qcfg.thinking_budget, Some(2048));
    assert_eq!(qcfg.tool_result_budget, 99_999);
    assert_eq!(qcfg.system_prompt.as_deref(), Some("Override system prompt."));
    assert_eq!(qcfg.append_system_prompt.as_deref(), Some("Suffix."));
    assert_eq!(qcfg.fallback_model.as_deref(), Some("anthropic/claude-sonnet-4-6"));
    assert!(matches!(
        qcfg.effort_level,
        Some(claurst_core::effort::EffortLevel::High)
    ));
    // The agent definition is mirrored so existing readers (system-prompt
    // assembly path) keep functioning.
    assert!(qcfg.agent_definition.is_some());
}

#[test]
fn apply_agent_config_appends_to_existing_suffix() {
    let mut qcfg = QueryConfig::default();
    qcfg.append_system_prompt = Some("Base.".to_string());

    let agent = AgentConfig {
        append_system_prompt: Some("Extra.".to_string()),
        ..Default::default()
    };
    apply_agent_config_to_query_config(&agent, &mut qcfg);

    assert_eq!(
        qcfg.append_system_prompt.as_deref(),
        Some("Base.\n\nExtra.")
    );
}

#[test]
fn apply_agent_config_unparseable_effort_is_ignored() {
    let mut qcfg = QueryConfig::default();
    let agent = AgentConfig {
        effort: Some("turbo".to_string()), // not a real EffortLevel
        ..Default::default()
    };
    apply_agent_config_to_query_config(&agent, &mut qcfg);
    assert!(qcfg.effort_level.is_none());
}

#[test]
fn live_session_resolve_agent_config_reads_settings_then_ephemeral() {
    let mut settings = Settings::default();
    settings.agents.insert(
        "research".to_string(),
        AgentConfig {
            description: Some("settings-defined".to_string()),
            model: Some("settings/model".to_string()),
            ..Default::default()
        },
    );

    let cwd = std::env::temp_dir();
    let cost = CostTracker::new();
    let permissions = PermissionManager::new(PermissionMode::Default, &settings);
    let live = LiveSession::new(settings, cwd, cost, permissions);

    // Settings agent resolves.
    let cfg = live.resolve_agent_config(Some("research"));
    assert_eq!(cfg.description.as_deref(), Some("settings-defined"));
    assert_eq!(cfg.model.as_deref(), Some("settings/model"));

    // Ephemeral agent shadows when settings doesn't have it.
    {
        let mut eph = live.ephemeral.write();
        eph.agents.insert(
            "scratch".to_string(),
            AgentConfig {
                description: Some("ephemeral-only".to_string()),
                ..Default::default()
            },
        );
    }
    let cfg = live.resolve_agent_config(Some("scratch"));
    assert_eq!(cfg.description.as_deref(), Some("ephemeral-only"));

    // Unknown name → default config.
    let cfg = live.resolve_agent_config(Some("unknown"));
    assert!(cfg.description.is_none());
    assert!(cfg.model.is_none());

    // None → default config.
    let cfg = live.resolve_agent_config(None);
    assert!(cfg.description.is_none());
}

#[test]
fn live_session_ephemeral_overrides_apply_to_resolved_config() {
    let settings = Settings::default();
    let cwd = std::env::temp_dir();
    let cost = CostTracker::new();
    let permissions = PermissionManager::new(PermissionMode::Default, &settings);
    let live = LiveSession::new(settings, cwd, cost, permissions);

    {
        let mut eph = live.ephemeral.write();
        eph.overrides = EphemeralOverrides {
            model: Some("ephemeral/model".to_string()),
            ..Default::default()
        };
    }

    let cfg = live.resolve_agent_config(None);
    assert_eq!(cfg.model.as_deref(), Some("ephemeral/model"));
}

#[test]
fn ephemeral_state_round_trips_serde() {
    // Constraint preserved for the future named-session-snapshot path.
    let mut eph = EphemeralState::default();
    eph.agents.insert("foo".to_string(), AgentConfig::default());
    eph.tool_denylist.insert("Bash".to_string());
    eph.overrides.model = Some("anthropic/claude-haiku-4-5".to_string());

    let json = serde_json::to_string(&eph).expect("serialize EphemeralState");
    let back: EphemeralState = serde_json::from_str(&json).expect("re-parse");
    assert_eq!(back.agents.len(), 1);
    assert!(back.tool_denylist.contains("Bash"));
    assert_eq!(
        back.overrides.model.as_deref(),
        Some("anthropic/claude-haiku-4-5")
    );
}
