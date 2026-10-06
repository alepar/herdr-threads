//! Registered operational parser handles, independent of executable metadata.
//!
//! A handle selects one declared schema. It is neither a runtime witness nor
//! a qualification for optional native capabilities.

/// Codex's registered HooksV1 input contract.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CodexContract {
    _registered: (),
}

impl CodexContract {
    pub const fn registered() -> Self {
        Self { _registered: () }
    }
}

/// Claude's registered core hooks contract. Compact remains unqualified.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ClaudeContract {
    _registered: (),
}

impl ClaudeContract {
    pub const fn registered() -> Self {
        Self { _registered: () }
    }
}
