//! opencode's delivery declaration (opencode design D1, D4, D10): a push channel claimed as
//! `cooperative_mod_delivery` that wakes the session itself with the content-free wake, and the
//! per-binding text choices. Included from `src/harness/opencode/delivery.rs`.
use super::*;
use crate::harness::{adapter::HarnessAdapter, opencode::OpencodeAdapter, registry::builtins};
use crate::notification::policy::{MARKER, WAKE_UP};
use crate::protocol::authority::{
    COOPERATIVE_ENV_SESSION_PROVENANCE, COOPERATIVE_TOP_LEVEL_PROVENANCE,
};

#[test]
fn the_declaration_is_a_self_waking_push_channel() {
    let policy = OpencodeAdapter.delivery_policy().expect("declared");
    assert_eq!(policy.mechanism(), DeliveryMechanism::PushChannel);
    assert_eq!(policy.provenance(), COOPERATIVE_MOD_DELIVERY_PROVENANCE);
    assert_eq!(policy.idle_wake(), IdleWake::Channel);
    assert_eq!(policy.wake_text(), WAKE_UP);
    // The registry accepted the pairing (it validates every builtin) and lists it as a push id.
    assert!(builtins().push_channel_ids().contains(&"opencode"));
    assert_eq!(
        builtins()
            .delivery_policy("opencode")
            .map(|p| p.provenance()),
        Some(COOPERATIVE_MOD_DELIVERY_PROVENANCE)
    );
}

#[test]
fn no_composer_no_local_witness_no_hook_step_no_turn_edge() {
    assert!(OpencodeAdapter.composer_policy().is_none());
    assert!(OpencodeAdapter.hook_step().is_none());
    assert!(OpencodeAdapter.launch_policy().is_none(), "ht-vxdp.5");
    let input = crate::harness::adapter::HookInput {
        bytes: br#"{"event":"UserPromptSubmit","session_id":"ses_a","cwd":"/w"}"#.to_vec(),
        registered_event: Some("UserPromptSubmit".into()),
    };
    let event = OpencodeAdapter
        .decode(&crate::harness::opencode::OpencodeAdmission, &input)
        .unwrap();
    assert!(OpencodeAdapter.turn_state_edge(&event).is_none());
    let request = crate::harness::adapter::LocalEvidenceRequest {
        input: &input,
        home: None,
        state_dir: None,
        pane_session: &|| None,
        deadline: std::time::Instant::now(),
    };
    assert_eq!(
        OpencodeAdapter.local_evidence(&request),
        crate::harness::adapter::LocalEvidence::default()
    );
}

/// D10: a hook-registered binding gets its items from the plugin (channel or turn hook) and a
/// content-free wake; a binding with only plain CLI evidence keeps the `inbox` marker and the
/// `N pending` hint.
#[test]
fn binding_text_follows_the_hook_registered_provenance() {
    let registry = builtins();
    assert_eq!(
        registry.binding_wake_text("opencode", Some(COOPERATIVE_TOP_LEVEL_PROVENANCE)),
        WAKE_UP
    );
    assert!(registry.binding_turn_delivery("opencode", Some(COOPERATIVE_TOP_LEVEL_PROVENANCE)));
    for provenance in [Some(COOPERATIVE_ENV_SESSION_PROVENANCE), None] {
        assert_eq!(
            registry.binding_wake_text("opencode", provenance),
            MARKER,
            "{provenance:?}"
        );
        assert!(
            !registry.binding_turn_delivery("opencode", provenance),
            "{provenance:?}"
        );
    }
}
