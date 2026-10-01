//! Host evidence the elected observation lane actually observed, reported by
//! Health. Nothing here is asserted: `Supported` coherent enumeration and a
//! `Ready` host exist only after a verified coherent publication in the
//! current server incarnation. A verified outcome (published or invalidated)
//! always replaces the previous one.
//!
//! Monotone evidence rule: an attempt that ends in `Err` (admission failed,
//! or a failed capture whose invalidation could not be written) has no
//! verified outcome, so it may only preserve or lower the prior evidence,
//! never raise it. It lowers a verified publication (or an earlier errored
//! attempt) to an explicit non-verified `Degraded`/`Unknown` state. Every
//! other prior (no evidence, or a verified fail-closed invalidation such as
//! `HostUnavailable`) keeps its level; the errored attempt is only appended
//! to that prior's detail.
use crate::{
    daemon::health::ComponentStatus,
    ports::HostInvalidationReason,
    protocol::{
        results::{ApiError, CapabilityState},
        time::UtcMillis,
    },
};
use std::sync::Mutex;

#[derive(Debug, Clone, PartialEq, Eq)]
enum Latest {
    Published,
    Invalidated {
        reason: HostInvalidationReason,
        detail: Option<String>,
    },
    /// The latest attempt produced no durable outcome at all.
    Errored {
        detail: String,
    },
}

#[derive(Debug, Default)]
struct State {
    latest: Option<Latest>,
    /// Errored attempts after a prior that an errored attempt must not
    /// replace (it would raise it); reported beside that prior only.
    later_error: Option<String>,
    last_reconciliation_at: Option<UtcMillis>,
}

/// Shared between the observation worker (writer) and the Health producer.
#[derive(Debug, Default)]
pub struct HostEvidenceStatus {
    state: Mutex<State>,
}

/// Host fields for `HealthInputs`, derived from observed evidence.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostEvidenceHealth {
    pub host: ComponentStatus,
    pub coherent_enumeration: CapabilityState,
    pub last_reconciliation_at: Option<UtcMillis>,
}

impl HostEvidenceStatus {
    /// A capture was durably published as a verified coherent enumeration.
    pub fn record_published(&self) {
        if let Ok(mut state) = self.state.lock() {
            state.latest = Some(Latest::Published);
            state.later_error = None;
        }
    }

    /// A capture was invalidated (fail-closed), with its host cause if any.
    pub fn record_invalidated(&self, reason: HostInvalidationReason, cause: Option<&ApiError>) {
        if let Ok(mut state) = self.state.lock() {
            state.latest = Some(Latest::Invalidated {
                reason,
                detail: cause.map(|error| format!("{:?}: {}", error.code, error.detail)),
            });
            state.later_error = None;
        }
    }

    /// The latest observation attempt ended in `Err` before any durable
    /// outcome. Monotone: it lowers a verified publication (or refreshes an
    /// earlier errored state) and otherwise preserves the prior evidence.
    pub fn record_capture_failed(&self, error: &ApiError) {
        if let Ok(mut state) = self.state.lock() {
            let detail = format!("{:?}: {}", error.code, error.detail);
            match state.latest {
                Some(Latest::Published | Latest::Errored { .. }) => {
                    state.latest = Some(Latest::Errored { detail });
                    state.later_error = None;
                }
                None | Some(Latest::Invalidated { .. }) => state.later_error = Some(detail),
            }
        }
    }

    /// Every saved-seat page of a verified publication was reconciled.
    pub fn record_reconciled(&self, at: UtcMillis) {
        if let Ok(mut state) = self.state.lock() {
            state.last_reconciliation_at = Some(at);
        }
    }

    pub fn health(&self, witness: CapabilityState) -> HostEvidenceHealth {
        let (latest, later_error, last_reconciliation_at) = match self.state.lock() {
            Ok(state) => (
                state.latest.clone(),
                state.later_error.clone(),
                state.last_reconciliation_at,
            ),
            Err(_) => {
                return HostEvidenceHealth {
                    host: ComponentStatus::Unknown,
                    coherent_enumeration: CapabilityState::Unknown,
                    last_reconciliation_at: None,
                };
            }
        };
        let (host, coherent_enumeration) = if witness == CapabilityState::Unsupported {
            (
                ComponentStatus::Unsupported(
                    "host server incarnation cannot be witnessed on this platform".into(),
                ),
                CapabilityState::Unsupported,
            )
        } else {
            match latest {
                None => (ComponentStatus::Unknown, CapabilityState::Unknown),
                Some(Latest::Published) => (ComponentStatus::Ready, CapabilityState::Supported),
                Some(Latest::Errored { detail }) => (
                    ComponentStatus::Degraded(format!(
                        "latest host capture attempt errored without a verified outcome: {detail}"
                    )),
                    CapabilityState::Unknown,
                ),
                Some(Latest::Invalidated { reason, detail }) => {
                    let detail = detail.unwrap_or_else(|| format!("{reason:?}"));
                    match reason {
                        HostInvalidationReason::HostUnavailable => (
                            ComponentStatus::Unavailable(format!(
                                "latest host capture failed: {detail}"
                            )),
                            CapabilityState::Unknown,
                        ),
                        HostInvalidationReason::UnknownIncarnation => (
                            ComponentStatus::Unsupported(
                                "latest host capture carried no verified server incarnation".into(),
                            ),
                            CapabilityState::Unsupported,
                        ),
                        HostInvalidationReason::PartialEnumeration
                        | HostInvalidationReason::CoherenceLost
                        | HostInvalidationReason::PublicationFailed => (
                            ComponentStatus::Degraded(format!(
                                "latest host capture was not a verified coherent publication: {detail}"
                            )),
                            CapabilityState::Unknown,
                        ),
                    }
                }
            }
        };
        // Same level as the prior; only its detail mentions the errored
        // attempt. `Unknown` and `Ready` carry no detail and stay as they are.
        let host = match (host, later_error) {
            (host, None) => host,
            (ComponentStatus::Unavailable(detail), Some(error)) => {
                ComponentStatus::Unavailable(with_later_error(detail, &error))
            }
            (ComponentStatus::Degraded(detail), Some(error)) => {
                ComponentStatus::Degraded(with_later_error(detail, &error))
            }
            (ComponentStatus::Unsupported(detail), Some(error)) => {
                ComponentStatus::Unsupported(with_later_error(detail, &error))
            }
            (host @ (ComponentStatus::Unknown | ComponentStatus::Ready), Some(_)) => host,
        };
        HostEvidenceHealth {
            host,
            coherent_enumeration,
            last_reconciliation_at,
        }
    }
}

fn with_later_error(detail: String, error: &str) -> String {
    format!("{detail}; a later host capture attempt errored without a verified outcome: {error}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::results::ErrorCode;

    fn unavailable() -> ApiError {
        ApiError {
            code: ErrorCode::HostUnavailable,
            detail: "socket gone".into(),
            restart_argv: None,
            required_minimum_bytes: None,
        }
    }

    /// Kills: reporting Supported/Ready before any capture (asserted rather
    /// than observed evidence).
    #[test]
    fn no_capture_yet_is_unknown() {
        let status = HostEvidenceStatus::default();
        assert_eq!(
            status.health(CapabilityState::Unknown),
            HostEvidenceHealth {
                host: ComponentStatus::Unknown,
                coherent_enumeration: CapabilityState::Unknown,
                last_reconciliation_at: None,
            }
        );
    }

    /// Kills: ignoring the adapter's static platform witness (non-macOS
    /// NativeCli), which would report Unknown/verified instead of Unsupported.
    #[test]
    fn platform_without_incarnation_witness_stays_unsupported() {
        let status = HostEvidenceStatus::default();
        let health = status.health(CapabilityState::Unsupported);
        assert!(matches!(health.host, ComponentStatus::Unsupported(_)));
        assert_eq!(health.coherent_enumeration, CapabilityState::Unsupported);
        // Even a (hypothetical) publication cannot upgrade an unsupported witness.
        status.record_published();
        assert_eq!(
            status
                .health(CapabilityState::Unsupported)
                .coherent_enumeration,
            CapabilityState::Unsupported
        );
    }

    /// Kills: a sticky Supported after a later fail-closed capture, and a
    /// reconciliation timestamp that is lost on invalidation.
    #[test]
    fn latest_outcome_wins_and_reconciliation_time_is_kept() {
        let status = HostEvidenceStatus::default();
        status.record_published();
        status.record_reconciled(UtcMillis(42));
        let verified = status.health(CapabilityState::Unknown);
        assert_eq!(verified.host, ComponentStatus::Ready);
        assert_eq!(verified.coherent_enumeration, CapabilityState::Supported);
        assert_eq!(verified.last_reconciliation_at, Some(UtcMillis(42)));

        status.record_invalidated(
            HostInvalidationReason::HostUnavailable,
            Some(&unavailable()),
        );
        let failed = status.health(CapabilityState::Unknown);
        let ComponentStatus::Unavailable(detail) = &failed.host else {
            panic!("{failed:?}");
        };
        assert!(detail.contains("socket gone"), "{detail}");
        assert_eq!(failed.coherent_enumeration, CapabilityState::Unknown);
        assert_eq!(failed.last_reconciliation_at, Some(UtcMillis(42)));

        status.record_invalidated(HostInvalidationReason::UnknownIncarnation, None);
        let unverified = status.health(CapabilityState::Unknown);
        assert!(matches!(unverified.host, ComponentStatus::Unsupported(_)));
        assert_eq!(
            unverified.coherent_enumeration,
            CapabilityState::Unsupported
        );

        status.record_invalidated(HostInvalidationReason::PartialEnumeration, None);
        let partial = status.health(CapabilityState::Unknown);
        assert!(matches!(partial.host, ComponentStatus::Degraded(_)));
        assert_eq!(partial.coherent_enumeration, CapabilityState::Unknown);

        status.record_published();
        assert_eq!(
            status.health(CapabilityState::Unknown).coherent_enumeration,
            CapabilityState::Supported
        );
    }

    /// Kills: `record_capture_failed` leaving the prior `Published` in place
    /// (errored attempt over-claims Ready/Supported), mapping the errored
    /// state to Ready/Supported, or dropping the reconciliation time.
    #[test]
    fn errored_attempt_after_publication_is_not_verified() {
        let status = HostEvidenceStatus::default();
        status.record_published();
        status.record_reconciled(UtcMillis(7));
        status.record_capture_failed(&ApiError {
            code: ErrorCode::StoreBusy,
            detail: "writer contended".into(),
            restart_argv: None,
            required_minimum_bytes: None,
        });
        let health = status.health(CapabilityState::Unknown);
        let ComponentStatus::Degraded(detail) = &health.host else {
            panic!("{health:?}");
        };
        assert!(detail.contains("errored"), "{detail}");
        assert!(detail.contains("writer contended"), "{detail}");
        assert_eq!(health.coherent_enumeration, CapabilityState::Unknown);
        assert_eq!(health.last_reconciliation_at, Some(UtcMillis(7)));
        // The platform witness still dominates.
        assert_eq!(
            status
                .health(CapabilityState::Unsupported)
                .coherent_enumeration,
            CapabilityState::Unsupported
        );
        // A later verified publication wins again.
        status.record_published();
        assert_eq!(
            status.health(CapabilityState::Unknown).host,
            ComponentStatus::Ready
        );
    }

    fn errored(detail: &str) -> ApiError {
        ApiError {
            code: ErrorCode::DeadlineExceeded,
            detail: detail.into(),
            restart_argv: None,
            required_minimum_bytes: None,
        }
    }

    /// Strength of the positive host claim Health makes. Verified negatives
    /// rank lowest; `Unknown` makes no claim; `Degraded` asserts a partly
    /// reachable host; `Ready` asserts a verified one.
    fn host_rank(host: &ComponentStatus) -> u8 {
        match host {
            ComponentStatus::Unavailable(_) | ComponentStatus::Unsupported(_) => 0,
            ComponentStatus::Unknown => 1,
            ComponentStatus::Degraded(_) => 2,
            ComponentStatus::Ready => 3,
        }
    }

    fn detail(host: &ComponentStatus) -> String {
        match host {
            ComponentStatus::Unavailable(detail)
            | ComponentStatus::Unsupported(detail)
            | ComponentStatus::Degraded(detail) => detail.clone(),
            ComponentStatus::Unknown | ComponentStatus::Ready => panic!("no detail: {host:?}"),
        }
    }

    fn enumeration_rank(state: CapabilityState) -> u8 {
        match state {
            CapabilityState::Unsupported => 0,
            CapabilityState::Unknown => 1,
            CapabilityState::Supported => 2,
        }
    }

    /// Monotone evidence rule: transition table of every prior evidence
    /// state x `Errored` (an attempt that ended in `Err` with no verified
    /// outcome). Each row states the exact Health it must produce, and no
    /// row may raise the host claim or the enumeration claim.
    ///
    /// Kills, by row:
    /// - M1 "Errored overwrites any prior" (the fix2 shape): the
    ///   `Invalidated(HostUnavailable)` row (unavailable raised to degraded),
    ///   the `Invalidated(UnknownIncarnation)` row (unsupported raised to
    ///   degraded/unknown) and the `none` row (unknown raised to degraded).
    /// - M2 "Errored never changes the prior": the `published` row (a stale
    ///   verified publication stays Ready/Supported).
    /// - M3 "Errored from no evidence records Errored": the `none` row.
    /// - M4 "Errored overwrites a verified non-coherent invalidation": the
    ///   partial/coherence-lost/publication-failed rows lose the verified
    ///   invalidation detail.
    /// - M5 "repeated Errored keeps the first detail": the `errored` row.
    /// - M6b "a verified invalidation does not clear the errored
    ///   annotation": the trailing unavailable -> errored -> partial check.
    #[test]
    fn errored_attempt_only_preserves_or_lowers_prior_evidence() {
        type Prior = (&'static str, fn(&HostEvidenceStatus));
        let priors: [Prior; 8] = [
            ("none", |_| {}),
            ("published", |s| s.record_published()),
            ("invalidated host unavailable", |s| {
                s.record_invalidated(
                    HostInvalidationReason::HostUnavailable,
                    Some(&unavailable()),
                )
            }),
            ("invalidated unknown incarnation", |s| {
                s.record_invalidated(HostInvalidationReason::UnknownIncarnation, None)
            }),
            ("invalidated partial enumeration", |s| {
                s.record_invalidated(HostInvalidationReason::PartialEnumeration, None)
            }),
            ("invalidated coherence lost", |s| {
                s.record_invalidated(HostInvalidationReason::CoherenceLost, None)
            }),
            ("invalidated publication failed", |s| {
                s.record_invalidated(HostInvalidationReason::PublicationFailed, None)
            }),
            // Only reachable from a publication: errored attempts cannot
            // create an `Errored` state from any lower prior.
            ("errored", |s| {
                s.record_published();
                s.record_capture_failed(&errored("first error"))
            }),
        ];
        for (name, prior) in priors {
            let status = HostEvidenceStatus::default();
            prior(&status);
            let before = status.health(CapabilityState::Unknown);
            status.record_capture_failed(&errored("second error"));
            let after = status.health(CapabilityState::Unknown);
            assert!(
                host_rank(&after.host) <= host_rank(&before.host),
                "{name}: errored attempt raised host {before:?} -> {after:?}"
            );
            assert!(
                enumeration_rank(after.coherent_enumeration)
                    <= enumeration_rank(before.coherent_enumeration),
                "{name}: errored attempt raised enumeration {before:?} -> {after:?}"
            );
            match name {
                // A stale verified publication (or an earlier errored
                // attempt) is reported as the latest non-verified attempt.
                "published" | "errored" => {
                    let ComponentStatus::Degraded(detail) = &after.host else {
                        panic!("{name}: {after:?}");
                    };
                    assert!(detail.contains("second error"), "{name}: {detail}");
                    assert!(!detail.contains("first error"), "{name}: {detail}");
                    assert_eq!(after.coherent_enumeration, CapabilityState::Unknown);
                }
                // No evidence stays no evidence.
                "none" => assert_eq!(after, before, "{name}"),
                // A verified fail-closed outcome is at least as low and more
                // exact: same level and same prior detail, with the errored
                // attempt only appended.
                _ => {
                    assert_eq!(
                        std::mem::discriminant(&after.host),
                        std::mem::discriminant(&before.host),
                        "{name}: {before:?} -> {after:?}"
                    );
                    assert_eq!(after.coherent_enumeration, before.coherent_enumeration);
                    assert_eq!(after.last_reconciliation_at, before.last_reconciliation_at);
                    let (before_detail, after_detail) = (detail(&before.host), detail(&after.host));
                    assert!(
                        after_detail.starts_with(&before_detail),
                        "{name}: prior evidence lost: {before_detail} -> {after_detail}"
                    );
                    assert!(
                        after_detail.contains("later host capture attempt errored")
                            && after_detail.contains("second error"),
                        "{name}: {after_detail}"
                    );
                }
            }
        }
        // The reviewed repro shape, stated directly.
        let status = HostEvidenceStatus::default();
        status.record_invalidated(
            HostInvalidationReason::HostUnavailable,
            Some(&unavailable()),
        );
        status.record_capture_failed(&errored("store call deadline exceeded"));
        let health = status.health(CapabilityState::Unknown);
        let ComponentStatus::Unavailable(detail) = &health.host else {
            panic!("errored attempt raised an unavailable host: {health:?}");
        };
        assert!(detail.contains("socket gone"), "{detail}");
        assert!(detail.contains("store call deadline exceeded"), "{detail}");
        // A later verified outcome replaces the preserved evidence and clears
        // the errored annotation.
        status.record_invalidated(HostInvalidationReason::PartialEnumeration, None);
        let ComponentStatus::Degraded(detail) = status.health(CapabilityState::Unknown).host else {
            panic!("partial enumeration must be degraded");
        };
        assert!(
            !detail.contains("errored"),
            "stale errored annotation: {detail}"
        );
        status.record_published();
        assert_eq!(
            status.health(CapabilityState::Unknown).host,
            ComponentStatus::Ready
        );
    }
}
