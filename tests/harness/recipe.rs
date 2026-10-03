use crate::harness::recipe::{
    self, LookupError, NativeSupport, Recipe, Version, VersionSet, describe, lookup,
};
use crate::harness::{claude, codex};
use crate::protocol::results::CapabilityState;

const fn v(major: u32, minor: u32, patch: u32) -> Version {
    Version::new(major, minor, patch)
}

/// A synthetic two-recipe table exercising both version-set forms.
const TABLE: &[Recipe<&str>] = &[
    Recipe {
        id: "exact",
        versions: VersionSet::Exact(&[v(1, 0, 3), v(1, 2, 0)]),
        evidence: &[],
        scope: "",
        evidence_levels: &[],
        known_broken: &[],
        profile: "a",
    },
    Recipe {
        id: "interval",
        versions: VersionSet::Interval {
            min: Some(v(2, 0, 5)),
            max: Some(v(2, 1, 0)),
        },
        evidence: &[],
        scope: "",
        evidence_levels: &[],
        known_broken: &[],
        profile: "b",
    },
];

/// Kills: accepting non-canonical version text (prefix/suffix/whitespace,
/// leading zeros, pre-release tags, extra or missing components) as a
/// version, which would let `1.2.03` or `1.2.3-alpha` alias a recipe version.
#[test]
fn version_parse_accepts_only_canonical_triples() {
    assert_eq!(Version::parse("0.157.1"), Some(v(0, 157, 1)));
    assert_eq!(Version::parse("10.0.999999"), Some(v(10, 0, 999_999)));
    for text in [
        "",
        "1",
        "1.2",
        "1.2.3.4",
        "01.2.3",
        "1.02.3",
        "1.2.03",
        "1.2.3-alpha",
        " 1.2.3",
        "1.2.3 ",
        "1.2.3\n",
        "v1.2.3",
        "1.2.+3",
        "1.2.x",
        "1..3",
        "1234567.0.0",
        "1.2.3a",
        "-1.2.3",
    ] {
        assert_eq!(Version::parse(text), None, "{text:?}");
    }
    assert_eq!(v(0, 157, 1).to_string(), "0.157.1");
}

/// Kills: an exclusive interval bound (dropping `min` or `max`), an interval
/// widened past its bounds, component-wise instead of lexicographic ordering,
/// and an exact set matched by prefix or nearest member.
#[test]
fn version_sets_match_exact_members_and_inclusive_interval_bounds() {
    const EXACT: VersionSet = VersionSet::Exact(&[v(1, 0, 3), v(1, 2, 0)]);
    let exact = EXACT;
    assert!(exact.contains(v(1, 0, 3)) && exact.contains(v(1, 2, 0)));
    for miss in [v(1, 0, 4), v(1, 1, 0), v(1, 0, 2), v(1, 2, 1), v(0, 0, 3)] {
        assert!(!exact.contains(miss), "{miss}");
    }
    let interval = VersionSet::Interval {
        min: Some(v(2, 0, 5)),
        max: Some(v(2, 1, 0)),
    };
    for hit in [v(2, 0, 5), v(2, 0, 6), v(2, 0, 999_999), v(2, 1, 0)] {
        assert!(interval.contains(hit), "{hit}");
    }
    for miss in [v(2, 0, 4), v(2, 1, 1), v(1, 9, 9), v(3, 0, 0), v(2, 2, 0)] {
        assert!(!interval.contains(miss), "{miss}");
    }
    assert_eq!(exact.to_string(), "{1.0.3, 1.2.0}");
    assert_eq!(interval.to_string(), "[2.0.5, 2.1.0]");
    // Open bounds (known_broken's encoding): None is unbounded on that side.
    let above = VersionSet::Interval {
        min: Some(v(2, 1, 290)),
        max: None,
    };
    assert!(above.contains(v(2, 1, 290)) && above.contains(v(9, 0, 0)));
    assert!(!above.contains(v(2, 1, 289)));
    let below = VersionSet::Interval {
        min: None,
        max: Some(v(0, 5, 0)),
    };
    assert!(below.contains(v(0, 0, 0)) && below.contains(v(0, 5, 0)));
    assert!(!below.contains(v(0, 5, 1)));
    assert_eq!(above.to_string(), "[2.1.290, \u{2026})");
    assert_eq!(below.to_string(), "(\u{2026}, 0.5.0]");
}

/// Kills: lookup falling back to a default/first recipe for an unknown
/// version, returning the wrong recipe for an interval bound, and treating
/// unrecognized text as a (merely unsupported) version.
#[test]
fn lookup_selects_exact_and_interval_recipes_and_fails_closed() {
    assert_eq!(lookup(TABLE, "1.0.3").unwrap().id, "exact");
    assert_eq!(lookup(TABLE, "1.2.0").unwrap().profile, "a");
    assert_eq!(lookup(TABLE, "2.0.5").unwrap().id, "interval");
    assert_eq!(lookup(TABLE, "2.1.0").unwrap().profile, "b");
    for unknown in ["1.0.4", "1.1.0", "2.0.4", "2.1.1", "9.9.9"] {
        assert_eq!(
            lookup(TABLE, unknown),
            Err(LookupError::Unsupported(Version::parse(unknown).unwrap())),
            "{unknown}"
        );
    }
    assert_eq!(lookup(TABLE, "2.1.0-rc1"), Err(LookupError::Unrecognized));
    assert_eq!(
        lookup::<&str>(&[], "1.0.3"),
        Err(LookupError::Unsupported(v(1, 0, 3)))
    );
    let message = recipe::unsupported_message("tool", "2.1.1", TABLE);
    assert!(
        message.contains("tool 2.1.1 has no adapter recipe"),
        "{message}"
    );
    assert!(
        message.contains("exact {1.0.3, 1.2.0}; interval [2.0.5, 2.1.0]"),
        "{message}"
    );
    assert_eq!(
        describe(TABLE),
        "exact {1.0.3, 1.2.0}; interval [2.0.5, 2.1.0]"
    );
    // Kills: the observation-failure refusal omitting the recipe list.
    let message = recipe::unavailable_message("tool", "timed out", "Do the tool thing", TABLE);
    assert!(
        message.contains("tool --version could not be observed (timed out)"),
        "{message}"
    );
    assert!(
        message.contains("supported recipes: exact {1.0.3, 1.2.0}; interval [2.0.5, 2.1.0]"),
        "{message}"
    );
    // Kills: the helper hard-coding one harness's remedy instead of the
    // caller's (the recipes-fix2 review Nit).
    assert!(message.ends_with(". Do the tool thing"), "{message}");
    assert!(!message.contains("absolute executable"), "{message}");
}

fn members<P>(recipe: &Recipe<P>) -> Vec<Version> {
    match recipe.versions {
        VersionSet::Exact(versions) => versions.to_vec(),
        VersionSet::Interval { min, max } => vec![min.unwrap(), max.unwrap()],
    }
}

fn overlaps<P>(a: &Recipe<P>, b: &Recipe<P>) -> bool {
    match (a.versions, b.versions) {
        (VersionSet::Interval { min: a0, max: a1 }, VersionSet::Interval { min: b0, max: b1 }) => {
            a0.unwrap() <= b1.unwrap() && b0.unwrap() <= a1.unwrap()
        }
        _ => {
            members(a).iter().any(|x| b.versions.contains(*x))
                || members(b).iter().any(|x| a.versions.contains(*x))
        }
    }
}

fn check_table<P>(table: &[Recipe<P>]) {
    assert!(!table.is_empty());
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    for (index, recipe) in table.iter().enumerate() {
        match recipe.versions {
            VersionSet::Exact(versions) => assert!(!versions.is_empty(), "{}", recipe.id),
            VersionSet::Interval { min, max } => {
                assert!(min.unwrap() <= max.unwrap(), "{}", recipe.id);
            }
        }
        assert!(!recipe.evidence.is_empty(), "{} has no evidence", recipe.id);
        for path in recipe.evidence {
            assert!(
                root.join(path).is_file(),
                "{}: missing evidence {path}",
                recipe.id
            );
        }
        assert!(!recipe.scope.is_empty(), "{}", recipe.id);
        for other in &table[index + 1..] {
            assert_ne!(recipe.id, other.id);
            assert!(
                !overlaps(recipe, other),
                "{} overlaps {}",
                recipe.id,
                other.id
            );
        }
    }
}

/// Kills: a production recipe without committed evidence (or pointing at a
/// moved/deleted report), an empty version set, and two recipes claiming
/// the same installed version.
#[test]
fn production_registries_are_evidence_backed_and_disjoint() {
    check_table(codex::RECIPES);
    check_table(claude::RECIPES);
    assert!(
        codex::RECIPES[0]
            .evidence
            .iter()
            .any(|path| path.ends_with("codex-158-hook-capture/report.md"))
    );
    assert!(
        claude::RECIPES[0]
            .evidence
            .iter()
            .any(|path| path.ends_with("claude-284-hook-capture/report.md"))
    );
    assert!(
        claude::RECIPES[0]
            .evidence
            .iter()
            .any(|path| path.ends_with("claude-285-hook-capture/report.md"))
    );
    assert!(
        claude::RECIPES[0]
            .evidence
            .iter()
            .any(|path| path.ends_with("claude-286-hook-capture/report.md"))
    );
}

/// Kills: dropping 0.157.1 or 0.158.0 from the Codex recipe; widening it to an
/// interval or to an uncaptured neighbour (0.155.1 shares the schemas but has
/// no parse evidence here; 0.157.0/0.157.2/0.158.1/0.159.0 have none); and a
/// lookup that accepts an unknown version.
#[test]
fn codex_registry_admits_exactly_the_captured_versions() {
    assert_eq!(codex::RECIPES.len(), 1);
    let recipe = &codex::RECIPES[0];
    assert_eq!(recipe.id, "codex-hooks-v1");
    const CAPTURED: VersionSet = VersionSet::Exact(&[v(0, 157, 1), v(0, 158, 0), v(0, 159, 3)]);
    assert_eq!(recipe.versions, CAPTURED);
    assert_eq!(recipe.profile.input_schema, codex::InputSchema::HooksV1);
    for version in ["0.157.1", "0.158.0", "0.159.3"] {
        assert_eq!(codex::recipe_for(version), Ok(recipe), "{version}");
    }
    for version in [
        "0.155.1", "0.157.0", "0.157.2", "0.158.1", "0.159.0", "1.157.1",
    ] {
        assert_eq!(
            codex::recipe_for(version),
            Err(LookupError::Unsupported(Version::parse(version).unwrap())),
            "{version}"
        );
    }
    assert_eq!(
        codex::recipe_for("0.157.01"),
        Err(LookupError::Unrecognized)
    );
    assert_eq!(codex::DECLARATION.recipes, codex::RECIPES);
}

/// Kills: a recipe flipping native transport or receipt to supported without
/// evidence, and health reporting either harness supported while any recipe
/// lacks transport/receipt.
#[test]
fn every_recipe_keeps_transport_and_receipt_unsupported_and_health_follows() {
    for recipe in codex::RECIPES {
        assert_eq!(
            recipe.profile.invocation_transport,
            NativeSupport::Unsupported
        );
        assert_eq!(recipe.profile.model_receipt, NativeSupport::Unsupported);
    }
    for recipe in claude::RECIPES {
        assert_eq!(recipe.profile.model_receipt, NativeSupport::Unsupported);
    }
    assert_eq!(
        codex::DECLARATION.health_capability(),
        CapabilityState::Unsupported
    );
    assert_eq!(claude::health_capability(), CapabilityState::Unsupported);
}

/// Kills: a capability declared for a version the spike did not test (a
/// neighbouring Claude patch, any Codex version), an interval widening that
/// inherits 2.1.287's declaration, a declaration that drops `poke_during_turn`
/// or `composer_stash` for 2.1.287, and an unobserved or malformed version
/// reporting anything but NONE.
#[test]
fn poke_capabilities_follow_the_recipe() {
    use crate::harness::recipe::{PokeCapabilities, poke_capabilities};
    use crate::protocol::authority::Harness;
    let declared = PokeCapabilities {
        composer_stash: NativeSupport::Supported,
        poke_during_turn: NativeSupport::Supported,
    };
    assert_eq!(
        poke_capabilities(Harness::Claude, Some("2.1.287")),
        declared
    );
    // Every other recipe version, and every Claude version outside them.
    for version in [
        "2.1.283", "2.1.284", "2.1.285", "2.1.286", "2.1.288", "2.2.0",
    ] {
        assert_eq!(
            poke_capabilities(Harness::Claude, Some(version)),
            PokeCapabilities::NONE,
            "claude {version}"
        );
    }
    // Codex: the spike tested 0.160.0, which no recipe covers, so nothing is
    // declared for any version.
    for version in ["0.157.1", "0.158.0", "0.160.0"] {
        assert_eq!(
            poke_capabilities(Harness::Codex, Some(version)),
            PokeCapabilities::NONE,
            "codex {version}"
        );
    }
    for harness in [Harness::Claude, Harness::Codex, Harness::Human] {
        for version in [
            None,
            Some(""),
            Some("2.1.287 "),
            Some("v2.1.287"),
            Some("2.1"),
        ] {
            assert_eq!(
                poke_capabilities(harness, version),
                PokeCapabilities::NONE,
                "{harness:?} {version:?}"
            );
        }
    }
    assert_eq!(
        poke_capabilities(Harness::Human, Some("2.1.287")),
        PokeCapabilities::NONE
    );
}

/// Kills: a recipe declaring a poke capability without citing the spike
/// findings in its evidence, a declaration on a recipe covering a version the
/// spike did not test, and any Codex declaration (no recipe covers 0.160.0).
#[test]
fn recipes_declare_poke_capabilities_only_with_spike_evidence() {
    for recipe in claude::RECIPES {
        let declares = recipe.profile.composer_stash == NativeSupport::Supported
            || recipe.profile.poke_during_turn == NativeSupport::Supported;
        let cites = recipe
            .evidence
            .contains(&"docs/evidence/poke-spike/findings.md");
        assert_eq!(declares, cites, "{}", recipe.id);
        if declares {
            const ONLY: VersionSet = VersionSet::Exact(&[v(2, 1, 287)]);
            assert_eq!(recipe.versions, ONLY);
        }
    }
    for recipe in codex::RECIPES {
        assert_eq!(recipe.profile.composer_stash, NativeSupport::Unsupported);
        assert_eq!(recipe.profile.poke_during_turn, NativeSupport::Unsupported);
    }
}
