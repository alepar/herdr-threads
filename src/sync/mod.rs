//! Cross-node synchronisation: the per-origin journal, its envelopes and the
//! clocks that order them.
//!
//! Core and harness-neutral: nothing here names a harness, and nothing in this
//! tree touches the store directly. Store-facing code lives in `src/store/`.

pub mod hlc;
