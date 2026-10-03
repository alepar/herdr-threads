//! Pure summary core (design spec §2-§3; owner ht-1ip.3): the job-bundle
//! renderer and `chunking_version`, the deterministic chunker, index-aligned
//! rollup addressing with the displayed-cover algorithm, and bundle bounding.
//! No SQLite here: the store-side loader lives in `store::summary`.
//!
//! Thread summary ledger logic (design spec §5, §6): pure computation over the
//! wire types in `protocol::summary`. No I/O: the store layer persists the
//! records these modules produce and consumes the folds they compute.
pub mod chunk;
pub mod cover;
pub mod fold;
pub mod identifiers;
pub mod ledger;
pub mod render;
pub mod validate;
