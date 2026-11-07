//! Identity boundaries. Ordinary structural resolution is distinct from native caller authority.
pub mod caller;
pub mod process_continuity;
pub mod reconcile;
pub mod repair;
pub mod shell_continuity;

pub use repair::OrdinaryIdentity;
