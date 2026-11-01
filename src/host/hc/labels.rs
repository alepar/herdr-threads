//! The hc label vocabulary and the `<session>:tN` address format, defined once.
//!
//! Responsibility
//! - Owns: the label keys herdr-core terminals carry that this system reads or
//!   writes, and [`HcAddress`], the `<session>:tN` display/selector format.
//! - Hides: nothing; these are the names the readers (ht-kqrt.43) and writers
//!   (ht-kqrt.55, ht-kqrt.57) share.
//! - Must not: be spelled as literals elsewhere; interpret a label as
//!   continuity evidence. `ht.bootstrap` is a hint only (H3).
//! - Role: value types and constants.

use std::fmt;

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
