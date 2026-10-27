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
