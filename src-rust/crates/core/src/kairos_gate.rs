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
    /// Frozen at `resolve_runtime_state` time so `/kairos` can explain *why*
    /// a gate is on or off without re-reading env/flags mid-session.
    pub diagnostics: KairosGateDiagnostics,
}

/// Per-input breakdown that fed into gate decisions. Use `format_summary` to
/// render a human-readable status block.
#[derive(Debug, Clone, Default)]
pub struct KairosGateDiagnostics {
    pub brief_feature_compiled: bool,
    pub channels_feature_compiled: bool,
    pub env_kairos: bool,
    pub env_brief: bool,
    pub env_channels: bool,
    pub env_proactive: bool,
    pub trusted: bool,
    pub trust_bypass: bool,
    pub forced: bool,
    pub require_entitlement: bool,
    pub entitlement_ok: bool,
}

impl KairosGateDiagnostics {
    /// Human-readable multi-line summary suitable for `/kairos` output.
    pub fn format_summary(&self) -> String {
        let yn = |b: bool| if b { "yes" } else { "no" };
        format!(
            "  brief_feature_compiled = {}\n  \
               channels_feature_compiled = {}\n  \
               env KAIROS={} KAIROS_BRIEF={} KAIROS_CHANNELS={} KAIROS_PROACTIVE={}\n  \
               trusted = {} (trust_bypass={})\n  \
               forced = {} require_entitlement = {} entitlement_ok = {}",
            yn(self.brief_feature_compiled),
            yn(self.channels_feature_compiled),
            yn(self.env_kairos),
            yn(self.env_brief),
            yn(self.env_channels),
            yn(self.env_proactive),
            yn(self.trusted),
            yn(self.trust_bypass),
            yn(self.forced),
            yn(self.require_entitlement),
            yn(self.entitlement_ok),
        )
    }
}

impl Default for KairosRuntimeState {
    fn default() -> Self {
        Self {
            brief_enabled: false,
            channels_enabled: false,
            proactive_enabled: false,
            entitlement_checked: false,
            entitled: false,
            diagnostics: KairosGateDiagnostics::default(),
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
        diagnostics: KairosGateDiagnostics {
            brief_feature_compiled: brief_feature_enabled,
            channels_feature_compiled: channels_feature_enabled,
            env_kairos,
            env_brief,
            env_channels,
            env_proactive,
            trusted,
            trust_bypass,
            forced,
            require_entitlement,
            entitlement_ok,
        },
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

/// Tick interval for proactive mode, in seconds.
/// Read from KAIROS_PROACTIVE_INTERVAL_SECS; defaults to 900 (15 min); clamped to [60, 3600].
pub fn proactive_interval_secs() -> u64 {
    std::env::var("KAIROS_PROACTIVE_INTERVAL_SECS")
        .ok()
        .and_then(|s| s.parse::<u64>().ok())
        .unwrap_or(900)
        .clamp(60, 3600)
}

/// Per-tick spend ceiling for proactive mode, in USD.
/// Read from `KAIROS_TICK_MAX_USD`; returns `None` if unset/invalid/non-positive,
/// meaning no ceiling. A single tick whose delta exceeds this value counts as an
/// overrun and contributes to the shutdown strike counter in `proactive_ticker`.
pub fn proactive_tick_max_usd() -> Option<f64> {
    std::env::var("KAIROS_TICK_MAX_USD")
        .ok()
        .and_then(|s| s.parse::<f64>().ok())
        .filter(|v| *v > 0.0)
}

/// Prompt sent to the model on each proactive tick.
/// This is the user-turn message; the system prompt already carries the Kairos addendum.
pub fn proactive_tick_prompt() -> String {
    "Kairos proactive tick. Review the current working directory and session context, then take \
     the most useful autonomous action available: run pending tasks, check for relevant changes \
     (git status, test results, file modifications), or execute scheduled work. If nothing \
     requires action, send a Brief notification with your current status. Keep all output \
     concise."
        .to_string()
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
