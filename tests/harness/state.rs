//! Contract-first state derivation (ht-xoc.5). Each test names the mutation
//! it kills.
use super::*;
use crate::harness::manifest::{Manifest, ManifestRow};

const HARNESS: &str = "claude";
const CONTRACT: &str = "0123456789abcdef";

fn evidence(version: &str) -> EvidenceRow {
    EvidenceRow {
        harness: HARNESS.into(),
        version: version.into(),
        contract_id: CONTRACT.into(),
        first_seen_at: 1_000,
        lifecycle_ok_at: None,
        tool_ok_at: None,
        violation_at: None,
        violation_event: None,
        violation_field: None,
        last_seen_at: 1_000,
    }
}

fn unverified(version: &str) -> EvidenceRow {
    let mut row = evidence(version);
    row.lifecycle_ok_at = Some(1_000);
    row
}

fn verified(version: &str) -> EvidenceRow {
    let mut row = unverified(version);
    row.tool_ok_at = Some(1_001);
    row
}

fn violated(mut row: EvidenceRow) -> EvidenceRow {
    row.violation_at = Some(1_002);
    row.violation_event = Some("PreToolUse".into());
    row.violation_field = Some("tool_input.command".into());
    row
}

fn manifest_row(
    version: &str,
    status: RowStatus,
    contract: Option<&str>,
    source: RowSource,
) -> ManifestRow {
    ManifestRow {
        harness: HARNESS.into(),
        version: version.into(),
        status: Some(status),
        evidence: Some(RowEvidence::Live),
        contract_id: contract.map(str::to_owned),
        source: Some(source),
        supported_since: None,
        broken_event: (status == RowStatus::KnownBroken).then(|| "SessionStart".into()),
        broken_field: (status == RowStatus::KnownBroken).then(|| "source".into()),
        last_working: None,
        issue_url: None,
    }
}

fn input<'a>(
    version: &'a str,
    ladder: Ladder,
    local: Option<&'a EvidenceRow>,
    manifest: Option<&'a ManifestRow>,
    pointers: &'a ReleasePointers,
) -> StateInput<'a> {
    StateInput {
        harness: HARNESS,
        version,
        contract_id: CONTRACT,
        ladder,
        local,
        manifest_status: manifest,
        other_contract_verified: None,
        pointers,
        newest_verified_here_below: None,
        running_release: "0.5.0",
    }
}

fn cause_word(state: &State) -> &'static str {
    match state {
        State::Working(WorkingSource::Local) => "working:local",
        State::Working(WorkingSource::Manifest { .. }) => "working:manifest",
        State::Working(WorkingSource::Recipe) => "working:recipe",
        State::New => "new",
        State::Broken(Broken { cause, .. }) => match cause {
            BrokenCause::LocalViolation { .. } => "broken:violation",
            BrokenCause::BelowFloor { .. } => "broken:below-floor",
            BrokenCause::ManifestKnownBroken { .. } => "broken:manifest",
            BrokenCause::RecipeKnownBroken { .. } => "broken:recipe",
        },
    }
}

fn floor() -> Ladder {
    Ladder::BelowFloor {
        min: "2.1.283".into(),
    }
}

fn recipe_broken() -> Ladder {
    Ladder::RecipeKnownBroken {
        range: ">= 2.1.290, <= 2.1.291".into(),
        newest_working: Some("2.1.287".into()),
    }
}

/// Kills: any reordering of the derivation (violation vs floor vs local
/// verified vs manifest/recipe known_broken vs manifest verified vs listed),
/// over every combination of the three inputs.
#[test]
fn derive_table() {
    let none: Option<EvidenceRow> = None;
    let locals: [(&str, Option<EvidenceRow>); 5] = [
        ("none", none),
        ("ok-unverified", Some(unverified("2.1.286"))),
        ("verified", Some(verified("2.1.286"))),
        ("violation", Some(violated(unverified("2.1.286")))),
        ("verified+violation", Some(violated(verified("2.1.286")))),
    ];
    let ladders: [(&str, Ladder); 4] = [
        ("admitted", Ladder::Admitted),
        ("listed", Ladder::Listed),
        ("below-floor", floor()),
        ("recipe-broken", recipe_broken()),
    ];
    let manifests: [(&str, Option<ManifestRow>); 3] = [
        ("none", None),
        (
            "verified",
            Some(manifest_row(
                "2.1.286",
                RowStatus::Verified,
                Some(CONTRACT),
                RowSource::Canary,
            )),
        ),
        (
            "known_broken",
            Some(manifest_row(
                "2.1.286",
                RowStatus::KnownBroken,
                Some(CONTRACT),
                RowSource::Canary,
            )),
        ),
    ];
    let pointers = ReleasePointers::default();
    let mut checked = 0;
    for (local_name, local) in &locals {
        for (ladder_name, ladder) in &ladders {
            for (manifest_name, manifest) in &manifests {
                let derived = derive(&input(
                    "2.1.286",
                    ladder.clone(),
                    local.as_ref(),
                    manifest.as_ref(),
                    &pointers,
                ));
                let violation = local.as_ref().is_some_and(|r| r.violation_at.is_some());
                let local_verified = local.as_ref().is_some_and(|r| r.verified()) && !violation;
                let below_floor = matches!(ladder, Ladder::BelowFloor { .. });
                let recipe_broken = matches!(ladder, Ladder::RecipeKnownBroken { .. });
                let manifest_status = manifest.as_ref().and_then(|row| row.status);
                let expected = if violation {
                    "broken:violation"
                } else if below_floor {
                    "broken:below-floor"
                } else if recipe_broken {
                    "broken:recipe"
                } else if local_verified {
                    "working:local"
                } else if manifest_status == Some(RowStatus::KnownBroken) {
                    "broken:manifest"
                } else if manifest_status == Some(RowStatus::Verified) {
                    "working:manifest"
                } else if matches!(ladder, Ladder::Listed) {
                    "working:recipe"
                } else {
                    "new"
                };
                assert_eq!(
                    cause_word(&derived.state),
                    expected,
                    "local={local_name} ladder={ladder_name} manifest={manifest_name}"
                );
                // A broken verdict always carries an action; below the floor
                // it is only ever "upgrade the harness".
                if let State::Broken(broken) = &derived.state {
                    assert_eq!(
                        broken.action == Action::UpgradeHarness,
                        below_floor && !violation,
                        "local={local_name} ladder={ladder_name} manifest={manifest_name}"
                    );
                }
                // Notes appear exactly when local evidence overrides a manifest
                // break, or when a recipe break outranks local evidence.
                let expects_notes = (expected == "working:local"
                    && manifest_status == Some(RowStatus::KnownBroken))
                    || (expected == "broken:recipe" && local_verified);
                assert_eq!(
                    !derived.doctor_notes.is_empty(),
                    expects_notes,
                    "local={local_name} ladder={ladder_name} manifest={manifest_name}"
                );
                checked += 1;
            }
        }
    }
    assert_eq!(checked, 60);
}

fn broken_manifest() -> ManifestRow {
    manifest_row(
        "2.1.286",
        RowStatus::KnownBroken,
        Some(CONTRACT),
        RowSource::Canary,
    )
}

fn action_of(derived: &Derived) -> Action {
    match &derived.state {
        State::Broken(broken) => broken.action.clone(),
        other => panic!("expected broken, got {other:?}"),
    }
}

/// Kills: an upgrade action offered when the other contract's release is not
/// newer than the running one (nothing to upgrade to), a `v` prefix that is
/// not stripped, and an upgrade chosen over a pin when it applies.
#[test]
fn upgrade_action_from_other_contract_supported_since() {
    let broken = broken_manifest();
    let mut other = manifest_row(
        "2.1.286",
        RowStatus::Verified,
        Some("fedcba9876543210"),
        RowSource::Canary,
    );
    other.supported_since = Some("v0.6.0".into());
    let pointers = ReleasePointers {
        last_working: Some("2.1.283".into()),
        ..Default::default()
    };
    let mut state = input("2.1.286", Ladder::Admitted, None, Some(&broken), &pointers);
    state.other_contract_verified = Some(&other);
    assert_eq!(
        action_of(&derive(&state)),
        Action::UpgradeHerdrThreads { to: "0.6.0".into() }
    );
    for not_newer in ["0.5.0", "0.4.9"] {
        let mut older = other.clone();
        older.supported_since = Some(not_newer.into());
        let mut state = input("2.1.286", Ladder::Admitted, None, Some(&broken), &pointers);
        state.other_contract_verified = Some(&older);
        assert_eq!(
            action_of(&derive(&state)),
            Action::Pin {
                max: "2.1.283".into()
            },
            "supported_since {not_newer} is not newer than the running release"
        );
    }
}

/// Kills: a pin to the manifest's last_working that ignores a newer locally
/// verified version, and a pin without a version.
#[test]
fn pin_action_from_newest_verified_here() {
    let broken = broken_manifest();
    let pointers = ReleasePointers {
        last_working: Some("2.1.283".into()),
        ..Default::default()
    };
    let mut state = input("2.1.286", Ladder::Admitted, None, Some(&broken), &pointers);
    state.newest_verified_here_below = Some("2.1.285".into());
    assert_eq!(
        action_of(&derive(&state)),
        Action::Pin {
            max: "2.1.285".into()
        }
    );
}

/// Kills: a last_working pointer that is ignored (falls through to report).
#[test]
fn pin_action_from_manifest_last_working() {
    let broken = broken_manifest();
    let pointers = ReleasePointers {
        last_working: Some("2.1.283".into()),
        ..Default::default()
    };
    let state = input("2.1.286", Ladder::Admitted, None, Some(&broken), &pointers);
    assert_eq!(
        action_of(&derive(&state)),
        Action::Pin {
            max: "2.1.283".into()
        }
    );
    // A recipe known_broken range supplies the recipe's newest working version.
    let empty = ReleasePointers::default();
    let state = input("2.1.290", recipe_broken(), None, None, &empty);
    assert_eq!(
        action_of(&derive(&state)),
        Action::Pin {
            max: "2.1.287".into()
        }
    );
}

/// Kills: a report URL taken from somewhere other than the row.
#[test]
fn report_action_from_row_issue_url() {
    let mut broken = broken_manifest();
    broken.issue_url = Some("https://example.test/issues/7".into());
    let pointers = ReleasePointers::default();
    let state = input("2.1.286", Ladder::Admitted, None, Some(&broken), &pointers);
    assert_eq!(
        action_of(&derive(&state)),
        Action::Report {
            url: "https://example.test/issues/7".into()
        }
    );
}

/// Kills: no action at all when nothing else is known.
#[test]
fn report_action_falls_back_to_repo_issues_url() {
    let broken = broken_manifest();
    let pointers = ReleasePointers::default();
    let state = input("2.1.286", Ladder::Admitted, None, Some(&broken), &pointers);
    assert_eq!(
        action_of(&derive(&state)),
        Action::Report {
            url: ISSUES_URL.into()
        }
    );
    let violation = violated(unverified("2.1.286"));
    let state = input(
        "2.1.286",
        Ladder::Admitted,
        Some(&violation),
        None,
        &pointers,
    );
    assert_eq!(
        action_of(&derive(&state)),
        Action::Report {
            url: ISSUES_URL.into()
        }
    );
}

/// Kills: local proof that beats a recipe known_broken range (the hook's B6
/// ladder refuses it), and a missing doctor note.
#[test]
fn recipe_known_broken_with_local_verified_is_broken_with_note() {
    let local = verified("2.1.290");
    let derived = derive(&input(
        "2.1.290",
        recipe_broken(),
        Some(&local),
        None,
        &ReleasePointers::default(),
    ));
    assert!(
        matches!(
            derived.state,
            State::Broken(Broken {
                cause: BrokenCause::RecipeKnownBroken { .. },
                ..
            })
        ),
        "{:?}",
        derived.state
    );
    assert_eq!(
        derived.doctor_notes,
        vec![
            "it has worked here, but the recipe tables mark >= 2.1.290, <= 2.1.291 known broken and the hook refuses it"
        ]
    );
    let manifest = broken_manifest();
    let derived = derive(&input(
        "2.1.286",
        Ladder::Admitted,
        Some(&verified("2.1.286")),
        Some(&manifest),
        &ReleasePointers::default(),
    ));
    assert_eq!(
        derived.doctor_notes,
        vec!["the canary reports a break in SessionStart/source; it has worked here"]
    );
}

/// Kills: a source that changes the verdict (only the display differs).
#[test]
fn manual_and_canary_rows_derive_identically() {
    let pointers = ReleasePointers::default();
    for status in [RowStatus::Verified, RowStatus::KnownBroken] {
        let canary = manifest_row("2.1.286", status, Some(CONTRACT), RowSource::Canary);
        let manual = manifest_row("2.1.286", status, Some(CONTRACT), RowSource::Manual);
        let a = derive(&input(
            "2.1.286",
            Ladder::Admitted,
            None,
            Some(&canary),
            &pointers,
        ));
        let b = derive(&input(
            "2.1.286",
            Ladder::Admitted,
            None,
            Some(&manual),
            &pointers,
        ));
        assert_eq!(cause_word(&a.state), cause_word(&b.state));
        assert_ne!(a.state, b.state, "the source is carried for display");
        assert_ne!(source_text(&a.state), source_text(&b.state));
    }
}

/// Kills: an unlisted newer version refused (or reported broken) by the real
/// recipe tables, a below-floor version that is not, and a listed version
/// that is not listed.
#[test]
fn unlisted_newer_version_is_admitted_not_refused() {
    for harness in ["claude", "codex"] {
        assert_eq!(
            ladder_for(harness, "999.0.0"),
            Ladder::Admitted,
            "{harness}"
        );
        assert!(
            matches!(ladder_for(harness, "0.0.1"), Ladder::BelowFloor { .. }),
            "{harness}"
        );
    }
    assert_eq!(ladder_for("claude", "2.1.286"), Ladder::Listed);
    assert_eq!(ladder_for("codex", "0.158.0"), Ladder::Listed);
    let Ladder::BelowFloor { min } = ladder_for("claude", "1.0.0") else {
        panic!("1.0.0 is below the claude floor");
    };
    assert_eq!(
        min,
        crate::harness::claude::RECIPES
            .iter()
            .map(|r| r.min_version())
            .min()
            .unwrap()
            .to_string()
    );
    let derived = derive(&input(
        "999.0.0",
        ladder_for("claude", "999.0.0"),
        None,
        None,
        &ReleasePointers::default(),
    ));
    assert_eq!(derived.state, State::New);
}

/// Kills: any drift in the operator-facing line formats.
#[test]
fn line_formats_are_pinned() {
    let upgrade = Action::UpgradeHerdrThreads { to: "0.6.0".into() };
    let pin = Action::Pin {
        max: "2.1.283".into(),
    };
    let report = Action::Report {
        url: "https://example.test/i".into(),
    };
    let violation = |action| Broken {
        cause: BrokenCause::LocalViolation {
            event: "PreToolUse".into(),
            field: "tool_input.command".into(),
        },
        action,
    };
    assert_eq!(
        broken_line("claude", "2.1.286", &violation(upgrade)),
        "harness claude 2.1.286 broken: PreToolUse payload field tool_input.command is missing \
         or has the wrong type; upgrade herdr-threads to 0.6.0 (supports claude 2.1.286)"
    );
    assert_eq!(
        broken_line("claude", "2.1.286", &violation(pin.clone())),
        "harness claude 2.1.286 broken: PreToolUse payload field tool_input.command is missing \
         or has the wrong type; pin claude to <= 2.1.283"
    );
    let manifest = |source| Broken {
        cause: BrokenCause::ManifestKnownBroken {
            event: "SessionStart".into(),
            field: "source".into(),
            source,
            issue_url: None,
        },
        action: report.clone(),
    };
    assert_eq!(
        broken_line("codex", "0.170.0", &manifest(RowSource::Canary)),
        "harness codex 0.170.0 broken: the canary manifest row reports SessionStart payload \
         field source; report: https://example.test/i"
    );
    assert_eq!(
        broken_line("codex", "0.170.0", &manifest(RowSource::Manual)),
        "harness codex 0.170.0 broken: the manual manifest row reports SessionStart payload \
         field source; report: https://example.test/i"
    );
    assert_eq!(
        broken_line(
            "claude",
            "2.1.290",
            &Broken {
                cause: BrokenCause::RecipeKnownBroken {
                    range: ">= 2.1.290".into()
                },
                action: pin,
            }
        ),
        "harness claude 2.1.290 broken: known broken in >= 2.1.290; pin claude to <= 2.1.283"
    );
    assert_eq!(
        broken_line(
            "claude",
            "2.0.1",
            &Broken {
                cause: BrokenCause::BelowFloor {
                    min: "2.1.283".into()
                },
                action: Action::UpgradeHarness,
            }
        ),
        "claude 2.0.1 is below the supported floor 2.1.283; upgrade claude"
    );
}

/// Kills: a source label that drops the evidence level or the event/field.
#[test]
fn source_texts_are_pinned() {
    assert_eq!(
        source_text(&State::Working(WorkingSource::Local)),
        "local evidence (lifecycle + tool payloads)"
    );
    assert_eq!(
        source_text(&State::Working(WorkingSource::Manifest {
            source: RowSource::Canary,
            evidence: Some(RowEvidence::NoModel)
        })),
        "canary manifest row (no_model)"
    );
    assert_eq!(
        source_text(&State::Working(WorkingSource::Manifest {
            source: RowSource::Manual,
            evidence: None
        })),
        "manual manifest row"
    );
    assert_eq!(
        source_text(&State::Working(WorkingSource::Recipe)),
        "recipe tables"
    );
    assert_eq!(source_text(&State::New), "no evidence yet");
}

fn row_at(version: &str, contract: &str, first: u64, last: u64) -> EvidenceRow {
    let mut row = verified(version);
    row.contract_id = contract.into();
    row.first_seen_at = first;
    row.last_seen_at = last;
    row
}

/// Kills: choosing by first_seen (a downgrade keeps the newer contract
/// deciding), by version or by the lexicographically first id.
#[test]
fn newest_contract_is_the_one_seen_most_recently() {
    let rows = [
        row_at("2.1.286", "aaaa", 100, 9_000),
        row_at("2.1.285", "bbbb", 500, 600),
        row_at("2.1.287", "bbbb", 200, 700),
    ];
    assert_eq!(newest_contract(&rows), Some("aaaa"));
    // Downgrade: bbbb (newer contract, first seen later) was in use, then the
    // older binary's hooks touched aaaa again.
    let downgrade = [
        row_at("2.1.286", "aaaa", 100, 5_000),
        row_at("2.1.286", "bbbb", 1_000, 4_000),
    ];
    assert_eq!(newest_contract(&downgrade), Some("aaaa"));
    let tie = [
        row_at("2.1.286", "aaaa", 100, 1),
        row_at("2.1.286", "bbbb", 100, 1),
    ];
    assert_eq!(newest_contract(&tie), Some("bbbb"));
    assert_eq!(newest_contract(&[]), None);
}

/// Kills: a roll-up that evaluates rows of an older contract, or keeps a
/// version with no session in the window.
#[test]
fn roll_up_evaluates_the_newest_contract_inside_the_window() {
    let day = HEALTH_WINDOW_MS;
    let now = 10 * day;
    let mut recent_violation = violated(row_at("2.1.286", "new", 5 * day, now - 10));
    recent_violation.violation_at = Some(now - 10);
    let stale_violation = violated(row_at("2.1.285", "new", 5 * day, now - 2 * day));
    let old_contract = violated(row_at("2.1.287", "old", day, now - 3 * day));
    let rollup = roll_up(
        HARNESS,
        &[recent_violation, stale_violation, old_contract],
        &Manifest::default(),
        "0.5.0",
        now,
    );
    assert_eq!(rollup.contract_id.as_deref(), Some("new"));
    let versions: Vec<_> = rollup
        .versions
        .iter()
        .map(|v| (v.version.as_str(), v.in_health_window))
        .collect();
    assert_eq!(versions, [("2.1.286", true), ("2.1.285", false)]);
    let line = rollup
        .health_line()
        .expect("the recent violation is broken");
    assert!(
        line.starts_with("harness claude 2.1.286 broken: "),
        "{line}"
    );
}

#[test]
fn exact_build_ladder_never_orders_informational_release_text() {
    use crate::harness::{
        registry::builtins,
        runtime::{RuntimeDescriptor, RuntimeIdentity},
    };
    let registry = builtins();
    for harness in ["claude", "codex"] {
        let registration = registry.by_id(registry.agent(harness).unwrap()).unwrap();
        let build = RuntimeIdentity::build(RuntimeDescriptor {
            release_version: Some("0.0.1".into()),
            source: "git".into(),
            base_version: Some("0.0.1".into()),
            derived_version: Some("0.0.1+dev".into()),
            commit: None,
            dirty: Some(true),
            distance: Some(1),
        })
        .unwrap();
        assert_eq!(registration.version_ladder(&build), Ladder::Admitted);
        let release = RuntimeIdentity::stable_release("0.0.1", "native_transcript").unwrap();
        assert!(matches!(
            registration.version_ladder(&release),
            Ladder::BelowFloor { .. }
        ));
    }
}

// Catches stale descriptor credit, semver promotion of build metadata and loss of sticky violation.
#[test]
fn exact_runtime_verdict_requires_current_descriptor_and_never_collapses_build() {
    use crate::{
        harness::{
            registry,
            runtime::{RuntimeDescriptor, RuntimeIdentity},
        },
        protocol::results::RuntimeEvidenceState,
        store::harness_evidence::EvidenceRowV2,
    };
    let registration = registry::builtins()
        .by_id(registry::builtins().agent("claude").unwrap())
        .unwrap();
    let descriptor = &registration.contracts()[0];
    let identity = RuntimeIdentity::build(RuntimeDescriptor {
        release_version: None,
        source: "fixture".into(),
        base_version: Some("2.1.286".into()),
        derived_version: Some("2.1.286+7.gabcdef0".into()),
        commit: Some("a".repeat(40)),
        dirty: Some(false),
        distance: Some(7),
    })
    .unwrap();
    let mut row = EvidenceRowV2 {
        harness: "claude".into(),
        identity: identity.clone(),
        domain: descriptor.domain_id.into(),
        origin: descriptor.origin,
        contract_id: descriptor.contract_id_v2().unwrap(),
        first_seen_at: 1,
        last_seen_at: 2,
        milestones: Default::default(),
        violation_at: None,
        violation_event: None,
        violation_field: None,
    };
    let derive = |row: &EvidenceRowV2| {
        derive_runtime(
            registration,
            &identity,
            &row.domain,
            row.origin,
            &row.contract_id,
            Some(row),
            &Manifest::default(),
        )
    };
    assert_eq!(
        derive(&row).state,
        RuntimeEvidenceState::New,
        "build base version must not earn recipe Working"
    );
    for milestone in descriptor.required_milestones {
        row.milestones.insert((*milestone).into(), 1);
    }
    assert_eq!(derive(&row).state, RuntimeEvidenceState::Working);
    row.violation_at = Some(2);
    row.violation_event = Some("PreToolUse".into());
    row.violation_field = Some("tool_input".into());
    assert_eq!(derive(&row).state, RuntimeEvidenceState::Broken);
    row.contract_id = "0000000000000000".into();
    assert_eq!(
        derive(&row).state,
        RuntimeEvidenceState::Unavailable,
        "historical contract must not become a current verdict"
    );
}
