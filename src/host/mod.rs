//! Herdr host boundary: the production `NativeCli` HostPort (Herdr 0.9.1/0.9.3
//! protocol 22) and its observation parsing.
mod compatibility;
pub mod native;
pub mod observation;
mod transport;

pub mod continuity;
pub use transport::{WitnessedResponse, request_witnessed};
