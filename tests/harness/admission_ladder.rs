//! The admission-classification table: every row of the ladder, every row of
//! the harness version set, first-match order, the exactly-one-row property,
//! and the test-only recipe override.
use crate::harness::admission::{
    self, ISSUES_URL, Placement, Refusal, Row, classify, override_table,
};
use crate::harness::claude::{self, ClaudeProfile, InputSchema};
use crate::harness::codex;
use crate::harness::recipe::{Evidence, NativeSupport, Recipe, Version, VersionSet};

const fn v(major: u32, minor: u32, patch: u32) -> Version {
    Version::new(major, minor, patch)
}

#[derive(Clone, Copy, Debug)]
enum Fp {
    /// A harness without fingerprints (Claude).
    None,
    /// The binary's schemas match the recipe with this id.
    Match(&'static str),
    Differs,
    Unreadable,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    Unparsable,
    Older,
    KnownBroken,
}

#[derive(Clone, Copy, Debug)]
enum Expect {
    Refused(Kind),
    Listed(&'static str),
    SchemaMatched(&'static str),
    Optimistic {
        recipe: &'static str,
        placement: Placement,
        major: bool,
    },
}

struct Case<P: 'static> {
    table: &'static [Recipe<P>],
    version: &'static str,
    fingerprint: Fp,
    expect: Expect,
}

fn run<P: PartialEq + std::fmt::Debug>(case: &Case<P>) {
    let matching = match case.fingerprint {
        Fp::Match(id) => case.table.iter().find(|recipe| recipe.id == id),
        Fp::None | Fp::Differs | Fp::Unreadable => None,
    };
    let row = classify(case.table, case.version, || matching);
    let what = format!("{:?} {:?}", case.version, case.fingerprint);
    match (case.expect, &row) {
        (Expect::Refused(Kind::Unparsable), Row::Refused(Refusal::Unparsable))
        | (Expect::Refused(Kind::Older), Row::Refused(Refusal::OlderThanSupported(_)))
        | (Expect::Refused(Kind::KnownBroken), Row::Refused(Refusal::KnownBroken { .. })) => (),
        (Expect::Listed(id), Row::Listed(recipe)) => assert_eq!(recipe.id, id, "{what}"),
        (Expect::SchemaMatched(id), Row::SchemaMatched(recipe)) => {
            assert_eq!(recipe.id, id, "{what}");
        }
        (
            Expect::Optimistic {
                recipe: id,
                placement,
                major,
            },
            Row::Optimistic { recipe, admission },
        ) => {
            assert_eq!(recipe.id, id, "{what}");
            assert_eq!(admission.assumed_recipe, id, "{what}");
            assert_eq!(admission.placement, placement, "{what}");
            assert_eq!(admission.major_version_change, major, "{what}");
            assert_eq!(admission.issues_url, ISSUES_URL, "{what}");
            let verified = case.table.iter().map(Recipe::max_version).max().unwrap();
            assert_eq!(admission.verified_max, verified, "{what}");
        }
        (expect, row) => panic!("{what}: expected {expect:?}, got {row:?}"),
    }
}

use Expect::{Listed, Optimistic, Refused, SchemaMatched};
use Fp::{Differs, Match, Unreadable};

const CLAUDE: &str = "claude-hooks-2.1.283";
const CLAUDE_287: &str = "claude-hooks-2.1.287";
const CODEX: &str = "codex-hooks-v1";

fn claude_case(version: &'static str, expect: Expect) -> Case<ClaudeProfile> {
    Case {
        table: claude::RECIPES,
        version,
        fingerprint: Fp::None,
        expect,
    }
}

fn codex_case(version: &'static str, fingerprint: Fp, expect: Expect) -> Case<codex::CodexProfile> {
    Case {
        table: codex::RECIPES,
        version,
        fingerprint,
        expect,
    }
}

fn newer(recipe: &'static str, major: bool) -> Expect {
    Optimistic {
        recipe,
        placement: Placement::NewerThanVerified,
        major,
    }
}

fn within(recipe: &'static str, major: bool) -> Expect {
    Optimistic {
        recipe,
        placement: Placement::WithinSpan,
        major,
    }
}

#[test]
fn claude_rows() {
    for (version, expect) in [
        ("2.1.283", Listed(CLAUDE)),
        ("2.1.284", Listed(CLAUDE)),
        ("2.1.285", Listed(CLAUDE)),
        ("2.1.286", Listed(CLAUDE)),
        ("2.1.287", Listed(CLAUDE_287)),
        ("2.1.282", Refused(Kind::Older)),
        ("1.0.0", Refused(Kind::Older)),
        // Newer than every recipe: the recipe with the greatest max (the
        // 2.1.287 recipe) is assumed.
        ("2.1.288", newer(CLAUDE_287, false)),
        ("2.1.301", newer(CLAUDE_287, false)),
        ("2.2.0", newer(CLAUDE_287, false)),
        ("3.0.0", newer(CLAUDE_287, true)),
        ("v2.1.286", Refused(Kind::Unparsable)),
        ("2.1", Refused(Kind::Unparsable)),
        ("02.1.286", Refused(Kind::Unparsable)),
        ("", Refused(Kind::Unparsable)),
    ] {
        run(&claude_case(version, expect));
    }
}

#[test]
fn codex_rows() {
    for (version, fingerprint, expect) in [
        ("0.157.1", Fp::None, Listed(CODEX)),
        ("0.158.0", Fp::None, Listed(CODEX)),
        ("0.159.3", Fp::None, Listed(CODEX)),
        // Row 5 beats row 4: older than every min is refused even when the
        // fingerprint matches (design roast r2, ht-p03.62).
        ("0.155.1", Match(CODEX), Refused(Kind::Older)),
        ("0.100.0", Fp::None, Refused(Kind::Older)),
        ("0.159.2", Match(CODEX), SchemaMatched(CODEX)),
        ("0.159.4", Match(CODEX), SchemaMatched(CODEX)),
        ("0.159.4", Differs, newer(CODEX, false)),
        ("0.159.4", Unreadable, newer(CODEX, false)),
        ("0.160.0", Differs, newer(CODEX, false)),
        ("0.158.1", Differs, within(CODEX, false)),
        // Inside the Exact set's span but not listed (design roast r2,
        // ht-p03.65).
        ("0.157.5", Differs, within(CODEX, false)),
        ("0.157.5", Unreadable, within(CODEX, false)),
        ("0.157.5", Match(CODEX), SchemaMatched(CODEX)),
        ("1.0.0", Differs, newer(CODEX, true)),
        ("0.159.0", Differs, within(CODEX, false)),
        ("v0.158.0", Fp::None, Refused(Kind::Unparsable)),
    ] {
        run(&codex_case(version, fingerprint, expect));
    }
}

const PROFILE: ClaudeProfile = ClaudeProfile {
    input_schema: InputSchema::Hooks2_1_283,
    model_receipt: NativeSupport::Unsupported,
    session_start_compact: NativeSupport::Unsupported,
    composer_stash: NativeSupport::Unsupported,
    poke_during_turn: NativeSupport::Unsupported,
};

const fn planted(
    id: &'static str,
    versions: VersionSet,
    levels: &'static [(Version, Evidence)],
    known_broken: &'static [VersionSet],
) -> Recipe<ClaudeProfile> {
    Recipe {
        id,
        versions,
        evidence: &[],
        scope: "planted",
        evidence_levels: levels,
        known_broken,
        profile: PROFILE,
    }
}

const fn closed(min: Version, max: Version) -> VersionSet {
    VersionSet::Interval {
        min: Some(min),
        max: Some(max),
    }
}

/// A listed version that is also known broken.
static LISTED_AND_BROKEN: &[Recipe<ClaudeProfile>] = &[planted(
    "listed-broken",
    VersionSet::Exact(&[v(1, 0, 0), v(1, 0, 1)]),
    &[(v(1, 0, 0), Evidence::Live), (v(1, 0, 1), Evidence::Live)],
    &[closed(v(1, 0, 1), v(1, 0, 1))],
)];

/// A known-broken range open above, the snippet form report.py emits.
static BROKEN_OPEN: &[Recipe<ClaudeProfile>] = &[planted(
    "broken-open",
    closed(v(2, 1, 283), v(2, 1, 286)),
    &[
        (v(2, 1, 283), Evidence::Live),
        (v(2, 1, 284), Evidence::Live),
        (v(2, 1, 285), Evidence::Live),
        (v(2, 1, 286), Evidence::Live),
    ],
    &[VersionSet::Interval {
        min: Some(Version::new(2, 1, 290)),
        max: None,
    }],
)];

/// Two recipes with a gap.
static GAP: &[Recipe<ClaudeProfile>] = &[
    planted("low", closed(v(1, 0, 0), v(1, 2, 0)), &[], &[]),
    planted("high", closed(v(1, 5, 0), v(1, 6, 0)), &[], &[]),
];

/// A gap across majors: the major flag must compare against the ASSUMED
/// recipe's max (1.x), not the table's greatest max (3.x).
static MAJOR_GAP: &[Recipe<ClaudeProfile>] = &[
    planted("one", closed(v(1, 0, 0), v(1, 2, 0)), &[], &[]),
    planted("three", closed(v(3, 0, 0), v(3, 1, 0)), &[], &[]),
];

fn planted_case(
    table: &'static [Recipe<ClaudeProfile>],
    version: &'static str,
    expect: Expect,
) -> Case<ClaudeProfile> {
    Case {
        table,
        version,
        fingerprint: Fp::None,
        expect,
    }
}

#[test]
fn known_broken_wins_over_listed_and_over_newer_than_every_max() {
    run(&planted_case(
        LISTED_AND_BROKEN,
        "1.0.1",
        Refused(Kind::KnownBroken),
    ));
    run(&planted_case(
        LISTED_AND_BROKEN,
        "1.0.0",
        Listed("listed-broken"),
    ));
    run(&planted_case(
        BROKEN_OPEN,
        "2.1.300",
        Refused(Kind::KnownBroken),
    ));
    // Just below the broken range stays optimistic.
    run(&planted_case(
        BROKEN_OPEN,
        "2.1.289",
        newer("broken-open", false),
    ));
    run(&planted_case(
        BROKEN_OPEN,
        "2.1.290",
        Refused(Kind::KnownBroken),
    ));
    // Row 2 beats a fingerprint match too.
    let row = classify(LISTED_AND_BROKEN, "1.0.1", || Some(&LISTED_AND_BROKEN[0]));
    assert!(
        matches!(row, Row::Refused(Refusal::KnownBroken { .. })),
        "{row:?}"
    );
}

#[test]
fn known_broken_refusal_names_the_range_and_the_newest_working_version() {
    let Row::Refused(Refusal::KnownBroken {
        range,
        newest_working,
    }) = classify(LISTED_AND_BROKEN, "1.0.1", || None)
    else {
        panic!("not refused");
    };
    assert_eq!(range, closed(v(1, 0, 1), v(1, 0, 1)));
    assert_eq!(newest_working, Some(v(1, 0, 0)));
    let Row::Refused(Refusal::KnownBroken { newest_working, .. }) =
        classify(BROKEN_OPEN, "2.1.300", || None)
    else {
        panic!("not refused");
    };
    assert_eq!(newest_working, Some(v(2, 1, 286)));
    let message = admission::known_broken_message(
        "claude",
        "2.1.300",
        &VersionSet::Interval {
            min: Some(v(2, 1, 290)),
            max: None,
        },
        newest_working,
    );
    assert!(message.contains("[2.1.290, \u{2026})"), "{message}");
    assert!(message.contains("2.1.286"), "{message}");
}

#[test]
fn gap_versions_are_optimistic_within_span_under_the_recipe_below() {
    run(&planted_case(GAP, "1.3.4", within("low", false)));
    run(&planted_case(GAP, "1.4.9", within("low", false)));
    run(&planted_case(GAP, "1.1.0", Listed("low")));
    run(&planted_case(GAP, "1.5.5", Listed("high")));
    run(&planted_case(GAP, "2.0.0", newer("high", true)));
    run(&planted_case(GAP, "1.6.1", newer("high", false)));
    run(&planted_case(GAP, "0.9.9", Refused(Kind::Older)));
    run(&planted_case(MAJOR_GAP, "2.5.0", within("one", true)));
    run(&planted_case(MAJOR_GAP, "1.9.0", within("one", false)));
    run(&planted_case(MAJOR_GAP, "3.1.1", newer("three", false)));
    run(&planted_case(MAJOR_GAP, "4.0.0", newer("three", true)));
}

// -- exactly one row ------------------------------------------------------

struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    fn below(&mut self, bound: u32) -> u32 {
        (self.next() % u64::from(bound)) as u32
    }
}

/// The ladder row (1-6) the independent predicates select: the first whose
/// predicate holds. Also asserts row 6 is the complement of rows 2-5.
fn predicate_row<P>(
    table: &[Recipe<P>],
    version: Version,
    fingerprint_matches: bool,
) -> (u8, [bool; 6]) {
    let older = table.iter().all(|r| version < r.min_version());
    let predicates = [
        false,
        table
            .iter()
            .any(|r| r.known_broken.iter().any(|range| range.contains(version))),
        table.iter().any(|r| r.versions.contains(version)),
        !older && fingerprint_matches,
        older,
        true,
    ];
    let first = predicates.iter().position(|p| *p).unwrap() as u8 + 1;
    (first, predicates)
}

fn row_number<P>(row: &Row<'_, P>) -> u8 {
    match row {
        Row::Refused(Refusal::Unparsable) => 1,
        Row::Refused(Refusal::KnownBroken { .. }) => 2,
        Row::Listed(_) => 3,
        Row::SchemaMatched(_) => 4,
        Row::Refused(Refusal::OlderThanSupported(_)) => 5,
        Row::Optimistic { .. } => 6,
    }
}

fn exactly_one_row<P: PartialEq + std::fmt::Debug>(
    table: &'static [Recipe<P>],
    seed: u64,
    shape: (u32, u32, u32),
) -> [usize; 7] {
    let mut rng = Rng(seed);
    let mut seen = [0usize; 7];
    let anchors: Vec<Version> = table
        .iter()
        .flat_map(|r| {
            let mut anchors = r.listed_versions();
            anchors.extend([r.min_version(), r.max_version()]);
            anchors.extend(r.known_broken.iter().flat_map(|range| match range {
                VersionSet::Interval { min, max } => {
                    min.iter().chain(max.iter()).copied().collect()
                }
                VersionSet::Exact(versions) => versions.to_vec(),
            }));
            anchors
        })
        .collect();
    for _ in 0..50_000 {
        let mut version = Version::new(
            rng.below(shape.0 + 1),
            rng.below(shape.1 + 1),
            rng.below(shape.2 + 1),
        );
        if rng.below(2) == 0 {
            let anchor = anchors[rng.below(anchors.len() as u32) as usize];
            let bump = |x: u32, rng: &mut Rng| (x + rng.below(5)).saturating_sub(2);
            version = Version::new(
                anchor.major,
                if rng.below(4) == 0 {
                    bump(anchor.minor, &mut rng)
                } else {
                    anchor.minor
                },
                bump(anchor.patch, &mut rng),
            );
        }
        let text = version.to_string();
        for fingerprint_matches in [false, true] {
            let row = classify(table, &text, || fingerprint_matches.then(|| &table[0]));
            let (first, predicates) = predicate_row(table, version, fingerprint_matches);
            let reached = row_number(&row);
            assert_eq!(reached, first, "{text} fp={fingerprint_matches}: {row:?}");
            assert!(predicates[usize::from(reached) - 1], "{text}");
            assert!(
                predicates[..usize::from(reached) - 1].iter().all(|p| !p),
                "{text}: an earlier row also matches"
            );
            seen[usize::from(reached)] += 1;
        }
    }
    seen
}

#[test]
fn every_parsed_version_reaches_exactly_one_row() {
    let claude = exactly_one_row(claude::RECIPES, 0x9e37_79b9_7f4a_7c15, (3, 200, 400));
    let codex = exactly_one_row(codex::RECIPES, 0xd1b5_4a32_d192_ed03, (3, 200, 400));
    let planted = exactly_one_row(LISTED_AND_BROKEN, 0x1234_5678_9abc_def1, (2, 3, 5));
    let open = exactly_one_row(BROKEN_OPEN, 0x0bad_cafe_0bad_cafe, (3, 3, 320));
    let gap = exactly_one_row(GAP, 0xfeed_face_cafe_beef, (3, 8, 12));
    // The property is only meaningful if every row it claims to cover was
    // actually reached by some generated version.
    assert!(
        claude[3] > 0 && claude[5] > 0 && claude[6] > 0,
        "{claude:?}"
    );
    assert!(
        codex[3] > 0 && codex[4] > 0 && codex[5] > 0 && codex[6] > 0,
        "{codex:?}"
    );
    assert!(planted[2] > 0 && planted[3] > 0, "{planted:?}");
    assert!(open[2] > 0 && open[6] > 0, "{open:?}");
    assert!(gap[3] > 0 && gap[5] > 0 && gap[6] > 0, "{gap:?}");
}

#[test]
fn unparsable_text_is_row_one_for_both_tables() {
    for text in [
        "",
        "1",
        "1.2",
        "1.2.3.4",
        "01.2.3",
        "1.2.3-rc1",
        " 2.1.286",
        "2.1.286\n",
        "v2.1.286",
        "2.1.x",
        "1234567.0.0",
        "-1.2.3",
    ] {
        assert!(
            matches!(
                classify(claude::RECIPES, text, || None),
                Row::Refused(Refusal::Unparsable)
            ),
            "{text:?}"
        );
        assert!(
            matches!(
                classify(codex::RECIPES, text, || Some(&codex::RECIPES[0])),
                Row::Refused(Refusal::Unparsable)
            ),
            "{text:?}"
        );
    }
}

/// The fingerprint closure runs only for row 4: never for a listed, known
/// broken, unparsable or older-than-supported version (hashing a 240 MB
/// binary must be skipped for them).
#[test]
fn schema_match_is_evaluated_only_for_row_four_candidates() {
    for (table_text, evaluated) in [
        ("0.157.1", false),
        ("0.158.0", false),
        ("0.155.1", false),
        ("nonsense", false),
        ("0.159.3", false),
        ("0.157.5", true),
        ("0.159.4", true),
    ] {
        let mut called = false;
        classify(codex::RECIPES, table_text, || {
            called = true;
            None
        });
        assert_eq!(called, evaluated, "{table_text}");
    }
    let mut called = false;
    classify(LISTED_AND_BROKEN, "1.0.1", || {
        called = true;
        None
    });
    assert!(!called, "known broken must not fingerprint");
}

// -- the test-only override -----------------------------------------------

fn write_document(name: &str, document: serde_json::Value) -> std::ffi::OsString {
    let path = std::env::temp_dir().join(format!(
        "herdr-threads-admission-{}-{name}.json",
        std::process::id()
    ));
    std::fs::write(&path, serde_json::to_vec_pretty(&document).unwrap()).unwrap();
    path.into_os_string()
}

fn with_path(path: &std::ffi::OsString) -> impl Fn(&str) -> Option<std::ffi::OsString> + '_ {
    move |key| (key == "HT_TEST_RECIPES_JSON").then(|| path.clone())
}

#[test]
fn override_known_broken_refuses() {
    let path = write_document(
        "known-broken",
        serde_json::json!({"schema_version": 1, "rows": [
            {"harness": "claude", "version": "2.1.285", "recipe": CLAUDE, "evidence": "live", "known_broken": []},
            {"harness": "claude", "version": "2.1.286", "recipe": CLAUDE, "evidence": "live",
             "known_broken": [{"min": "2.1.286", "max": "2.1.286"}]},
        ]}),
    );
    let table = override_table("claude", claude::RECIPES, with_path(&path)).expect("override");
    // Inside the broken range, though listed.
    let Row::Refused(Refusal::KnownBroken {
        range,
        newest_working,
    }) = classify(table, "2.1.286", || None)
    else {
        panic!("2.1.286 is listed and known broken");
    };
    assert_eq!(range, closed(v(2, 1, 286), v(2, 1, 286)));
    assert_eq!(newest_working, Some(v(2, 1, 285)));
    assert!(matches!(
        classify(table, "2.1.285", || None),
        Row::Listed(recipe) if recipe.id == CLAUDE
    ));
    // The unplanted harness keeps its compiled table.
    assert!(override_table("codex", codex::RECIPES, with_path(&path)).is_none());
    // Unset: no override.
    assert!(override_table("claude", claude::RECIPES, |_| None).is_none());
    let _ = std::fs::remove_file(&path);
}

#[test]
fn override_builds_a_gap_table_from_json_alone() {
    let path = write_document(
        "gap",
        serde_json::json!({"schema_version": 1, "rows": [
            {"harness": "claude", "version": "1.0.0", "recipe": "one", "evidence": "live", "known_broken": []},
            {"harness": "claude", "version": "1.2.0", "recipe": "one", "evidence": "no_model", "known_broken": []},
            {"harness": "claude", "version": "3.0.0", "recipe": "three", "evidence": "none", "known_broken": []},
            {"harness": "claude", "version": "3.1.0", "recipe": "three", "evidence": "live", "known_broken": []},
        ]}),
    );
    let table = override_table("claude", claude::RECIPES, with_path(&path)).expect("override");
    assert_eq!(table.len(), 2);
    assert_eq!(table[0].evidence_level(v(1, 2, 0)), Some(Evidence::NoModel));
    let Row::Optimistic { recipe, admission } = classify(table, "2.5.0", || None) else {
        panic!("2.5.0 lies in the gap");
    };
    assert_eq!(recipe.id, "one");
    assert_eq!(admission.placement, Placement::WithinSpan);
    assert!(
        admission.major_version_change,
        "2 > the assumed recipe's max major 1"
    );
    assert_eq!(admission.verified_max, v(3, 1, 0));
    let Row::Optimistic { admission, .. } = classify(table, "1.5.0", || None) else {
        panic!();
    };
    assert!(!admission.major_version_change);
    let _ = std::fs::remove_file(&path);
}

/// A release build ignores the override: the code and the variable name are
/// compiled only under `cfg(test)` / `test-support`. This test pins the gate
/// at the source level; the release-binary check is `strings` in the commit.
#[test]
fn override_is_gated_out_of_release_builds() {
    let source = include_str!("../../src/harness/admission.rs");
    let uses = source.matches("HT_TEST_RECIPES_JSON").count();
    assert!(uses > 0);
    // The only reader is behind the cfg gate.
    let gate = "#[cfg(any(test, feature = \"test-support\"))]\npub fn override_table";
    assert!(source.contains(gate), "override_table must be cfg-gated");
    let reads = source.matches("HT_TEST_RECIPES_JSON\")").count();
    assert_eq!(reads, 1, "exactly one reader of the variable");
}

#[test]
fn registry_admission_refuses_mismatched_runtime_identity_before_adapter() {
    use crate::harness::{
        adapter::{AdmissionRequest, InstallObservation, RuntimeIdentity},
        registry::builtins,
    };
    let registry = builtins();
    let registration = registry.by_id(registry.agent("claude").unwrap()).unwrap();
    let mut identity = RuntimeIdentity::stable_release("2.1.287", "installed_probe").unwrap();
    identity.key = "release:2.1.286".into();
    let request = AdmissionRequest {
        installed: InstallObservation::Available {
            binary: "unused".into(),
            identity,
        },
        input: None,
        runtime_candidate: None,
    };
    let budget = crate::protocol::time::CallBudget {
        deadline: crate::protocol::time::MonoInstant(100),
        cancellation: Default::default(),
    };
    assert!(registration.admit(&request, &budget).is_err());
}
