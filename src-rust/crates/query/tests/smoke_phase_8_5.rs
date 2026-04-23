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
