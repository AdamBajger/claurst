//! Phase 8.5 smoke tests — ProjectRegistry + LiveSession project wiring.
//!
//! Verifies:
//! 1. ProjectConfig round-trips on disk via `save_one` + `load_from_dir`.
//! 2. `load_from_dir` returns empty registry when dir is missing (no error).
//! 3. `load_from_dir` skips garbage files but still loads the rest.
//! 4. `LiveSession::switch_project` updates cwd + active_project marker.
//! 5. `LiveSession::switch_project` returns Err for unknown name.
//! 6. `LiveSession::resolve_cwd` prefers explicit > project root > live cwd.
//! 7. Cron-style spawn with project-named agent resolves cwd from project root.

use std::collections::BTreeMap;
use std::path::PathBuf;

use claurst_core::cost::CostTracker;
use claurst_core::live_session::LiveSession;
use claurst_core::permissions::PermissionManager;
use claurst_core::project_registry::{ProjectConfig, ProjectRegistry};
use claurst_core::{AgentConfig, PermissionMode, Settings};

fn make_registry_with(name: &str, root: &PathBuf) -> ProjectRegistry {
    let mut reg = ProjectRegistry::new();
    reg.insert(ProjectConfig::new(name, root.clone()));
    reg
}

#[test]
fn project_config_roundtrips_on_disk() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("projects");

    let cfg = ProjectConfig {
        name: "alpha".to_string(),
        root_path: PathBuf::from("/tmp/alpha"),
        permission_rules: Vec::new(),
        default_agent: Some("build".to_string()),
        mcp_servers: BTreeMap::new(),
    };

    ProjectRegistry::save_one(&dir, &cfg).expect("save");
    let reg = ProjectRegistry::load_from_dir(&dir).expect("load");
    assert_eq!(reg.len(), 1);
    let loaded = reg.get("alpha").expect("alpha present");
    assert_eq!(loaded.root_path, PathBuf::from("/tmp/alpha"));
    assert_eq!(loaded.default_agent.as_deref(), Some("build"));
}

#[test]
fn load_from_missing_dir_is_empty_not_error() {
    let tmp = tempfile::tempdir().unwrap();
    let missing = tmp.path().join("does_not_exist");
    let reg = ProjectRegistry::load_from_dir(&missing).expect("load returns Ok");
    assert!(reg.is_empty());
}

#[test]
fn load_from_dir_skips_garbage_keeps_valid() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("projects");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("garbage.json"), b"{not valid json").unwrap();
    std::fs::write(dir.join("not_json.txt"), b"ignored").unwrap();

    let cfg = ProjectConfig::new("good", PathBuf::from("/tmp/good"));
    ProjectRegistry::save_one(&dir, &cfg).expect("save");

    let reg = ProjectRegistry::load_from_dir(&dir).expect("load");
    assert_eq!(reg.len(), 1);
    assert!(reg.get("good").is_some());
    assert!(reg.get("garbage").is_none());
}

#[test]
fn delete_one_returns_false_when_absent() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("projects");
    std::fs::create_dir_all(&dir).unwrap();
    let removed = ProjectRegistry::delete_one(&dir, "ghost").expect("delete ok");
    assert!(!removed);
}

#[test]
fn live_session_switch_project_updates_cwd_and_marker() {
    let settings = Settings::default();
    let initial_cwd = std::env::temp_dir();
    let project_root = std::env::temp_dir().join("phase_8_5_project");
    let cost = CostTracker::new();
    let permissions = PermissionManager::new(PermissionMode::Default, &settings);
    let registry = make_registry_with("alpha", &project_root);

    let live = LiveSession::with_projects(settings, initial_cwd.clone(), cost, permissions, registry);

    assert!(live.active_project_name().is_none());
    assert_eq!(*live.runtime.working_directory.read(), initial_cwd);

    live.switch_project("alpha").expect("switch ok");

    assert_eq!(live.active_project_name().as_deref(), Some("alpha"));
    assert_eq!(*live.runtime.working_directory.read(), project_root);

    live.clear_active_project();
    assert!(live.active_project_name().is_none());
    // cwd is intentionally NOT reverted on clear — explicit Phase 8.5 semantics.
    assert_eq!(*live.runtime.working_directory.read(), project_root);
}

#[test]
fn live_session_switch_project_unknown_returns_err() {
    let settings = Settings::default();
    let cost = CostTracker::new();
    let permissions = PermissionManager::new(PermissionMode::Default, &settings);
    let live = LiveSession::new(settings, std::env::temp_dir(), cost, permissions);

    let err = live.switch_project("ghost").expect_err("unknown project");
    assert_eq!(err, "ghost");
}

#[test]
fn resolve_cwd_precedence_explicit_project_live() {
    let settings = Settings::default();
    let live_cwd = std::env::temp_dir().join("live_cwd");
    let project_root = std::env::temp_dir().join("project_root");
    let explicit = std::env::temp_dir().join("explicit_override");

    let cost = CostTracker::new();
    let permissions = PermissionManager::new(PermissionMode::Default, &settings);
    let registry = make_registry_with("foo", &project_root);
    let live = LiveSession::with_projects(settings, live_cwd.clone(), cost, permissions, registry);

    // Explicit wins over everything.
    assert_eq!(
        live.resolve_cwd(Some(&explicit), Some("foo")),
        explicit
    );
    // Project name wins over live cwd.
    assert_eq!(live.resolve_cwd(None, Some("foo")), project_root);
    // Unknown project name falls back to live cwd (warn-level, no error).
    assert_eq!(live.resolve_cwd(None, Some("ghost")), live_cwd);
    // No project, no explicit → live cwd.
    assert_eq!(live.resolve_cwd(None, None), live_cwd);
}

#[test]
fn agent_config_with_project_resolves_via_live_session() {
    // Cron-task scenario: AgentConfig.project = "foo" → resolve_cwd returns the
    // project root regardless of the live session cwd.
    let settings = Settings::default();
    let live_cwd = std::env::temp_dir().join("phase_8_5_live");
    let project_root = std::env::temp_dir().join("phase_8_5_proj");

    let cost = CostTracker::new();
    let permissions = PermissionManager::new(PermissionMode::Default, &settings);
    let registry = make_registry_with("foo", &project_root);
    let live = LiveSession::with_projects(settings, live_cwd, cost, permissions, registry);

    let agent = AgentConfig {
        project: Some("foo".to_string()),
        ..Default::default()
    };

    let resolved = live.resolve_cwd(None, agent.project.as_deref());
    assert_eq!(resolved, project_root);
}

#[test]
fn switch_project_loads_rules_into_permission_manager() {
    use claurst_core::permissions::{PermissionAction, PermissionScope, SerializedPermissionRule};

    let settings = Settings::default();
    let cost = CostTracker::new();
    let perm = PermissionManager::new(PermissionMode::Default, &settings);

    let mut reg = ProjectRegistry::new();
    let mut alpha = ProjectConfig::new("alpha", PathBuf::from("/tmp/alpha"));
    alpha.permission_rules.push(SerializedPermissionRule {
        tool_name: Some("Bash".to_string()),
        path_pattern: None,
        action: PermissionAction::Allow,
        id: None,
        subject: None,
    });
    reg.insert(alpha);

    let live = LiveSession::with_projects(settings, std::env::temp_dir(), cost, perm, reg);

    // Before switch: no active project, Bash should still Ask (default).
    let decision_before = live
        .runtime
        .permissions
        .lock()
        .evaluate("Bash", "echo hi", None);
    assert!(matches!(
        decision_before,
        claurst_core::permissions::PermissionDecision::Ask { .. }
    ));

    live.switch_project("alpha").expect("switch ok");

    // After switch: active project's Allow rule wins.
    let perm_guard = live.runtime.permissions.lock();
    assert_eq!(perm_guard.active_project(), Some("alpha"));
    let decision_after = perm_guard.evaluate("Bash", "echo hi", None);
    assert_eq!(
        decision_after,
        claurst_core::permissions::PermissionDecision::Allow
    );
    // Loaded rules carry Project scope (overwritten by loader regardless of
    // the on-disk serde, which doesn't store scope).
    let listed = perm_guard.list_rules();
    assert!(listed.iter().any(|r| matches!(
        &r.scope,
        PermissionScope::Project { name } if name == "alpha"
    )));
    drop(perm_guard);

    live.clear_active_project();
    // Cleared bucket no longer participates.
    let cleared = live
        .runtime
        .permissions
        .lock()
        .evaluate("Bash", "echo hi", None);
    assert!(matches!(
        cleared,
        claurst_core::permissions::PermissionDecision::Ask { .. }
    ));
}

#[test]
fn persist_project_rule_appends_to_disk_and_registry() {
    use claurst_core::permissions::{
        PermissionAction, PermissionRule, PermissionScope, PermissionSubject,
    };

    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("projects");

    // Bootstrap a project on disk so persist_project_rule has something to update.
    let alpha = ProjectConfig::new("alpha", PathBuf::from("/tmp/alpha"));
    ProjectRegistry::save_one(&dir, &alpha).expect("save project");
    let registry = ProjectRegistry::load_from_dir(&dir).expect("reload");

    let settings = Settings::default();
    let cost = CostTracker::new();
    let perm = PermissionManager::new(PermissionMode::Default, &settings);
    let live =
        LiveSession::with_projects(settings, std::env::temp_dir(), cost, perm, registry);

    let mut rule = PermissionRule::legacy(
        None,
        None,
        PermissionAction::Allow,
        PermissionScope::Project { name: "alpha".into() },
    );
    rule.subject = Some(PermissionSubject::Tool { name: "Bash".into() });

    live
        .persist_project_rule("alpha", &rule, Some(&dir))
        .expect("persist");

    // Reload from disk and confirm the rule is present with id + subject.
    let reloaded = ProjectRegistry::load_from_dir(&dir).expect("reload");
    let cfg = reloaded.get("alpha").expect("alpha present");
    assert_eq!(cfg.permission_rules.len(), 1);
    let serialized = &cfg.permission_rules[0];
    assert_eq!(serialized.id, Some(rule.id));
    assert!(matches!(
        serialized.subject,
        Some(PermissionSubject::Tool { ref name }) if name == "Bash"
    ));

    // Removal mirror — happy path.
    let removed = live
        .persist_remove_project_rule("alpha", rule.id, Some(&dir))
        .expect("remove");
    assert!(removed);
    let reloaded = ProjectRegistry::load_from_dir(&dir).expect("reload");
    let cfg = reloaded.get("alpha").expect("alpha present");
    assert!(cfg.permission_rules.is_empty());

    // Removing again is a no-op (returns false).
    let removed_again = live
        .persist_remove_project_rule("alpha", rule.id, Some(&dir))
        .expect("remove again");
    assert!(!removed_again);
}

#[test]
fn load_from_dir_with_failures_reports_skipped_files() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("projects");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("garbage.json"), b"{not valid json").unwrap();
    std::fs::write(dir.join("not_json.txt"), b"ignored").unwrap();
    let cfg = ProjectConfig::new("good", PathBuf::from("/tmp/good"));
    ProjectRegistry::save_one(&dir, &cfg).expect("save");

    let (reg, failed) =
        ProjectRegistry::load_from_dir_with_failures(&dir).expect("load");
    assert_eq!(reg.len(), 1);
    assert!(reg.get("good").is_some());
    // garbage.json reported, not_json.txt ignored entirely (not a JSON file).
    assert_eq!(failed.len(), 1);
    assert_eq!(failed[0], "garbage.json");
}

#[test]
fn load_from_missing_dir_with_failures_returns_empty_no_failures() {
    let tmp = tempfile::tempdir().unwrap();
    let missing = tmp.path().join("nope");
    let (reg, failed) =
        ProjectRegistry::load_from_dir_with_failures(&missing).expect("ok");
    assert!(reg.is_empty());
    assert!(failed.is_empty());
}

#[test]
fn persist_project_rule_unknown_project_errors() {
    use claurst_core::permissions::{PermissionAction, PermissionRule, PermissionScope};
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("projects");

    let settings = Settings::default();
    let cost = CostTracker::new();
    let perm = PermissionManager::new(PermissionMode::Default, &settings);
    let live = LiveSession::with_projects(
        settings,
        std::env::temp_dir(),
        cost,
        perm,
        ProjectRegistry::new(),
    );

    let rule = PermissionRule::legacy(
        Some("Read".into()),
        None,
        PermissionAction::Allow,
        PermissionScope::Project { name: "ghost".into() },
    );
    let err = live
        .persist_project_rule("ghost", &rule, Some(&dir))
        .unwrap_err();
    assert_eq!(err.kind(), std::io::ErrorKind::NotFound);
}

#[test]
fn switch_project_signals_mcp_reconnect_pending() {
    let settings = Settings::default();
    let cost = CostTracker::new();
    let perm = PermissionManager::new(PermissionMode::Default, &settings);

    let mut reg = ProjectRegistry::new();
    reg.insert(ProjectConfig::new("alpha", PathBuf::from("/tmp/alpha")));
    let live = LiveSession::with_projects(settings, std::env::temp_dir(), cost, perm, reg);

    // Initially no signal.
    assert!(!live.take_mcp_reconnect_pending());

    live.switch_project("alpha").expect("switch");
    // Take consumes — first read True, second read False.
    assert!(live.take_mcp_reconnect_pending());
    assert!(!live.take_mcp_reconnect_pending());

    // Clearing the active project should also signal a reconnect (project-only
    // servers must drop).
    live.clear_active_project();
    assert!(live.take_mcp_reconnect_pending());
}

#[test]
fn active_project_mcp_specs_returns_project_servers() {
    use claurst_core::config::McpServerConfig;
    use std::collections::BTreeMap;

    let settings = Settings::default();
    let cost = CostTracker::new();
    let perm = PermissionManager::new(PermissionMode::Default, &settings);

    let mut reg = ProjectRegistry::new();
    let mut alpha = ProjectConfig::new("alpha", PathBuf::from("/tmp/alpha"));
    let mut servers = BTreeMap::new();
    servers.insert(
        "docs-rag".to_string(),
        McpServerConfig {
            name: "docs-rag".to_string(),
            command: Some("docs-rag".into()),
            args: vec![],
            env: Default::default(),
            url: None,
            server_type: "stdio".to_string(),
        },
    );
    alpha.mcp_servers = servers;
    reg.insert(alpha);

    let live = LiveSession::with_projects(settings, std::env::temp_dir(), cost, perm, reg);

    // No active project → empty.
    assert!(live.active_project_mcp_specs().is_empty());

    live.switch_project("alpha").expect("switch");
    let specs = live.active_project_mcp_specs();
    assert_eq!(specs.len(), 1);
    assert_eq!(specs[0].name, "docs-rag");
}

#[test]
fn add_rule_routes_project_scope_to_bucket() {
    use claurst_core::permissions::{
        PermissionAction, PermissionRule, PermissionScope,
    };

    let settings = Settings::default();
    let mut perm = PermissionManager::new(PermissionMode::Default, &settings);
    perm.add_rule(PermissionRule::legacy(
        Some("Read".into()),
        None,
        PermissionAction::Allow,
        PermissionScope::Project { name: "alpha".into() },
    ));
    // The bucket exists and contains the rule. The persistent + session
    // buckets stay empty.
    assert!(perm.persistent_rules.is_empty());
    assert!(perm.session_rules.is_empty());
    let bucket = perm.project_rules_for("alpha");
    assert_eq!(bucket.len(), 1);
    assert_eq!(bucket[0].tool_name.as_deref(), Some("Read"));
}
