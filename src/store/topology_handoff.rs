//! Frozen bootstrap identity and transaction-local canonical persistence.
//!
//! The equipped daemon supplies its actual daemon-selected namespace,
//! never a namespace inferred from a caller, original intent or frozen identity.
//! Persistence/status never confer submission or receipt authority. The store-only
//! attempts API reserves one-use authorization against A2 in its deciding transaction.
//! Schema27 is registered; public dispatch requires the equipped runtime. Begin reuses
//! existing live mapping/member guards in its caller transaction.
use super::connection::api_error;
use crate::protocol::{
    handoff::{BootstrapIdentity, HandoffNamespace},
    results::{ApiError, ErrorCode},
};

/// Retained envelope ceiling, aligned with the existing delivery terminal's
/// 131072-byte bound (original journal intent remains independently 64 KiB).
/// Encode and retained reads refuse overflow; neither truncates data.
pub const MAX_IDENTITY_BYTES: usize = 128 * 1024;

/// Encode the entire validated immutable identity in its canonical namespace.
/// Frozen paths are compared as supplied bytes, without replay normalization.
pub fn encode_identity(
    canonical: &HandoffNamespace,
    identity: &BootstrapIdentity,
) -> Result<Vec<u8>, ApiError> {
    identity
        .validate()
        .map_err(|detail| api_error(ErrorCode::InvalidRequest, detail))?;
    let frozen = &identity.payload.handoff.namespace;
    if canonical.instance != frozen.instance
        || canonical.state_dir.as_os_str() != frozen.state_dir.as_os_str()
        || canonical.host_endpoint.as_os_str() != frozen.host_endpoint.as_os_str()
    {
        return Err(api_error(
            ErrorCode::InstanceMismatch,
            "bootstrap canonical namespace mismatch",
        ));
    }
    let bytes = serde_json::to_vec(identity).map_err(|_| {
        api_error(
            ErrorCode::InvalidRequest,
            "invalid bootstrap identity encoding",
        )
    })?;
    if bytes.len() > MAX_IDENTITY_BYTES {
        return Err(api_error(
            ErrorCode::InvalidRequest,
            "oversized bootstrap identity",
        ));
    }
    Ok(bytes)
}

/// Refuse corrupt retained formats separately from a different valid identity.
/// Compare the full immutable envelope, including scope, original claim, payload
/// and child keys, even when the compound UUID is copied and digest recomputed.
/// JSON object order/whitespace is immaterial; frozen path spelling is retained.
pub fn compare_identity(
    canonical: &HandoffNamespace,
    retained: &[u8],
    submitted: &BootstrapIdentity,
) -> Result<(), ApiError> {
    let submitted_bytes = encode_identity(canonical, submitted)?;
    let corrupt = || {
        api_error(
            ErrorCode::StoreCorrupt,
            "invalid retained bootstrap identity",
        )
    };
    if retained.len() > MAX_IDENTITY_BYTES {
        return Err(corrupt());
    }
    let saved: BootstrapIdentity = serde_json::from_slice(retained).map_err(|_| corrupt())?;
    saved.validate().map_err(|_| corrupt())?;
    // Some reused nested types admit unknown fields or absent Option fields.
    // Roundtrip shape equality rejects that lost data without changing the
    // protocol contracts, and accepts legal JSON object reordering/whitespace.
    let saved_value: serde_json::Value = serde_json::from_slice(retained).map_err(|_| corrupt())?;
    if saved_value != serde_json::to_value(&saved).map_err(|_| corrupt())? {
        return Err(corrupt());
    }
    // Struct equality uses PathBuf equality, which can fold path components.
    // Canonical serialization preserves every frozen path's original spelling.
    if serde_json::to_vec(&saved).map_err(|_| corrupt())? != submitted_bytes {
        return Err(api_error(
            ErrorCode::OperationPayloadMismatch,
            "retained bootstrap identity mismatch",
        ));
    }
    Ok(())
}

mod attachment;
pub mod attempts;
mod persistence;
mod runtime;
pub use attachment::{attach_created, attach_pending, complete_linked_pending};
pub(crate) use attachment::{
    begin_selected_child, guard_bare_completion, guard_child_begin, guard_linked_begin,
    guard_unscoped_child_begin, guard_unscoped_child_phase, guard_unscoped_create,
    validate_bootstrap_resolution, validate_create_command, validate_selected_child_phase,
};
pub use persistence::{
    MAX_ATTACHMENT_BYTES, MAX_COMPLETED_BYTES, MAX_CREATION_BYTES, MAX_RECOVERY_BYTES,
    begin_pending, current,
};

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::handoff::topology_contract_tests::identity;

    #[test]
    fn valid_full_identity_roundtrips_without_losing_frozen_arguments() {
        let submitted = identity();
        let canonical = &submitted.payload.handoff.namespace;
        let encoded = encode_identity(canonical, &submitted).unwrap();
        let decoded: BootstrapIdentity = serde_json::from_slice(&encoded).unwrap();
        assert_eq!(decoded.payload.launch.argv, ["--model", "fixed"]);
        assert_eq!(decoded, submitted);
        compare_identity(canonical, &encoded, &submitted).unwrap();
    }

    fn refresh(mut submitted: BootstrapIdentity) -> BootstrapIdentity {
        submitted.digest = submitted.semantic_digest().unwrap();
        submitted.validate().unwrap();
        submitted
    }

    #[test]
    fn copied_compound_refuses_foreign_canonical_namespace_bytes() {
        let submitted = identity();
        let retained = serde_json::to_vec(&submitted).unwrap();
        for (field, replacement) in [
            ("instance", "foreign"),
            ("state_dir", "/different-state"),
            ("host_endpoint", "/different.sock"),
            ("state_dir", "/state/"),
            ("host_endpoint", "//host.sock"),
        ] {
            let mut canonical = submitted.payload.handoff.namespace.clone();
            match field {
                "instance" => canonical.instance = replacement.into(),
                "state_dir" => canonical.state_dir = replacement.into(),
                _ => canonical.host_endpoint = replacement.into(),
            }
            assert_eq!(
                encode_identity(&canonical, &submitted).unwrap_err().code,
                ErrorCode::InstanceMismatch
            );
            assert_eq!(
                compare_identity(&canonical, &retained, &submitted)
                    .unwrap_err()
                    .code,
                ErrorCode::InstanceMismatch
            );
        }
    }

    #[test]
    fn same_compound_refuses_changed_full_identity_with_valid_digest() {
        let original = identity();
        let canonical = &original.payload.handoff.namespace;
        let retained = serde_json::to_vec(&original).unwrap();
        let mut changes = Vec::new();
        let mut v = original.clone();
        v.payload.handoff.body = "changed".into();
        changes.push(v);
        let mut v = original.clone();
        v.payload.launch.argv.push("--other".into());
        changes.push(v);
        let mut v = original.clone();
        v.claim.binding_generation += 1;
        changes.push(v);
        let mut v = original.clone();
        v.claim.target = crate::protocol::ids::HostTargetId::new("w2:p1");
        changes.push(v);
        let mut v = original.clone();
        v.payload.resolve_key = crate::protocol::ids::OperationId::new("new-resolve");
        changes.push(v);
        let mut v = original.clone();
        v.claim.seat = crate::protocol::ids::SeatId::new("other-sender");
        v.scope = crate::cli::journal::IntentScope::Cooperative {
            instance: "i".into(),
            seat: v.claim.seat.clone(),
        };
        changes.push(v);
        let mut v = original.clone();
        v.payload.cwd = "/cwd/".into();
        changes.push(v);
        for changed in changes.into_iter().map(refresh) {
            assert_eq!(changed.compound, original.compound);
            assert_eq!(
                compare_identity(canonical, &retained, &changed)
                    .unwrap_err()
                    .code,
                ErrorCode::OperationPayloadMismatch
            );
        }
    }

    #[test]
    fn retained_unknown_fields_and_missing_optional_fields_are_corrupt() {
        let submitted = identity();
        let canonical = &submitted.payload.handoff.namespace;
        for pointer in [
            "",
            "/scope",
            "/claim",
            "/payload/launch",
            "/payload/handoff/namespace",
        ] {
            let mut value = serde_json::to_value(&submitted).unwrap();
            value
                .pointer_mut(pointer)
                .unwrap()
                .as_object_mut()
                .unwrap()
                .insert("unknown".into(), true.into());
            assert_eq!(
                compare_identity(canonical, &serde_json::to_vec(&value).unwrap(), &submitted)
                    .unwrap_err()
                    .code,
                ErrorCode::StoreCorrupt
            );
        }
        let mut value = serde_json::to_value(&submitted).unwrap();
        value["payload"]["launch"]
            .as_object_mut()
            .unwrap()
            .remove("name");
        assert_eq!(
            compare_identity(canonical, &serde_json::to_vec(&value).unwrap(), &submitted)
                .unwrap_err()
                .code,
            ErrorCode::StoreCorrupt
        );
    }

    #[test]
    fn malformed_truncated_oversized_and_invalid_saved_identities_are_corrupt() {
        let submitted = identity();
        let canonical = &submitted.payload.handoff.namespace;
        let good = serde_json::to_vec(&submitted).unwrap();
        let mut invalid = submitted.clone();
        invalid.digest = "0".repeat(64);
        for retained in [
            vec![0xff],
            b"{}".to_vec(),
            good[..good.len() - 1].to_vec(),
            vec![b' '; MAX_IDENTITY_BYTES + 1],
            serde_json::to_vec(&invalid).unwrap(),
        ] {
            assert_eq!(
                compare_identity(canonical, &retained, &submitted)
                    .unwrap_err()
                    .code,
                ErrorCode::StoreCorrupt
            );
        }
        assert_eq!(
            encode_identity(canonical, &invalid).unwrap_err().code,
            ErrorCode::InvalidRequest
        );
        assert_eq!(
            compare_identity(canonical, &good, &invalid)
                .unwrap_err()
                .code,
            ErrorCode::InvalidRequest
        );
    }

    #[test]
    fn equivalent_json_order_and_whitespace_preserve_full_identity() {
        let submitted = identity();
        let canonical = &submitted.payload.handoff.namespace;
        let sorted = serde_json::to_value(&submitted).unwrap();
        compare_identity(
            canonical,
            &serde_json::to_vec_pretty(&sorted).unwrap(),
            &submitted,
        )
        .unwrap();
    }

    #[test]
    fn envelope_boundary_preserves_valid_full_data_and_refuses_overflow() {
        let mut submitted = identity();
        // Escaped argv stays within the protocol's 64 KiB raw argv bound.
        // This does not assert journal acceptance for a large original intent.
        submitted.payload.launch.argv = vec!["\"".repeat(4096); 16];
        let excess = serde_json::to_vec(&submitted).unwrap().len() - MAX_IDENTITY_BYTES;
        let last = submitted.payload.launch.argv.last_mut().unwrap();
        last.truncate(last.len() - excess.div_ceil(2));
        if !excess.is_multiple_of(2) {
            last.push('x');
        }
        let submitted = refresh(submitted);
        let canonical = &submitted.payload.handoff.namespace;
        let encoded = encode_identity(canonical, &submitted).unwrap();
        assert_eq!(encoded.len(), MAX_IDENTITY_BYTES);
        compare_identity(canonical, &encoded, &submitted).unwrap();
        let mut overflow = submitted.clone();
        overflow.payload.launch.argv.last_mut().unwrap().push('x');
        let overflow = refresh(overflow);
        assert_eq!(
            encode_identity(canonical, &overflow).unwrap_err().code,
            ErrorCode::InvalidRequest
        );
        assert_eq!(
            compare_identity(
                canonical,
                &serde_json::to_vec(&overflow).unwrap(),
                &submitted
            )
            .unwrap_err()
            .code,
            ErrorCode::StoreCorrupt
        );
    }
}
