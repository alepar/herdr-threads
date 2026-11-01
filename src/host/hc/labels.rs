//! The hc label vocabulary and the `<session>:tN` address format, defined once.
//!
//! Responsibility
//! - Owns: the label keys herdr-core terminals carry that this system reads or
//!   writes, and [`HcAddress`], the `<session>:tN` display/selector format.
//!   [`AppNames`] and [`DocNames`], the `herdr-app` doc's space and tab names
//!   parsed tolerantly and cached by doc version.
//! - Hides: nothing; these are the names the readers (ht-kqrt.43) and writers
//!   (ht-kqrt.55, ht-kqrt.57) share.
//! - Must not: be spelled as literals elsewhere; interpret a label as
//!   continuity evidence. `ht.bootstrap` is a hint only (H3).
//! - Role: value types and constants.

use super::wire::AppDoc;
use serde_json::Value;
use std::collections::BTreeMap;
use std::fmt;
use std::sync::{Arc, Mutex};

/// The core document holding the app layout and its space and tab names.
pub const APP_DOC: &str = "herdr-app";
/// Workspace (space) a terminal belongs to.
pub const LABEL_SPACE: &str = "herdr.space";
/// Tab a terminal belongs to.
pub const LABEL_TAB: &str = "herdr.tab";
/// Bootstrap hint written at launch. Hint only: never evidence.
pub const LABEL_BOOTSTRAP: &str = "ht.bootstrap";
/// The label this system gives a terminal it launches.
pub const LABEL_HT: &str = "ht.label";

/// A terminal address `tN` (herdr-core ids are monotonic).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct TerminalId(String);

impl TerminalId {
    /// `t` followed by one or more ASCII digits.
    pub fn parse(text: &str) -> Option<Self> {
        let digits = text.strip_prefix('t')?;
        (!digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit()))
            .then(|| Self(text.to_owned()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// `<session>:tN`, or bare `tN` where exactly one core is attached.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct HcAddress {
    pub session: Option<String>,
    pub terminal: TerminalId,
}

impl HcAddress {
    pub fn new(session: impl Into<String>, terminal: TerminalId) -> Self {
        Self {
            session: Some(session.into()),
            terminal,
        }
    }

    /// Parses `<session>:tN` or `tN`. The session is non-empty and has no `:`.
    pub fn parse(text: &str) -> Option<Self> {
        match text.rsplit_once(':') {
            None => Some(Self {
                session: None,
                terminal: TerminalId::parse(text)?,
            }),
            Some((session, terminal)) => {
                if session.is_empty() || session.contains(':') {
                    return None;
                }
                Some(Self {
                    session: Some(session.to_owned()),
                    terminal: TerminalId::parse(terminal)?,
                })
            }
        }
    }
}

impl fmt::Display for HcAddress {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.session {
            Some(session) => write!(f, "{session}:{}", self.terminal.as_str()),
            None => f.write_str(self.terminal.as_str()),
        }
    }
}

impl HcAddress {
    /// Resolves this address against the attached `cores`. `<session>:tN` names
    /// one core; a bare `tN` is accepted only when exactly one core is attached,
    /// and is refused with every candidate when several are.
    pub fn resolve(&self, cores: &[HcCore]) -> Result<(String, TerminalId), AddressRefusal> {
        match &self.session {
            Some(session) => cores
                .iter()
                .find(|core| &core.session == session && core.has(&self.terminal))
                .map(|core| (core.session.clone(), self.terminal.clone()))
                .ok_or(AddressRefusal::NoSuchTarget),
            None => {
                let hits: Vec<&HcCore> = cores.iter().filter(|c| c.has(&self.terminal)).collect();
                match (cores.len(), hits.as_slice()) {
                    (_, []) => Err(AddressRefusal::NoSuchTarget),
                    (1, [only]) => Ok((only.session.clone(), self.terminal.clone())),
                    _ => Err(AddressRefusal::Ambiguous(
                        hits.iter()
                            .map(|c| HcAddress::new(c.session.clone(), self.terminal.clone()))
                            .collect(),
                    )),
                }
            }
        }
    }
}

/// One attached core: its session and the terminals it owns.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HcCore {
    pub session: String,
    pub terminals: Vec<TerminalId>,
}

impl HcCore {
    fn has(&self, terminal: &TerminalId) -> bool {
        self.terminals.contains(terminal)
    }
}

/// Why an address did not resolve to one core's terminal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AddressRefusal {
    NoSuchTarget,
    /// A bare `tN` that more than one attached core owns, or any bare `tN`
    /// while several cores are attached: the `<session>:tN` candidates.
    Ambiguous(Vec<HcAddress>),
}

impl fmt::Display for AddressRefusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoSuchTarget => f.write_str("no such herdr-core terminal"),
            Self::Ambiguous(candidates) => {
                f.write_str("ambiguous terminal, use one of:")?;
                for candidate in candidates {
                    write!(f, " {candidate}")?;
                }
                Ok(())
            }
        }
    }
}

/// Space and tab names of one `herdr-app` doc version. Unknown fields and
/// malformed entries are skipped; the doc's own `title`-like fields are never read.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct AppNames {
    spaces: BTreeMap<String, String>,
    /// Tab id to its name, `""` when unnamed, and its 1-based position in its space.
    tabs: BTreeMap<String, (String, usize)>,
}

impl AppNames {
    /// `{spaces: [{id, name, tabs: [{id, name}]}]}`, any part of which may be absent.
    pub fn parse(doc: &Value) -> Self {
        let mut names = Self::default();
        let text = |v: &Value, key: &str| v.get(key).and_then(Value::as_str).map(str::to_owned);
        for space in doc
            .get("spaces")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            if let Some(id) = text(space, "id")
                && let Some(name) = text(space, "name").filter(|n| !n.is_empty())
            {
                names.spaces.insert(id, name);
            }
            let tabs = space.get("tabs").and_then(Value::as_array);
            for (index, tab) in tabs.into_iter().flatten().enumerate() {
                if let Some(id) = text(tab, "id") {
                    names
                        .tabs
                        .insert(id, (text(tab, "name").unwrap_or_default(), index + 1));
                }
            }
        }
        names
    }

    pub fn space_name(&self, id: &str) -> Option<&str> {
        self.spaces.get(id).map(String::as_str)
    }

    /// The tab's name, or its 1-based position when unnamed; `None` for an unknown tab.
    pub fn tab_label(&self, id: &str) -> Option<String> {
        let (name, position) = self.tabs.get(id)?;
        Some(if name.is_empty() {
            position.to_string()
        } else {
            name.clone()
        })
    }
}

/// [`AppNames`] cached by doc version: a different version is re-parsed.
#[derive(Debug, Default)]
pub struct DocNames {
    cached: Mutex<Option<(u64, Arc<AppNames>)>>,
}

impl DocNames {
    pub fn names(&self, doc: &AppDoc) -> Arc<AppNames> {
        let mut cached = self.cached.lock().unwrap_or_else(|e| e.into_inner());
        if let Some((version, names)) = cached.as_ref()
            && *version == doc.version
        {
            return names.clone();
        }
        let names = Arc::new(AppNames::parse(&doc.value));
        *cached = Some((doc.version, names.clone()));
        names
    }
}
