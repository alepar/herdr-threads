//! Cross-tree integration sweep (ht-4is.12): the installed executable and its
//! detached daemon against a private Herdr protocol endpoint. No model runs;
//! stand-in seats use the cooperative caller flags exactly as the isolated
//! host suite (`tests/native/recovery`) does.
/// ht-4is.8.17: `read --follow`, IRC style for a person, JSON lines otherwise.
#[path = "integration/follow.rs"]
mod follow;
/// ht-4is.6.7: read-only latency under a multi-agent party with a slow host.
#[path = "integration/latency.rs"]
mod latency;
/// ht-4is.8.9/ht-4is.8.10: a person's own pane identity (`me init`) and
/// `--pane` names, through the installed executable and the same private
/// Herdr endpoint.
#[path = "integration/operator_ux.rs"]
mod operator_ux;
#[path = "integration/sweep.rs"]
mod sweep;
/// ht-4is.8.18: compact agent-facing machine text (short cursors, one line
/// per message, no page blobs), measured through the installed executable.
#[path = "integration/token_diet.rs"]
mod token_diet;
/// ht-rzi.8: B5 trust-policy guards end to end, with a stand-in Herdr that
/// restarts as its own process (a new incarnation).
#[path = "integration/trust_policy.rs"]
mod trust_policy;
