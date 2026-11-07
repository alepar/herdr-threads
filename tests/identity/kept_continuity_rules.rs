//! ht-dtx0: how a kept continuity claim's outcomes are read. Mounted from
//! `src/identity/repair.rs`. Kills: a site-based rule that drops a claim
//! refused for writer admission, budget or a raced read; a replay read error
//! that drops an undecided claim.
use super::*;

fn refused(code: ErrorCode) -> ApiError {
    ApiError::new(code, "test")
}

fn reattached() -> CommandResult {
    CommandResult::ContinuityReattached(crate::protocol::results::ContinuityReattachment {
        seat: crate::protocol::ids::SeatId::new("s"),
        binding_generation: 1,
        fast_forward: None,
    })
}

#[test]
fn transient_refusals_keep_the_claim_and_definitive_ones_do_not() {
    for code in [
        ErrorCode::ServiceBusy,
        ErrorCode::StoreBusy,
        ErrorCode::DeadlineExceeded,
        ErrorCode::StaleHostObservation,
        ErrorCode::HostUnavailable,
    ] {
        assert!(
            continuity_not_ready(&Err(refused(code.clone()))),
            "{code:?}"
        );
    }
    for code in [
        ErrorCode::NotFound,
        ErrorCode::Conflict,
        ErrorCode::TargetAlreadyOwned,
        ErrorCode::InvalidRequest,
        ErrorCode::LivenessFingerprintRequired,
        // A key conflict's refusal and a claimless key are definitive: never kept (ht-ma6i.2).
        ErrorCode::TargetUnresolved,
        ErrorCode::ContinuityKeyPending,
    ] {
        assert!(
            !continuity_not_ready(&Err(refused(code.clone()))),
            "{code:?}"
        );
    }
    assert!(!continuity_not_ready(&Ok(reattached())));
}

#[test]
fn a_replay_settles_only_a_recorded_decision_or_a_definitive_refusal() {
    assert_eq!(replay_settles(&Ok(Some(reattached()))), Some(true));
    assert_eq!(replay_settles(&Ok(None)), None);
    assert_eq!(
        replay_settles(&Err(refused(ErrorCode::OperationPayloadMismatch))),
        Some(true)
    );
    for code in [
        ErrorCode::StoreBusy,
        ErrorCode::DeadlineExceeded,
        ErrorCode::ServiceBusy,
    ] {
        assert_eq!(
            replay_settles(&Err(refused(code.clone()))),
            Some(false),
            "{code:?}"
        );
    }
}

/// Kills: a kept keyed claim decided without its key, at intake or by the observation worker.
#[test]
fn a_claim_rests_on_its_key_when_it_presents_one() {
    let mut command = ContinuityCheckIn {
        host: crate::test_support::test_host_locator(),
        target: crate::protocol::ids::HostTargetId::new("p"),
        harness: crate::protocol::authority::Harness::Claude,
        native_session: crate::protocol::ids::NativeSessionId::new("n"),
        source: ContinuityCheckIn::RESUME_SOURCE.into(),
        operation: crate::protocol::ids::OperationId::new("op"),
        execution: crate::protocol::ids::ExecutionId::new("e"),
        liveness: None,
        continuity_key: None,
    };
    assert_eq!(
        claim_evidence(&command),
        crate::ports::ContinuityEvidence::Cooperative
    );
    command.continuity_key = Some(crate::protocol::ids::ContinuityKey::new("k"));
    assert_eq!(
        claim_evidence(&command),
        crate::ports::ContinuityEvidence::Key
    );
}
