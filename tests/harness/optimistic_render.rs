//! ht-p03.23: the optimistic-admission and known-broken labels. Each test
//! names the mutation it kills.

use super::admission::{ISSUES_URL, OptimisticAdmission, Placement};
use super::recipe::{Version, VersionSet};
use super::{known_broken_label, optimistic_label};

fn admission(placement: Placement, major: bool) -> OptimisticAdmission {
    OptimisticAdmission {
        verified_max: Version::new(2, 1, 286),
        assumed_recipe: "claude-hooks-2.1.283",
        major_version_change: major,
        placement,
        issues_url: ISSUES_URL,
    }
}

/// Kills: the two placements sharing one wording, a label that drops the
/// verified maximum (newer) or invents one (within span), and one that drops
/// the assumed recipe id.
#[test]
fn both_placements_render_their_own_wording_with_the_recipe() {
    assert_eq!(
        optimistic_label(&admission(Placement::NewerThanVerified, false), false),
        "optimistic \u{2014} newer than verified 2.1.286, assumed compatible with recipe \
         claude-hooks-2.1.283"
    );
    assert_eq!(
        optimistic_label(&admission(Placement::WithinSpan, false), false),
        "optimistic \u{2014} unlisted within the supported span, assumed compatible with recipe \
         claude-hooks-2.1.283"
    );
}

/// Kills: a major-version flag that is ignored or always on.
#[test]
fn major_version_change_is_appended_only_when_flagged() {
    let flagged = optimistic_label(&admission(Placement::NewerThanVerified, true), false);
    assert!(flagged.ends_with("; major version change"), "{flagged}");
    let plain = optimistic_label(&admission(Placement::NewerThanVerified, false), false);
    assert!(!plain.contains("major version change"), "{plain}");
}

/// Kills: the issues URL leaking into the short Health form, or missing from
/// the doctor form.
#[test]
fn issues_url_is_in_the_doctor_form_only() {
    for placement in [Placement::NewerThanVerified, Placement::WithinSpan] {
        let health = optimistic_label(&admission(placement, true), false);
        let doctor = optimistic_label(&admission(placement, true), true);
        assert!(!health.contains(ISSUES_URL), "{health}");
        assert_eq!(doctor, format!("{health} (report issues: {ISSUES_URL})"));
    }
}

/// Kills: a refusal that omits the broken range, omits the newest working
/// version, or renders a missing one as a blank.
#[test]
fn known_broken_names_the_range_and_the_newest_working_version() {
    let range = VersionSet::Interval {
        min: Some(Version::new(2, 1, 286)),
        max: None,
    };
    assert_eq!(
        known_broken_label("claude", "2.1.290", &range, Some(Version::new(2, 1, 285))),
        "claude 2.1.290: refused: known broken in [2.1.286, \u{2026}); newest working: 2.1.285"
    );
    assert_eq!(
        known_broken_label("codex", "0.160.0", &range, None),
        "codex 0.160.0: refused: known broken in [2.1.286, \u{2026}); newest working: none listed"
    );
}
