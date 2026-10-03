use super::{ApiError, ErrorClass, ErrorCode};

/// The class every code must default to. Written out by hand (not computed)
/// so a changed mapping is a visible diff.
const EXPECTED: &[(ErrorCode, Option<ErrorClass>)] = &[
    (ErrorCode::Unsupported, None),
    (ErrorCode::InvalidRequest, None),
    (ErrorCode::UnknownWireVersion, Some(ErrorClass::VersionSkew)),
    (
        ErrorCode::DaemonVersionMismatch,
        Some(ErrorClass::VersionSkew),
    ),
    (ErrorCode::InstanceMismatch, Some(ErrorClass::Transient)),
    (ErrorCode::DaemonBootChanged, Some(ErrorClass::Transient)),
    (ErrorCode::CursorStale, Some(ErrorClass::Transient)),
    (ErrorCode::InvalidCursor, None),
    (ErrorCode::InvalidBudget, None),
    (ErrorCode::ReadBudgetExhausted, Some(ErrorClass::Transient)),
    (ErrorCode::SequenceExhausted, None),
    (ErrorCode::Unauthorized, None),
    (ErrorCode::CallerUnverified, None),
    (ErrorCode::PermitExpired, Some(ErrorClass::Transient)),
    (ErrorCode::StaleHostObservation, Some(ErrorClass::Transient)),
    (ErrorCode::TargetUnresolved, None),
    (ErrorCode::TargetUnsafe, None),
    (ErrorCode::TargetAlreadyOwned, None),
    (ErrorCode::ThreadNotOrphaned, None),
    (ErrorCode::Archived, None),
    (ErrorCode::NotFound, None),
    (ErrorCode::OperationPayloadMismatch, None),
    (ErrorCode::StoreBusy, Some(ErrorClass::Transient)),
    (ErrorCode::StoreCorrupt, Some(ErrorClass::Corrupt)),
    (ErrorCode::StoreFull, Some(ErrorClass::Transient)),
    (ErrorCode::IncompatibleSchema, Some(ErrorClass::VersionSkew)),
    (ErrorCode::HostUnavailable, Some(ErrorClass::Unavailable)),
    (ErrorCode::UnknownOutcome, Some(ErrorClass::Transient)),
    (ErrorCode::MissingHook, None),
    (ErrorCode::UnsupportedHarness, None),
    (ErrorCode::Cancelled, None),
    (ErrorCode::DeadlineExceeded, Some(ErrorClass::Transient)),
    (ErrorCode::Conflict, None),
    (ErrorCode::ServiceBusy, Some(ErrorClass::Transient)),
    (
        ErrorCode::ServiceNotRegistered,
        Some(ErrorClass::Unavailable),
    ),
    (
        ErrorCode::StaleServiceGeneration,
        Some(ErrorClass::Transient),
    ),
    (
        ErrorCode::IncompatibleOwnership,
        Some(ErrorClass::VersionSkew),
    ),
    (ErrorCode::RequiredInvitationNeedsManagedThread, None),
    (ErrorCode::MembershipRequired, None),
    (ErrorCode::StaleRequirementAcceptance, None),
    (ErrorCode::TransportDenied, Some(ErrorClass::Unavailable)),
];

#[test]
fn every_code_has_a_constructor_with_its_default_class() {
    assert_eq!(
        ErrorCode::ALL.len(),
        EXPECTED.len(),
        "ALL and EXPECTED list every code once"
    );
    for (code, class) in EXPECTED {
        assert!(ErrorCode::ALL.contains(code), "{code:?} missing from ALL");
        let error = ApiError::constructor_for(code.clone())("detail");
        assert_eq!(&error.code, code);
        assert_eq!(error.detail, "detail");
        assert_eq!(error.restart_argv, None);
        assert_eq!(error.required_minimum_bytes, None);
        assert_eq!(code.default_class(), *class, "{code:?}");
        assert_eq!(error.class(), *class, "{code:?}");
    }
}

#[test]
fn named_constructors_match_their_codes() {
    assert_eq!(ApiError::not_found("x").code, ErrorCode::NotFound);
    assert_eq!(ApiError::conflict("x").code, ErrorCode::Conflict);
    assert_eq!(
        ApiError::store_busy("x").class(),
        Some(ErrorClass::Transient)
    );
    assert_eq!(
        ApiError::store_corrupt("x").class(),
        Some(ErrorClass::Corrupt)
    );
    assert_eq!(
        ApiError::host_unavailable("x").class(),
        Some(ErrorClass::Unavailable)
    );
    assert_eq!(
        ApiError::unknown_wire_version("x").class(),
        Some(ErrorClass::VersionSkew)
    );
    let built = ApiError::invalid_budget("too big").with_required_minimum_bytes(7);
    assert_eq!(built.required_minimum_bytes, Some(7));
    let restart = ApiError::daemon_version_mismatch("x").with_restart_argv(vec!["a".into()]);
    assert_eq!(restart.restart_argv, Some(vec!["a".to_string()]));
}

/// The snake_case wire name of a code, as the serde rename produces it.
fn wire_name(code: &ErrorCode) -> String {
    serde_json::to_value(code)
        .unwrap()
        .as_str()
        .unwrap()
        .to_string()
}

/// Wire format unchanged: a constructed error serializes to the frozen
/// hand-written JSON form for every code, and decodes back equal.
#[test]
fn constructed_errors_keep_the_literal_wire_form() {
    for code in ErrorCode::ALL {
        let built = ApiError::constructor_for(code.clone())("d");
        let frozen = format!(
            r#"{{"code":"{}","detail":"d","restart_argv":null,"required_minimum_bytes":null}}"#,
            wire_name(code)
        );
        let encoded = serde_json::to_string(&built).unwrap();
        assert_eq!(encoded, frozen);
        assert_eq!(serde_json::from_str::<ApiError>(&frozen).unwrap(), built);
        assert!(
            !encoded.contains("class"),
            "class must not reach the wire: {encoded}"
        );
    }
    assert_eq!(
        serde_json::to_string(&ApiError::store_busy("d")).unwrap(),
        r#"{"code":"store_busy","detail":"d","restart_argv":null,"required_minimum_bytes":null}"#
    );
}

#[test]
fn class_override_never_reaches_the_wire() {
    let plain = ApiError::store_busy("x");
    let overridden = ApiError::store_busy("x").with_class(ErrorClass::Unavailable);
    assert_eq!(overridden.class(), Some(ErrorClass::Unavailable));
    let encoded = serde_json::to_string(&overridden).unwrap();
    assert_eq!(encoded, serde_json::to_string(&plain).unwrap());
    let decoded: ApiError = serde_json::from_str(&encoded).unwrap();
    assert_eq!(decoded.class(), Some(ErrorClass::Transient));
}
