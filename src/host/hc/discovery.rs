//! `HcDiscovery`: finds running herdr-core cores from their state files.
//!
//! Reads `$XDG_STATE_HOME|~/.local/state/herdr-core/*/core.json`, derives each
//! core's socket by the core's own rules, sends `hello` and keeps the core
//! only when `hello`'s `core.id` equals the `core.json` id; cores are deduped
//! by id. It never runs `hc` (`hc server status` would autostart a core) and
//! never attaches: a scan only reports addresses and witnesses. It is used
//! when no caller site names a core. Environment is read through a lookup
//! closure, never the process environment.

use super::{HcBoot, HcClient, HcHost};
use crate::host::continuity::LocalEndpointWitness;
use serde::Deserialize;
use std::{
    collections::BTreeSet,
    ffi::OsString,
    path::{Path, PathBuf},
};

/// Environment lookup: the process environment in production, a map in tests.
pub type EnvLookup<'a> = &'a dyn Fn(&str) -> Option<OsString>;

/// A core that answered `hello` with the id its `core.json` names.
#[derive(Debug, Clone, PartialEq)]
pub struct DiscoveredCore {
    pub id: String,
    pub session: String,
    pub socket: PathBuf,
    pub boot: HcBoot,
    pub witness: LocalEndpointWitness,
}

#[derive(Deserialize)]
struct CoreJson {
    #[serde(default)]
    id: String,
}

pub struct HcDiscovery;

impl HcDiscovery {
    /// The verified cores, ordered by session name, one per core id.
    pub fn scan(env: EnvLookup<'_>) -> Vec<DiscoveredCore> {
        let Some(state_home) = state_home(env) else {
            return Vec::new();
        };
        let socket_override = socket_override(env);
        let candidates: Vec<(String, String, PathBuf)> = sessions(&state_home)
            .into_iter()
            .map(|(session, id)| {
                let socket = socket_for(env, &state_home, &session, socket_override.as_deref());
                (session, id, socket)
            })
            .collect();
        HcHost::run(async move {
            let mut found: Vec<DiscoveredCore> = Vec::new();
            let mut seen = BTreeSet::new();
            for (session, id, socket) in candidates {
                if seen.contains(&id) || !socket.exists() {
                    continue;
                }
                let Ok(reply) = HcClient::new(&socket).hello().await else {
                    continue;
                };
                if reply.value.core.id != id {
                    continue;
                }
                let Some(boot) = HcBoot::from_hello(&reply.value) else {
                    continue;
                };
                seen.insert(id.clone());
                found.push(DiscoveredCore {
                    id,
                    session,
                    socket,
                    boot,
                    witness: reply.witness,
                });
            }
            Ok(found)
        })
        .unwrap_or_default()
    }
}

fn nonempty(env: EnvLookup<'_>, name: &str) -> Option<PathBuf> {
    env(name).filter(|v| !v.is_empty()).map(PathBuf::from)
}

fn home(env: EnvLookup<'_>) -> Option<PathBuf> {
    nonempty(env, "HOME")
}

fn state_home(env: EnvLookup<'_>) -> Option<PathBuf> {
    let base =
        nonempty(env, "XDG_STATE_HOME").or_else(|| home(env).map(|h| h.join(".local/state")))?;
    Some(base.join("herdr-core"))
}

/// `HC_SOCKET`, else `socket` in the core's `config.toml`.
fn socket_override(env: EnvLookup<'_>) -> Option<PathBuf> {
    if let Some(socket) = nonempty(env, "HC_SOCKET") {
        return Some(socket);
    }
    let config = nonempty(env, "XDG_CONFIG_HOME")
        .or_else(|| home(env).map(|h| h.join(".config")))?
        .join("herdr-core")
        .join("config.toml");
    let text = std::fs::read_to_string(config).ok()?;
    let doc = text.parse::<toml_edit::DocumentMut>().ok()?;
    let socket = doc.get("socket")?.as_str()?;
    (!socket.is_empty()).then(|| PathBuf::from(socket))
}

/// The socket the core for `session` listens on.
fn socket_for(
    env: EnvLookup<'_>,
    state_home: &Path,
    session: &str,
    chosen: Option<&Path>,
) -> PathBuf {
    if let Some(socket) = chosen {
        return socket.to_path_buf();
    }
    let run = nonempty(env, "XDG_RUNTIME_DIR")
        .or_else(|| Some(PathBuf::from(format!("/run/user/{}", uid()))).filter(|p| p.is_dir()));
    match run {
        Some(run) => run.join("herdr-core").join(format!("{session}.sock")),
        None => state_home.join(session).join(format!("{session}.sock")),
    }
}

fn uid() -> u32 {
    // SAFETY: getuid has no preconditions and cannot fail.
    unsafe { libc::getuid() }
}

/// `(session, core.json id)` for every readable state dir, by session name.
fn sessions(state_home: &Path) -> Vec<(String, String)> {
    let Ok(entries) = std::fs::read_dir(state_home) else {
        return Vec::new();
    };
    let mut found: Vec<(String, String)> = entries
        .flatten()
        .filter_map(|entry| {
            let session = entry.file_name().into_string().ok()?;
            let bytes = std::fs::read(entry.path().join("core.json")).ok()?;
            let core: CoreJson = serde_json::from_slice(&bytes).ok()?;
            (!core.id.is_empty()).then_some((session, core.id))
        })
        .collect();
    found.sort();
    found
}

#[cfg(all(test, feature = "test-support"))]
#[path = "../../../tests/host/hc_discovery.rs"]
mod tests;
