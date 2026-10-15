//! The node's own identity: key custody, public names and the `node.json`
//! hint (cross-node sync spec, node identity design). Core and
//! harness-neutral; it depends on nothing in store, sync or daemon, only on
//! paths handed in by the caller and on the shared base62 helper in
//! `protocol::ids`.
pub mod identity;
pub mod json;
pub mod tag;

pub use identity::{NodeIdentity, NodeKeyError};
pub use json::NodeJson;
pub use tag::{NodeTag, NodeUuid, RetagDecision, on_collision, tag_for};
