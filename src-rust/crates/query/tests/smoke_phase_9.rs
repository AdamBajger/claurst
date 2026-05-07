//! Phase 9 smoke tests — extended PermissionScope, PermissionSubject matching,
//! TaskSource enum, KairosPermissionPolicy.

use claurst_core::{
    CommandPattern, InputMatcher, KairosPermissionPolicy, PathMode, PermissionAction,
    PermissionRequest, PermissionRule, PermissionScope, PermissionSubject,
    SerializedPermissionRule, Shell, TaskSource, UrlPattern,
};

fn req(tool: &str, ctx: Option<&str>, read_only: bool) -> PermissionRequest {
    PermissionRequest {
        tool_name: tool.to_string(),
        description: ctx.unwrap_or("").to_string(),
        details: None,
        is_read_only: read_only,
        context_description: ctx.map(String::from),
    }
}

#[test]
fn permission_scope_serde_alias_persistent_loads_as_forever() {
    // Old settings.json wrote `"Persistent"`; new code spells it `Forever`.
    let json = r#""Persistent""#;
    let scope: PermissionScope = serde_json::from_str(json).expect("parse");
    assert_eq!(scope, PermissionScope::Forever);

    let written = serde_json::to_string(&PermissionScope::Forever).unwrap();
    assert_eq!(written, "\"Forever\"");
}

#[test]
fn permission_scope_project_serde_round_trip() {
    let scope = PermissionScope::Project { name: "alpha".into() };
    let json = serde_json::to_string(&scope).unwrap();
    let back: PermissionScope = serde_json::from_str(&json).unwrap();
    assert_eq!(back, scope);
}

#[test]
fn permission_scope_once_round_trip() {
    let scope = PermissionScope::Once;
    let json = serde_json::to_string(&scope).unwrap();
    assert_eq!(json, "\"Once\"");
    let back: PermissionScope = serde_json::from_str(&json).unwrap();
    assert_eq!(back, scope);
}

#[test]
fn legacy_serialized_rule_loads_with_fresh_id_and_timestamp() {
    // Old on-disk rules carry no id / created_at; serde defaults must fill in.
    let serialized = SerializedPermissionRule {
        tool_name: Some("Bash".into()),
        path_pattern: None,
        action: PermissionAction::Allow,
        id: None,
        subject: None,
    };
    let rule: PermissionRule = (&serialized).into();
    assert!(rule.subject.is_none());
    assert_eq!(rule.scope, PermissionScope::Forever);
    // id is a random uuid, just confirm it's not all-zero.
    assert_ne!(rule.id, uuid::Uuid::nil());
}

#[test]
fn permission_rule_legacy_constructor_assigns_id() {
    let r = PermissionRule::legacy(
        Some("Bash".into()),
        None,
        PermissionAction::Allow,
        PermissionScope::Session,
    );
    assert_ne!(r.id, uuid::Uuid::nil());
    assert!(r.subject.is_none());
}

// ---- PermissionSubject matching ---------------------------------------------

#[test]
fn subject_tool_matches_by_name() {
    let subj = PermissionSubject::Tool { name: "Bash".into() };
    assert!(subj.matches_request(&req("Bash", Some("echo hi"), false)));
    assert!(!subj.matches_request(&req("Read", Some("/etc/hosts"), true)));
}

#[test]
fn subject_tool_input_contains_substring() {
    let subj = PermissionSubject::ToolInput {
        name: "Bash".into(),
        input_match: InputMatcher::Contains("rm -rf".into()),
    };
    assert!(subj.matches_request(&req("Bash", Some("rm -rf /tmp/x"), false)));
    assert!(!subj.matches_request(&req("Bash", Some("ls -la"), false)));
    // Tool name must also match.
    assert!(!subj.matches_request(&req("Read", Some("rm -rf /tmp/x"), false)));
}

#[test]
fn subject_tool_input_any_matches_any_input_for_that_tool() {
    let subj = PermissionSubject::ToolInput {
        name: "Bash".into(),
        input_match: InputMatcher::Any,
    };
    assert!(subj.matches_request(&req("Bash", Some("anything"), false)));
    assert!(!subj.matches_request(&req("Read", Some("anything"), true)));
}

#[test]
fn subject_path_respects_mode() {
    let subj = PermissionSubject::Path {
        path: std::path::PathBuf::from("/tmp/secret"),
        mode: PathMode::Write,
    };
    // Write request mentioning the path matches.
    assert!(subj.matches_request(&req("Write", Some("write file: /tmp/secret/x"), false)));
    // Read request to same path doesn't match Write-mode subject.
    assert!(!subj.matches_request(&req("Read", Some("read: /tmp/secret/x"), true)));
}

#[test]
fn subject_path_any_mode_matches_either() {
    let subj = PermissionSubject::Path {
        path: std::path::PathBuf::from("/etc/hosts"),
        mode: PathMode::Any,
    };
    assert!(subj.matches_request(&req("Read", Some("read: /etc/hosts"), true)));
    assert!(subj.matches_request(&req("Write", Some("write: /etc/hosts"), false)));
}

#[test]
fn subject_url_glob_match() {
    let subj = PermissionSubject::Url {
        pattern: UrlPattern("https://api.example.com/*".into()),
    };
    let r = req("WebFetch", Some("fetch: https://api.example.com/v1/users"), false);
    assert!(subj.matches_request(&r));
    let r2 = req("WebFetch", Some("fetch: https://evil.com/v1/users"), false);
    assert!(!subj.matches_request(&r2));
}

#[test]
fn subject_command_shell_filter() {
    let subj = PermissionSubject::Command {
        shell: Shell::Bash,
        pattern: CommandPattern("*echo*".into()),
    };
    assert!(subj.matches_request(&req("Bash", Some("echo hi"), false)));
    // Wrong shell → no match even if pattern would match.
    assert!(!subj.matches_request(&req("PowerShell", Some("echo hi"), false)));
}

#[test]
fn subject_command_any_shell() {
    let subj = PermissionSubject::Command {
        shell: Shell::Any,
        pattern: CommandPattern("*git*".into()),
    };
    assert!(subj.matches_request(&req("Bash", Some("git status"), false)));
    assert!(subj.matches_request(&req("PowerShell", Some("git push"), false)));
    assert!(!subj.matches_request(&req("Bash", Some("ls"), false)));
}

#[test]
fn subject_composite_requires_all_to_match() {
    let subj = PermissionSubject::Composite(vec![
        PermissionSubject::Tool { name: "Bash".into() },
        PermissionSubject::ToolInput {
            name: "Bash".into(),
            input_match: InputMatcher::Contains("git".into()),
        },
    ]);
    assert!(subj.matches_request(&req("Bash", Some("git status"), false)));
    // Tool matches but input doesn't.
    assert!(!subj.matches_request(&req("Bash", Some("ls"), false)));
    // Input matches but tool doesn't.
    assert!(!subj.matches_request(&req("Read", Some("git history.txt"), true)));
}

// ---- PermissionRule::matches_request ----------------------------------------

#[test]
fn rule_with_subject_dispatches_through_subject() {
    let mut rule = PermissionRule::legacy(
        Some("Read".into()),  // legacy field — should be ignored when subject set
        None,
        PermissionAction::Allow,
        PermissionScope::Session,
    );
    rule.subject = Some(PermissionSubject::Tool { name: "Bash".into() });

    // Subject says Bash → matches Bash even though legacy tool_name says Read.
    assert!(rule.matches_request(&req("Bash", None, false)));
    assert!(!rule.matches_request(&req("Read", None, true)));
}

#[test]
fn rule_without_subject_uses_legacy_match() {
    let rule = PermissionRule::legacy(
        Some("Bash".into()),
        None,
        PermissionAction::Allow,
        PermissionScope::Session,
    );
    assert!(rule.matches_request(&req("Bash", Some("echo hi"), false)));
    assert!(!rule.matches_request(&req("Read", Some("anything"), true)));
}

// ---- TaskSource + KairosPermissionPolicy ------------------------------------

#[test]
fn task_source_serde_round_trip() {
    let cases = vec![
        TaskSource::MainSession,
        TaskSource::SlashCommand("model".into()),
        TaskSource::Cron("nightly-build".into()),
        TaskSource::Proactive,
        TaskSource::Agent("docs-rag".into()),
        TaskSource::BgLoop("btw-1".into()),
        TaskSource::System,
    ];
    for s in cases {
        let json = serde_json::to_string(&s).unwrap();
        let back: TaskSource = serde_json::from_str(&json).unwrap();
        assert_eq!(back, s);
    }
}

#[test]
fn kairos_policy_default_is_defer() {
    assert_eq!(
        KairosPermissionPolicy::default(),
        KairosPermissionPolicy::DeferToUser
    );
}

#[test]
fn kairos_policy_from_env_str_recognizes_aliases() {
    assert_eq!(
        KairosPermissionPolicy::from_env_str("defer"),
        KairosPermissionPolicy::DeferToUser
    );
    assert_eq!(
        KairosPermissionPolicy::from_env_str("DEFER"),
        KairosPermissionPolicy::DeferToUser
    );
    assert_eq!(
        KairosPermissionPolicy::from_env_str("read"),
        KairosPermissionPolicy::AutoAllowRead
    );
    assert_eq!(
        KairosPermissionPolicy::from_env_str("auto_allow_read"),
        KairosPermissionPolicy::AutoAllowRead
    );
    assert_eq!(
        KairosPermissionPolicy::from_env_str("reject"),
        KairosPermissionPolicy::Reject
    );
    // Unknown → safe default.
    assert_eq!(
        KairosPermissionPolicy::from_env_str("nonsense"),
        KairosPermissionPolicy::DeferToUser
    );
}

// ---- evaluate_with_source: source-aware policy routing --------------------

use claurst_core::config::PermissionMode;
use claurst_core::permissions::{PermissionDecision, PermissionManager};
use claurst_core::{AgentConfig, Settings};

fn fresh_manager() -> PermissionManager {
    PermissionManager::new(PermissionMode::Default, &Settings::default())
}

#[test]
fn evaluate_with_source_foreground_unchanged_by_policy() {
    let m = fresh_manager();
    let r = req("Bash", Some("echo hi"), false);
    // MainSession is foreground → policy ignored, base Ask preserved.
    let d = m.evaluate_with_source(&r, Some(&TaskSource::MainSession), KairosPermissionPolicy::Reject);
    assert!(matches!(d, PermissionDecision::Ask { .. }));
}

#[test]
fn evaluate_with_source_background_reject_collapses_ask_to_deny() {
    let m = fresh_manager();
    let r = req("Bash", Some("echo hi"), false);
    let d = m.evaluate_with_source(
        &r,
        Some(&TaskSource::Cron("nightly".into())),
        KairosPermissionPolicy::Reject,
    );
    assert_eq!(d, PermissionDecision::Deny);
}

#[test]
fn evaluate_with_source_background_defer_keeps_ask() {
    let m = fresh_manager();
    let r = req("Bash", Some("echo hi"), false);
    let d = m.evaluate_with_source(
        &r,
        Some(&TaskSource::Proactive),
        KairosPermissionPolicy::DeferToUser,
    );
    assert!(matches!(d, PermissionDecision::Ask { .. }));
}

#[test]
fn evaluate_with_source_background_auto_allow_read_only_when_read_only() {
    let m = fresh_manager();
    // is_read_only = true; default level for "Bash" is Execute → would Ask.
    let r = req("Bash", Some("inspect: cat /tmp/x"), true);
    let d = m.evaluate_with_source(
        &r,
        Some(&TaskSource::Agent("docs-rag".into())),
        KairosPermissionPolicy::AutoAllowRead,
    );
    assert_eq!(d, PermissionDecision::Allow);
}

#[test]
fn evaluate_with_source_background_auto_allow_does_not_help_writes() {
    let m = fresh_manager();
    let r = req("Bash", Some("write: rm /tmp/x"), false);
    let d = m.evaluate_with_source(
        &r,
        Some(&TaskSource::BgLoop("btw-1".into())),
        KairosPermissionPolicy::AutoAllowRead,
    );
    // Writes still prompt.
    assert!(matches!(d, PermissionDecision::Ask { .. }));
}

#[test]
fn evaluate_with_source_explicit_allow_rule_beats_policy() {
    let mut m = fresh_manager();
    m.add_session_allow("Bash");
    let r = req("Bash", Some("echo hi"), false);
    let d = m.evaluate_with_source(
        &r,
        Some(&TaskSource::Cron("any".into())),
        // Reject would normally Deny, but an explicit Allow rule wins first.
        KairosPermissionPolicy::Reject,
    );
    assert_eq!(d, PermissionDecision::Allow);
}

#[test]
fn evaluate_with_source_no_source_treated_as_foreground() {
    let m = fresh_manager();
    let r = req("Bash", Some("echo hi"), false);
    let d = m.evaluate_with_source(&r, None, KairosPermissionPolicy::Reject);
    assert!(matches!(d, PermissionDecision::Ask { .. }));
}

// ---- PendingPermission source attribution --------------------------------

#[test]
fn register_pending_with_source_round_trips_via_snapshot() {
    let mut m = fresh_manager();
    let _rx = m.register_pending_with_source(
        "tool-use-1".into(),
        Some(TaskSource::Cron("nightly".into())),
    );
    let _rx2 = m.register_pending("tool-use-2".into());
    let snap = m.pending_snapshot();
    assert_eq!(snap.len(), 2);
    assert_eq!(snap[0].0, "tool-use-1");
    assert_eq!(snap[0].1, Some(TaskSource::Cron("nightly".into())));
    // Legacy register_pending stores no source.
    assert_eq!(snap[1].0, "tool-use-2");
    assert_eq!(snap[1].1, None);
}

// ---- AgentConfig.kairos_policy field --------------------------------------

#[test]
fn agent_config_kairos_policy_serde_round_trip() {
    let mut cfg = AgentConfig::default();
    cfg.kairos_policy = Some(KairosPermissionPolicy::Reject);
    let json = serde_json::to_string(&cfg).unwrap();
    let back: AgentConfig = serde_json::from_str(&json).unwrap();
    assert_eq!(back.kairos_policy, Some(KairosPermissionPolicy::Reject));
}

#[test]
fn agent_config_kairos_policy_default_none() {
    let cfg = AgentConfig::default();
    assert!(cfg.kairos_policy.is_none());
}

#[test]
fn agent_config_legacy_json_without_kairos_policy_loads_as_none() {
    // Existing settings.json has no `kairos_policy` field — must deserialize.
    let json = r#"{"description":null,"model":null,"temperature":null,"access":"full","visible":true,"max_turns":null,"color":null}"#;
    let cfg: AgentConfig = serde_json::from_str(json).expect("parse");
    assert!(cfg.kairos_policy.is_none());
}
