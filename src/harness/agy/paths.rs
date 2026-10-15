//! agy filesystem locations, conversation-id validation and the hook argv contract.
use crate::harness::{
    adapter::{SetupEnvironment, SetupFailure},
    context::Harness,
    registry::{OccupantHarness, builtins},
    setup,
};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};

/// The three hook events the owned plugin registers. agy has no tool-event registration.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgyEvent {
    SessionStart,
    PreInvocation,
    Stop,
}
impl AgyEvent {
    pub const ALL: [Self; 3] = [Self::SessionStart, Self::PreInvocation, Self::Stop];
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::SessionStart => "SessionStart",
            Self::PreInvocation => "PreInvocation",
            Self::Stop => "Stop",
        }
    }
    pub fn parse(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|event| event.as_str() == name)
    }
}

/// Where agy keeps its data, derived from HOME only. Payload paths are never used for file checks.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgyRoots {
    /// `~/.gemini` (the adapter's default config root).
    pub config_root: PathBuf,
    /// `~/.gemini/antigravity-cli` (`brain/`, `presence/`, `settings.json`).
    pub data: PathBuf,
    /// `~/.gemini/config` (`config.json`, `plugins/`).
    pub config: PathBuf,
    /// `~/.gemini/config/plugins/herdr-threads`.
    pub plugin_dir: PathBuf,
    /// `~/.gemini/antigravity-cli/settings.json` (the owned `permissions.allow` member lives here).
    pub settings: PathBuf,
    /// `~/.gemini/config/config.json` (plugin enablement).
    pub plugins_config: PathBuf,
}
impl AgyRoots {
    pub fn from_home(home: &Path) -> Self {
        Self::from_config_root(home.join(".gemini"))
    }
    pub fn from_config_root(config_root: PathBuf) -> Self {
        let data = config_root.join("antigravity-cli");
        let config = config_root.join("config");
        Self {
            plugin_dir: config.join("plugins").join("herdr-threads"),
            settings: data.join("settings.json"),
            plugins_config: config.join("config.json"),
            config_root,
            data,
            config,
        }
    }
}

/// A conversation id is path material: `[0-9A-Za-z_-]{1,128}` before any path use.
pub fn valid_conversation_id(id: &str) -> bool {
    (1..=128).contains(&id.len())
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
}

/// ht-poh.5's per-conversation state file: `<state>/conversations/<first 32 hex of sha256(id)>.json`.
/// `state` is `<node_dir>/adapter-state/agy/`; the content and pruning belong to ht-poh.5.
/// A delivery cache only: never a lifecycle or role input (ht-poh.30).
pub fn conversation_state(state: &Path, conversation_id: &str) -> PathBuf {
    let digest = Sha256::digest(conversation_id.as_bytes());
    let name: String = digest[..16].iter().map(|b| format!("{b:02x}")).collect();
    state.join("conversations").join(format!("{name}.json"))
}

/// The registered hook command for one event:
/// `'<exe>' '--state-dir' S '--host-endpoint' H 'hook-event' 'agy' '<E>'`.
pub fn hook_command(env: &SetupEnvironment, event: AgyEvent) -> Result<String, SetupFailure> {
    let invalid = |detail: &str| SetupFailure::Invalid(format!("agy: {detail}"));
    let executable = env
        .executable
        .to_str()
        .ok_or_else(|| invalid("herdr-threads executable path is not UTF-8"))?;
    let state = env
        .state_dir
        .as_deref()
        .ok_or_else(|| invalid("state directory unknown"))?
        .to_str()
        .ok_or_else(|| invalid("state directory path is not UTF-8"))?;
    let host = env
        .host_endpoint
        .as_deref()
        .ok_or_else(|| invalid("Herdr host endpoint unknown"))?
        .to_str()
        .ok_or_else(|| invalid("host endpoint path is not UTF-8"))?;
    let harness: Harness = OccupantHarness::Agent(
        builtins()
            .agent("agy")
            .map_err(|error| SetupFailure::Invalid(error.to_string()))?,
    )
    .into();
    let argv = crate::cli::hook::installed_event_argv(
        executable,
        Some(state),
        Some(host),
        harness,
        event.as_str(),
    )
    .map_err(|_| invalid("hook command is not valid"))?;
    setup::native_event_command(&argv).map_err(|_| invalid("hook command is not valid"))
}
