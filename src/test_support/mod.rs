//! Test-only synchronization and fault fixtures; absent from ordinary builds.
pub mod failpoints;
pub mod history;
pub mod owner_watch;
#[cfg(feature = "test-support")]
pub mod search_barrier;
#[cfg(feature = "test-support")]
pub mod server_completion;
