//! Test-only synchronization and fault fixtures; absent from ordinary builds.
pub mod failpoints;
pub mod history;
#[cfg(feature = "test-support")]
pub mod search_barrier;
#[cfg(feature = "test-support")]
pub mod server_completion;
