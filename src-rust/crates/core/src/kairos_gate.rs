use crate::feature_flags::FeatureFlagManager;
use crate::feature_gates::is_env_truthy;
use once_cell::sync::Lazy;
use parking_lot::RwLock;
use tracing::warn;

#[derive(Debug, Clone)]
pub struct KairosRuntimeState {
    pub brief_enabled: bool,
    pub channels_enabled: bool,
    pub proactive_enabled: bool,
    pub entitlement_checked: bool,
    pub entitled: bool,
}

impl Default for KairosRuntimeState {
    fn default() -> Self {
        Self {
            brief_enabled: false,
            channels_enabled: false,
            proactive_enabled: false,
            entitlement_checked: false,
            entitled: false,
        }
    }
}

static RUNTIME_STATE: Lazy<RwLock<Option<KairosRuntimeState>>> = Lazy::new(|| RwLock::new(None));

fn env_enabled(name: &str) -> bool {
    is_env_truthy(std::env::var(name).ok().as_deref())
}

async fn check_entitlement() -> (bool, bool) {
    let manager = FeatureFlagManager::new();
    if let Err(e) = manager.fetch_flags_async().await {
        warn!(error = %e, "Kairos entitlement fetch failed; marking entitlement unavailable");
        return (false, false);
    }

    let entitled = manager.flag("kairos")
        || manager.flag("kairos_brief")
        || manager.flag("tengu_kairos_cron");
    (true, entitled)
}

pub async fn resolve_runtime_state(has_completed_onboarding: bool) -> KairosRuntimeState {
    let brief_feature_enabled = cfg!(feature = "kairos_brief");
    let channels_feature_enabled = cfg!(feature = "kairos_channels");

    let env_kairos = env_enabled("KAIROS");
    let env_brief = env_enabled("KAIROS_BRIEF") || env_kairos;
    let env_channels = env_enabled("KAIROS_CHANNELS") || env_kairos;
    let env_proactive = env_enabled("KAIROS_PROACTIVE") || env_kairos;

    let trust_bypass = env_enabled("KAIROS_TRUST_BYPASS");
    let trusted = has_completed_onboarding || trust_bypass;

    let forced = env_enabled("KAIROS_FORCE");
    let require_entitlement = env_enabled("KAIROS_REQUIRE_ENTITLEMENT");
    let (entitlement_checked, entitled) = check_entitlement().await;
    let entitlement_ok = forced || entitled || !require_entitlement;

    let brief_enabled = brief_feature_enabled && env_brief && trusted && entitlement_ok;
    let channels_enabled = channels_feature_enabled && env_channels && trusted && entitlement_ok;
    let proactive_enabled = brief_enabled && env_proactive;

    KairosRuntimeState {
        brief_enabled,
        channels_enabled,
        proactive_enabled,
        entitlement_checked,
        entitled,
    }
}

pub fn set_runtime_state(state: KairosRuntimeState) {
    *RUNTIME_STATE.write() = Some(state);
}

pub async fn initialize_runtime_state(has_completed_onboarding: bool) -> KairosRuntimeState {
    let state = resolve_runtime_state(has_completed_onboarding).await;
    set_runtime_state(state.clone());
    state
}

pub fn runtime_state() -> Option<KairosRuntimeState> {
    RUNTIME_STATE.read().clone()
}

fn require_runtime_state() -> KairosRuntimeState {
    runtime_state().expect(
        "Kairos runtime state is not initialized. Call claurst_core::kairos_gate::initialize_runtime_state() before querying Kairos gates.",
    )
}

pub fn is_kairos_brief_active() -> bool {
    require_runtime_state().brief_enabled
}

pub fn is_kairos_channels_active() -> bool {
    require_runtime_state().channels_enabled
}

pub fn is_kairos_proactive_active() -> bool {
    require_runtime_state().proactive_enabled
}

/// Assistant-mode addendum appended to the system prompt when Kairos is active.
///
/// This keeps behavior guidance centralized so CLI/bootstrap paths can apply it
/// consistently in both interactive and headless sessions.
pub fn assistant_system_prompt_addendum(proactive_enabled: bool) -> String {
    let mut lines = vec![
        "KAIROS assistant mode is active.",
        "Keep responses brief and action-oriented unless extra detail is explicitly requested.",
        "Use the Brief tool to proactively notify the user about meaningful progress or completion.",
        "If autonomous work is not required, avoid unnecessary chatter and focus on concrete outcomes.",
    ];

    if proactive_enabled {
        lines.push("Proactive mode is active: continue useful autonomous work between user turns and pace yourself deliberately.");
    }

    lines.join("\n")
}
