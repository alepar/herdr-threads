//! Herdr host boundary: the production `NativeCli` HostPort (Herdr 0.9.1 or
//! newer) and its observation parsing.
pub mod compatibility;
pub mod native;
pub mod observation;
mod transport;

pub mod continuity;
pub use transport::{WitnessedResponse, request_witnessed};
