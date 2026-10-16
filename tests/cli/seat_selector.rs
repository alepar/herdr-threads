//! Unit tests for the pure seat-selector resolver over hand-built topologies.
//! Mounted from `src/cli/seat_selector.rs`.

use super::*;
use crate::protocol::results::ErrorCode;

fn space(id: &str, label: &str) -> TopologySpace {
    TopologySpace {
        id: id.into(),
        label: Some(label.into()),
    }
}

fn tab(id: &str, space: &str, label: &str) -> TopologyTab {
    TopologyTab {
        id: id.into(),
        space: space.into(),
        label: Some(label.into()),
    }
}

fn pane(id: &str, space: &str, tab: &str, label: &str, agents: &[&str]) -> TopologyPane {
    TopologyPane {
        target: HostTargetId::new(id),
        space: space.into(),
        tab: tab.into(),
        label: (!label.is_empty()).then(|| label.to_owned()),
        agent_names: agents.iter().map(|a| (*a).to_owned()).collect(),
    }
}

fn base() -> HostTopology {
    HostTopology {
        spaces: vec![
            space("w1", "project"),
            space("w2", "solo"),
            space("w3", "other"),
        ],
        tabs: vec![
            tab("w1:t1", "w1", "main"),
            tab("w1:t2", "w1", "tryout"),
            tab("w2:t1", "w2", "only"),
            tab("w3:t1", "w3", "tryout"),
        ],
        panes: vec![
            pane("w1:p1", "w1", "w1:t1", "alice", &[]),
            pane("w1:p4", "w1", "w1:t1", "carol", &[]),
            pane("w1:p2", "w1", "w1:t2", "alice", &[]),
            pane("w2:p1", "w2", "w2:t1", "dave", &["eve"]),
            pane("w3:p1", "w3", "w3:t1", "alice", &[]),
        ],
    }
}

fn id(s: &str) -> HostTargetId {
    HostTargetId::new(s)
}

fn none() -> Narrowing {
    Narrowing::default()
}

fn resolved(outcome: Outcome) -> Candidate {
    match outcome {
        Outcome::Resolved(c) => c,
        other => panic!("expected Resolved, got {other:?}"),
    }
}

fn ambiguous(outcome: Outcome) -> Vec<Candidate> {
    match outcome {
        Outcome::Ambiguous(c) => c,
        other => panic!("expected Ambiguous, got {other:?}"),
    }
}

fn panes_of(candidates: &[Candidate]) -> Vec<&str> {
    candidates.iter().map(|c| c.pane.as_str()).collect()
}

fn reading(space: Option<&str>, tab: Option<&str>, pane: Option<&str>) -> Reading {
    Reading {
        space: space.map(str::to_owned),
        tab: tab.map(str::to_owned),
        pane: pane.map(str::to_owned),
    }
}

#[test]
fn one_segment_matches_pane_label_agent_name_and_pane_id() {
    let t = base();
    let c = resolved(resolve("carol", &t, None, &none()));
    assert_eq!(c.pane, id("w1:p4"));
    assert_eq!(c.readings, vec![reading(None, None, Some("carol"))]);
    // live agent name
    assert_eq!(
        resolved(resolve("eve", &t, None, &none())).pane,
        id("w2:p1")
    );
    // pane ID
    assert_eq!(
        resolved(resolve("w1:p4", &t, None, &none())).pane,
        id("w1:p4")
    );
}

#[test]
fn one_segment_tab_alias_needs_exactly_one_pane() {
    let t = base();
    // `tryout` names tabs w1:t2 (one pane) and w3:t1 (one pane): both aliases match.
    let c = ambiguous(resolve("tryout", &t, None, &none()));
    assert_eq!(panes_of(&c), vec!["w1:p2", "w3:p1"]);
    assert_eq!(c[0].readings, vec![reading(None, Some("tryout"), None)]);
    // `main` has two panes: no alias, so no match at all.
    assert_eq!(resolve("main", &t, None, &none()), Outcome::NoMatch);
    // tab ID alias works too, for the sole-pane tab only.
    assert_eq!(
        resolved(resolve("w1:t2", &t, None, &none())).pane,
        id("w1:p2")
    );
    assert_eq!(resolve("w1:t1", &t, None, &none()), Outcome::NoMatch);
}

#[test]
fn one_segment_space_alias_needs_one_tab_with_one_pane() {
    let t = base();
    let c = resolved(resolve("solo", &t, None, &none()));
    assert_eq!(c.pane, id("w2:p1"));
    assert_eq!(c.readings, vec![reading(Some("solo"), None, None)]);
    // `project` has several tabs: no space alias.
    assert_eq!(resolve("project", &t, None, &none()), Outcome::NoMatch);
    // `other` has one tab with one pane: alias by label; by ID too.
    assert_eq!(
        resolved(resolve("other", &t, None, &none())).pane,
        id("w3:p1")
    );
    assert_eq!(resolved(resolve("w3", &t, None, &none())).pane, id("w3:p1"));
}

#[test]
fn two_segments_pool_space_tab_tab_pane_and_space_pane() {
    let t = base();
    // space/tab (sole pane tab)
    let c = resolved(resolve("project/tryout", &t, None, &none()));
    assert_eq!(c.pane, id("w1:p2"));
    assert_eq!(
        c.readings,
        vec![reading(Some("project"), Some("tryout"), None)]
    );
    // tab/pane
    let c = resolved(resolve("main/carol", &t, None, &none()));
    assert_eq!(c.pane, id("w1:p4"));
    assert_eq!(c.readings, vec![reading(None, Some("main"), Some("carol"))]);
    // space/pane (any tab in the space)
    let c = resolved(resolve("project/carol", &t, None, &none()));
    assert_eq!(c.pane, id("w1:p4"));
    assert_eq!(
        c.readings,
        vec![reading(Some("project"), None, Some("carol"))]
    );
    // space/tab does not match a tab with two panes
    assert_eq!(resolve("project/main", &t, None, &none()), Outcome::NoMatch);
    // the tab/pane reading alone matches both `tryout` tabs
    let c = ambiguous(resolve("tryout/alice", &t, None, &none()));
    assert_eq!(panes_of(&c), vec!["w1:p2", "w3:p1"]);
    let mut both = base();
    both.spaces.push(space("tryout", "x"));
    both.tabs.push(tab("tryout:t9", "tryout", "tryout"));
    both.panes
        .push(pane("tryout:p9", "tryout", "tryout:t9", "alice", &[]));
    let c = ambiguous(resolve("tryout/alice", &both, None, &none()));
    let last = c.last().unwrap();
    assert_eq!(last.pane, id("tryout:p9"));
    assert_eq!(
        last.readings,
        vec![
            reading(None, Some("tryout"), Some("alice")),
            reading(Some("tryout"), None, Some("alice")),
        ]
    );
}

#[test]
fn three_segments_nest_space_tab_pane() {
    let t = base();
    let c = resolved(resolve("project/main/alice", &t, None, &none()));
    assert_eq!(c.pane, id("w1:p1"));
    assert_eq!(
        c.readings,
        vec![reading(Some("project"), Some("main"), Some("alice"))]
    );
    // levels must nest: `other` space does not contain tab `main`
    assert_eq!(
        resolve("other/main/alice", &t, None, &none()),
        Outcome::NoMatch
    );
}

#[test]
fn whole_string_label_containing_slash_matches() {
    let mut t = base();
    t.panes.push(pane("w1:p9", "w1", "w1:t1", "a/b", &[]));
    let c = resolved(resolve("a/b", &t, None, &none()));
    assert_eq!(c.pane, id("w1:p9"));
    assert_eq!(c.readings, vec![reading(None, None, Some("a/b"))]);
}

#[test]
fn id_segments_match_at_every_level() {
    let t = base();
    assert_eq!(
        resolved(resolve("w1/w1:t2/w1:p2", &t, None, &none())).pane,
        id("w1:p2")
    );
    assert_eq!(
        resolved(resolve("w1:t2/alice", &t, None, &none())).pane,
        id("w1:p2")
    );
    assert_eq!(
        resolved(resolve("w1/w1:t1/carol", &t, None, &none())).pane,
        id("w1:p4")
    );
    assert_eq!(
        resolved(resolve("project/w1:t1/w1:p4", &t, None, &none())).pane,
        id("w1:p4")
    );
}

#[test]
fn four_or_more_segments_have_only_the_whole_string_reading() {
    let mut t = base();
    assert_eq!(
        resolve("project/main/alice/x", &t, None, &none()),
        Outcome::NoMatch
    );
    t.panes
        .push(pane("w1:p9", "w1", "w1:t1", "p/main/alice/x", &[]));
    let c = resolved(resolve("p/main/alice/x", &t, None, &none()));
    assert_eq!(c.pane, id("w1:p9"));
    assert_eq!(
        c.readings,
        vec![reading(None, None, Some("p/main/alice/x"))]
    );
}

#[test]
fn empty_segment_disables_split_readings() {
    let mut t = base();
    assert_eq!(
        resolve("project//alice", &t, None, &none()),
        Outcome::NoMatch
    );
    assert_eq!(resolve("/alice", &t, None, &none()), Outcome::NoMatch);
    assert_eq!(resolve("alice/", &t, None, &none()), Outcome::NoMatch);
    assert_eq!(resolve("", &t, None, &none()), Outcome::NoMatch);
    t.panes.push(pane("w1:p9", "w1", "w1:t1", "x//y", &[]));
    assert_eq!(
        resolved(resolve("x//y", &t, None, &none())).pane,
        id("w1:p9")
    );
}

#[test]
fn tier_precedence_tab_beats_space_beats_any() {
    let t = base();
    // caller in w1:t1 -> w1:p1 (same tab) wins over w1:p2 (same space) and w3:p1.
    let c = resolved(resolve("alice", &t, Some(&id("w1:p4")), &none()));
    assert_eq!((c.pane, c.tier), (id("w1:p1"), Tier::Tab));
    // caller in w1:t2 -> w1:p2 is tab tier (the caller's own pane is a candidate too).
    let c = resolved(resolve("alice", &t, Some(&id("w1:p2")), &none()));
    assert_eq!((c.pane, c.tier), (id("w1:p2"), Tier::Tab));
    // caller in w1 on a tab with no alice: space tier beats any.
    let mut t2 = base();
    t2.tabs.push(tab("w1:t3", "w1", "scratch"));
    t2.panes.push(pane("w1:p7", "w1", "w1:t3", "zed", &[]));
    t2.panes.retain(|p| p.target != id("w1:p2"));
    let c = resolved(resolve("alice", &t2, Some(&id("w1:p7")), &none()));
    assert_eq!((c.pane, c.tier), (id("w1:p1"), Tier::Space));
}

#[test]
fn space_tier_wins_when_no_tab_tier_candidate() {
    let mut t = base();
    t.tabs.push(tab("w1:t3", "w1", "scratch"));
    t.panes.push(pane("w1:p7", "w1", "w1:t3", "zed", &[]));
    // alice: w1:p1 (w1:t1), w1:p2 (w1:t2), w3:p1; caller in w1:t3 -> space tier = p1 and p2.
    let c = ambiguous(resolve("alice", &t, Some(&id("w1:p7")), &none()));
    assert_eq!(panes_of(&c), vec!["w1:p1", "w1:p2"]);
    assert!(c.iter().all(|c| c.tier == Tier::Space));
    // caller in w2: only the any tier exists.
    let c = ambiguous(resolve("alice", &t, Some(&id("w2:p1")), &none()));
    assert_eq!(c.len(), 3);
    assert!(c.iter().all(|c| c.tier == Tier::Any));
}

#[test]
fn no_caller_or_stale_caller_makes_every_candidate_any() {
    let t = base();
    let c = ambiguous(resolve("alice", &t, None, &none()));
    assert_eq!(panes_of(&c), vec!["w1:p1", "w1:p2", "w3:p1"]);
    assert!(c.iter().all(|c| c.tier == Tier::Any));
    let stale = id("w9:p9");
    let c = ambiguous(resolve("alice", &t, Some(&stale), &none()));
    assert_eq!(c.len(), 3);
    assert!(c.iter().all(|c| c.tier == Tier::Any));
}

#[test]
fn narrowing_drops_candidates_outside_space_and_tab() {
    let t = base();
    let space_only = Narrowing {
        space: Some("w1".into()),
        tab: None,
    };
    let c = ambiguous(resolve("alice", &t, None, &space_only));
    assert_eq!(panes_of(&c), vec!["w1:p1", "w1:p2"]);
    let both = Narrowing {
        space: Some("w1".into()),
        tab: Some("w1:t2".into()),
    };
    assert_eq!(
        resolved(resolve("alice", &t, None, &both)).pane,
        id("w1:p2")
    );
    let outside = Narrowing {
        space: Some("w2".into()),
        tab: None,
    };
    assert_eq!(resolve("alice", &t, None, &outside), Outcome::NoMatch);
    // narrowing applies before tiering: a caller-tier match outside the space is dropped.
    let c = resolved(resolve(
        "alice",
        &t,
        Some(&id("w1:p4")),
        &Narrowing {
            space: Some("w3".into()),
            tab: None,
        },
    ));
    assert_eq!((c.pane, c.tier), (id("w3:p1"), Tier::Any));
}

#[test]
fn rendered_path_is_full_live_path_for_every_alias() {
    let t = base();
    // pane reading
    assert_eq!(
        resolved(resolve("carol", &t, None, &none())).path,
        "project/main/carol"
    );
    // tab alias: `tryout` narrowed to w1 prints the full path, not just the tab
    let c = resolved(resolve(
        "tryout",
        &t,
        None,
        &Narrowing {
            space: Some("w1".into()),
            tab: None,
        },
    ));
    assert_eq!(c.path, "project/tryout/alice");
    // space alias
    assert_eq!(
        resolved(resolve("solo", &t, None, &none())).path,
        "solo/only/dave"
    );
    // space/tab (sole pane)
    assert_eq!(
        resolved(resolve("project/tryout", &t, None, &none())).path,
        "project/tryout/alice"
    );
    assert_eq!(render_path(&t, &id("w9:p9")), None);
}

fn crowded(n: usize) -> HostTopology {
    let mut t = HostTopology {
        spaces: vec![space("w1", "s")],
        tabs: vec![tab("w1:t1", "w1", "t")],
        panes: Vec::new(),
    };
    for i in 0..n {
        t.panes
            .push(pane(&format!("w1:p{}", i + 1), "w1", "w1:t1", "dup", &[]));
    }
    t
}

#[test]
fn conflict_lists_every_candidate_up_to_cap_32() {
    let t = crowded(32);
    let mut calls = 0;
    let err = resolve("dup", &t, None, &none())
        .into_result("dup", |_| -> Result<Option<SeatId>, ()> {
            calls += 1;
            Ok(Some(SeatId::new(format!("sSeat{calls}"))))
        })
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::Conflict);
    assert_eq!(calls, 32);
    let lines: Vec<&str> = err.detail.lines().collect();
    assert_eq!(lines[0], "ambiguous seat dup: 32 candidates (tier: any)");
    assert_eq!(lines.len(), 1 + 32 + 1); // header + rows + hint, no "omitted"
    assert_eq!(lines[1], "  pane=dup s/t/w1:p1 w1:p1 seat=sSeat1");
    assert!(!err.detail.contains("omitted"));
    assert_eq!(
        lines[33],
        "qualify with a longer SPACE/TAB/PANE path, --space/--tab, or an exact seat or pane ID"
    );
    assert_eq!(err.detail.matches("qualify with").count(), 1);
}

#[test]
fn conflict_at_33_renders_32_and_one_omitted() {
    let t = crowded(33);
    let mut calls = 0;
    let err = resolve("dup", &t, None, &none())
        .into_result("dup", |_| -> Result<Option<SeatId>, ()> {
            calls += 1;
            Ok(None)
        })
        .unwrap_err();
    assert_eq!(calls, 32);
    let lines: Vec<&str> = err.detail.lines().collect();
    assert_eq!(lines[0], "ambiguous seat dup: 33 candidates (tier: any)");
    assert_eq!(lines.len(), 1 + 32 + 1 + 1);
    assert!(lines[1].ends_with("seat=none"));
    assert_eq!(lines[33], "  (1 omitted)");
    assert!(!err.detail.contains("w1:p33"));
    assert!(err.detail.contains("w1:p32"));
    assert!(lines[34].starts_with("qualify with"));
    // size bound: about 6.5 KB worst case
    assert!(err.detail.len() < 6_500, "{}", err.detail.len());
}

#[test]
fn conflict_renders_seat_unknown_on_lookup_failure_and_multiple_readings() {
    let t = base();
    let err = resolve("alice", &t, None, &none())
        .into_result("alice", |_| -> Result<Option<SeatId>, &str> { Err("boom") })
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::Conflict);
    assert_eq!(err.detail.matches("seat=?").count(), 3);
    assert!(
        err.detail
            .contains("  pane=alice project/main/alice w1:p1 seat=?")
    );
    // a pane matched by two readings joins them with `; `
    let mut t = base();
    t.spaces.push(space("tryout", "x"));
    t.tabs.push(tab("tryout:t9", "tryout", "tryout"));
    t.panes
        .push(pane("tryout:p9", "tryout", "tryout:t9", "alice", &[]));
    let err = resolve("tryout/alice", &t, None, &none())
        .into_result("tryout/alice", |_| -> Result<Option<SeatId>, ()> {
            Ok(None)
        })
        .unwrap_err();
    assert!(
        err.detail
            .contains("tab=tryout pane=alice; space=tryout pane=alice"),
        "{}",
        err.detail
    );
}

#[test]
fn no_match_is_not_found_naming_the_selector() {
    let t = base();
    let err = resolve("ghost", &t, None, &none())
        .into_result("ghost", |_| -> Result<Option<SeatId>, ()> { Ok(None) })
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::NotFound);
    assert_eq!(
        err.detail,
        "seat ghost: neither an existing seat ID nor a live space/tab/pane name"
    );
    // hostile selector is escaped and bounded
    let hostile = format!("bad\nline{}", "x".repeat(200));
    let err = resolve(&hostile, &t, None, &none())
        .into_result(&hostile, |_| -> Result<Option<SeatId>, ()> { Ok(None) })
        .unwrap_err();
    assert!(!err.detail.contains('\n'));
    assert!(err.detail.len() < 64 + 80, "{}", err.detail.len());
}

#[test]
fn resolved_converts_to_ok() {
    let t = base();
    let c = resolve("carol", &t, None, &none())
        .into_result("carol", |_| -> Result<Option<SeatId>, ()> {
            panic!("seat lookup must not run for a resolved outcome")
        })
        .unwrap();
    assert_eq!(c.pane, id("w1:p4"));
}

#[test]
fn nickname_match_mismatch_and_unverifiable() {
    let t = base();
    assert_eq!(
        split_nickname("project/main/sAb12Cd34"),
        Some(("project/main", "sAb12Cd34"))
    );
    assert_eq!(split_nickname("alice/sX"), Some(("alice", "sX")));
    assert_eq!(split_nickname("sX"), None);
    assert_eq!(split_nickname("/sX"), None);
    assert_eq!(split_nickname("alice/"), None);
    // Match: SPACE/PANE prefix pools tiers off
    assert_eq!(
        verify_nickname("project/carol", &t, Some(&id("w1:p4"))),
        LocationCheck::Match
    );
    // Match even though the name is ambiguous across tiers
    assert_eq!(
        verify_nickname("alice", &t, Some(&id("w3:p1"))),
        LocationCheck::Match
    );
    // Mismatch names the live path
    let check = verify_nickname("project/carol", &t, Some(&id("w3:p1")));
    assert_eq!(
        check,
        LocationCheck::Mismatch {
            live_path: "other/tryout/alice".into()
        }
    );
    let err = check
        .into_result("project/carol/sSeat1", &SeatId::new("sSeat1"))
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::Conflict);
    assert_eq!(
        err.detail,
        "seat sSeat1 is at other/tryout/alice, not project/carol/sSeat1; pass the bare seat ID sSeat1"
    );
    // Unverifiable: no target, or a target absent from the topology
    assert_eq!(
        verify_nickname("alice", &t, None),
        LocationCheck::Unverifiable
    );
    assert_eq!(
        verify_nickname("alice", &t, Some(&id("w9:p9"))),
        LocationCheck::Unverifiable
    );
    let err = LocationCheck::Unverifiable
        .into_result("x/sSeat1", &SeatId::new("sSeat1"))
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::Conflict);
    assert_eq!(
        err.detail,
        "cannot verify location of sSeat1; pass the bare seat ID"
    );
    assert!(
        LocationCheck::Match
            .into_result("x", &SeatId::new("s"))
            .is_ok()
    );
}

#[test]
fn location_check_with_narrowing() {
    let t = base();
    let w1 = Narrowing {
        space: Some("w1".into()),
        tab: None,
    };
    let w1t2 = Narrowing {
        space: Some("w1".into()),
        tab: Some("w1:t2".into()),
    };
    assert_eq!(
        verify_location(&t, &w1, Some(&id("w1:p4"))),
        LocationCheck::Match
    );
    assert_eq!(
        verify_location(&t, &w1t2, Some(&id("w1:p2"))),
        LocationCheck::Match
    );
    assert_eq!(
        verify_location(&t, &w1t2, Some(&id("w1:p4"))),
        LocationCheck::Mismatch {
            live_path: "project/main/carol".into()
        }
    );
    assert_eq!(
        verify_location(&t, &w1, Some(&id("w3:p1"))),
        LocationCheck::Mismatch {
            live_path: "other/tryout/alice".into()
        }
    );
    assert_eq!(verify_location(&t, &w1, None), LocationCheck::Unverifiable);
    assert_eq!(
        verify_location(&t, &w1, Some(&id("w9:p9"))),
        LocationCheck::Unverifiable
    );
}

fn one_pane(
    space_label: Option<&str>,
    tab_label: Option<&str>,
    label: Option<&str>,
) -> HostTopology {
    HostTopology {
        spaces: vec![TopologySpace {
            id: "w1".into(),
            label: space_label.map(str::to_owned),
        }],
        tabs: vec![TopologyTab {
            id: "w1:t1".into(),
            space: "w1".into(),
            label: tab_label.map(str::to_owned),
        }],
        panes: vec![TopologyPane {
            target: id("w1:p1"),
            space: "w1".into(),
            tab: "w1:t1".into(),
            label: label.map(str::to_owned),
            agent_names: vec!["me".into()],
        }],
    }
}

fn pane_part_of(label: &str) -> String {
    let path = render_path(&one_pane(Some("s"), Some("t"), Some(label)), &id("w1:p1")).unwrap();
    path.rsplit('/').next().unwrap().to_owned()
}

#[test]
fn renderer_uses_ids_for_unsafe_labels() {
    // safe label renders as itself, even equal to the pane's OWN agent name
    assert_eq!(pane_part_of("fine"), "fine");
    assert_eq!(pane_part_of("me"), "me");
    assert_eq!(pane_part_of(&"a".repeat(64)), "a".repeat(64));
    // each rule falls back to the ID
    assert_eq!(pane_part_of(&"a".repeat(65)), "w1:p1");
    assert_eq!(pane_part_of(""), "w1:p1");
    assert_eq!(pane_part_of("a b"), "w1:p1");
    assert_eq!(pane_part_of("a\tb"), "w1:p1");
    assert_eq!(pane_part_of("a\u{1b}b"), "w1:p1");
    assert_eq!(pane_part_of("a\u{200b}b"), "w1:p1");
    assert_eq!(pane_part_of("a/b"), "w1:p1");
    assert_eq!(pane_part_of("w4:p7"), "w1:p1");
    assert_eq!(pane_part_of("w4:t7"), "w1:p1");
    // no label at all
    let t = one_pane(Some("s"), Some("t"), None);
    assert_eq!(render_path(&t, &id("w1:p1")).unwrap(), "s/t/w1:p1");
    let t = one_pane(None, None, Some("x"));
    assert_eq!(render_path(&t, &id("w1:p1")).unwrap(), "w1/w1:t1/x");
    // unsafe tab and space labels
    let t = one_pane(Some("a b"), Some("t/u"), Some("x"));
    assert_eq!(render_path(&t, &id("w1:p1")).unwrap(), "w1/w1:t1/x");
    let t = one_pane(Some("w4:t7"), Some("w4:p7"), Some("x"));
    assert_eq!(render_path(&t, &id("w1:p1")).unwrap(), "w1/w1:t1/x");
}

#[test]
fn renderer_uses_ids_for_duplicate_and_colliding_labels() {
    let mut t = HostTopology {
        spaces: vec![space("w1", "dup"), space("w2", "dup"), space("w3", "w4")],
        tabs: vec![
            tab("w1:t1", "w1", "tt"),
            tab("w1:t2", "w1", "tt"),
            tab("w1:t3", "w1", "unique"),
            tab("w2:t1", "w2", "tt"),
            tab("w4:t1", "w4", "k"),
        ],
        panes: vec![
            pane("w1:p1", "w1", "w1:t1", "same", &[]),
            pane("w1:p2", "w1", "w1:t1", "same", &[]),
            pane("w1:p3", "w1", "w1:t1", "owner", &["agentx"]),
            pane("w1:p4", "w1", "w1:t1", "agentx", &[]),
            pane("w1:p5", "w1", "w1:t1", "w1:p3x", &[]),
            pane("w1:p6", "w1", "w1:t3", "agentx", &[]),
            pane("w2:p1", "w2", "w2:t1", "z", &[]),
            pane("w4:p1", "w4", "w4:t1", "n", &[]),
        ],
    };
    // duplicate space labels fall back to IDs; duplicate tab labels in one space do too, but the
    // same tab label in another space is not a sibling.
    assert_eq!(render_path(&t, &id("w2:p1")).unwrap(), "w2/tt/z");
    assert_eq!(render_path(&t, &id("w1:p6")).unwrap(), "w1/unique/agentx");
    assert_eq!(render_path(&t, &id("w1:p1")).unwrap(), "w1/w1:t1/w1:p1");
    // pane label equal to ANOTHER same-tab pane's agent name -> ID; the owner keeps its label
    assert_eq!(render_path(&t, &id("w1:p4")).unwrap(), "w1/w1:t1/w1:p4");
    assert_eq!(render_path(&t, &id("w1:p3")).unwrap(), "w1/w1:t1/owner");
    // the same label in another tab is not a collision
    assert_eq!(
        render_path(&t, &id("w1:p6")).unwrap().rsplit('/').next(),
        Some("agentx")
    );
    // space label equal to a sibling's ID -> the sibling space w4 keeps its ID, w3's label "w4" is unsafe
    t.spaces.push(space("w4", "four"));
    assert_eq!(render_path(&t, &id("w4:p1")).unwrap(), "four/k/n");
    // pane label equal to a same-tab pane's ID
    t.panes.push(pane("w4:p2", "w4", "w4:t1", "w4:p1x", &[]));
    t.panes.push(pane("w4:p3", "w4", "w4:t1", "ab", &[]));
    t.panes.push(pane("w4:p4", "w4", "w4:t1", "w4:p3", &[]));
    assert_eq!(render_path(&t, &id("w4:p4")).unwrap(), "four/k/w4:p4");
}

#[test]
fn renderer_sibling_id_collision_at_tab_and_space_level() {
    // a tab labelled with a sibling tab's ID-like string; ID shapes are caught by the shape rule,
    // so use opaque IDs that are not ID-shaped.
    let t = HostTopology {
        spaces: vec![space("spA", "spB"), space("spB", "other")],
        tabs: vec![tab("tabX", "spA", "tabY"), tab("tabY", "spA", "tabZ")],
        panes: vec![
            pane("pnA", "spA", "tabX", "p", &[]),
            pane("pnB", "spA", "tabY", "q", &[]),
            pane("pnC", "spB", "spB:none", "r", &[]),
        ],
    };
    // space spA is labelled "spB", the ID of its sibling -> falls back to its ID
    assert_eq!(render_path(&t, &id("pnA")).unwrap(), "spA/tabX/p");
    // tab tabX labelled "tabY", the ID of a sibling tab -> ID
    assert_eq!(render_path(&t, &id("pnB")).unwrap(), "spA/tabZ/q");
    // tab missing from the topology falls back to the pane's own tab ID
    assert_eq!(render_path(&t, &id("pnC")).unwrap(), "other/spB:none/r");
}

#[test]
fn render_string_round_trip() {
    let t = round_trip_fixture();
    for p in &t.panes {
        let path = render_path(&t, &p.target).unwrap();
        let parts: Vec<&str> = path.split('/').collect();
        assert_eq!(parts.len(), 3, "{path}");
        let space = t.spaces.iter().find(|s| s.id == p.space).unwrap();
        let tab = t.tabs.iter().find(|x| x.id == p.tab).unwrap();
        assert!(parts[0] == space.id || Some(parts[0]) == space.label.as_deref());
        assert!(parts[1] == tab.id || Some(parts[1]) == tab.label.as_deref());
        assert!(parts[2] == p.target.as_str() || Some(parts[2]) == p.label.as_deref());
    }
}

fn round_trip_fixture() -> HostTopology {
    HostTopology {
        spaces: vec![
            space("w1", "dup"),
            space("w2", "dup"),
            space("w3", "solid"),
            space("w4", "has/slash"),
        ],
        tabs: vec![
            tab("w1:t1", "w1", "tt"),
            tab("w1:t2", "w1", "tt"),
            tab("w1:t3", "w1", "deep"),
            tab("w3:t1", "w3", "tt"),
            tab("w3:t2", "w3", "other"),
            tab("w4:t1", "w4", "t/x"),
        ],
        panes: vec![
            pane("w1:p1", "w1", "w1:t1", "same", &[]),
            pane("w1:p2", "w1", "w1:t1", "same", &[]),
            pane("w1:p3", "w1", "w1:t1", "owner", &["agentx"]),
            // label equals another pane's agent name in the same tab
            pane("w1:p4", "w1", "w1:t1", "agentx", &[]),
            pane("w1:p5", "w1", "w1:t2", "same", &[]),
            // label containing '/'
            pane("w1:p6", "w1", "w1:t3", "a/b", &[]),
            pane("w1:p7", "w1", "w1:t3", "ok", &["ok"]),
            pane("w2:p1", "w2", "w2:t1x", "same", &[]),
            pane("w3:p1", "w3", "w3:t1", "same", &[]),
            pane("w3:p2", "w3", "w3:t2", "same", &[]),
            pane("w4:p1", "w4", "w4:t1", "z", &[]),
            // seat-ID-equal label alone in its tab (documented residual)
            pane("w3:p3", "w3", "w3:t3", "sAb12Cd34", &[]),
        ],
    }
    .with_tab(tab("w2:t1x", "w2", "tt"))
    .with_tab(tab("w3:t3", "w3", "solo-tab"))
}

trait WithTab {
    fn with_tab(self, tab: TopologyTab) -> Self;
}

impl WithTab for HostTopology {
    fn with_tab(mut self, tab: TopologyTab) -> Self {
        self.tabs.push(tab);
        self
    }
}

#[test]
fn render_resolve_round_trip() {
    let t = round_trip_fixture();
    for p in &t.panes {
        let path = render_path(&t, &p.target).unwrap();
        let c = resolved_or_panic(resolve(&path, &t, None, &none()), &path);
        assert_eq!(c.pane, p.target, "path {path}");
        assert_eq!(c.path, path);
    }
    // documented residual: a label equal to an existing seat ID takes the nickname form, and its
    // prefix still verifies against the pane (the stage then uses the seat ID).
    let residual = id("w3:p3");
    let path = render_path(&t, &residual).unwrap();
    assert_eq!(path, "solid/solo-tab/sAb12Cd34");
    let (prefix, last) = split_nickname(&path).unwrap();
    assert_eq!(last, "sAb12Cd34");
    assert_eq!(
        verify_nickname(prefix, &t, Some(&residual)),
        LocationCheck::Match
    );
}

fn resolved_or_panic(outcome: Outcome, path: &str) -> Candidate {
    match outcome {
        Outcome::Resolved(c) => c,
        other => panic!("path {path} did not resolve uniquely: {other:?}"),
    }
}

#[test]
fn reading_display_omits_unassigned_levels() {
    assert_eq!(
        reading(None, Some("tryout"), None).to_string(),
        "tab=tryout"
    );
    assert_eq!(
        reading(Some("a"), Some("b"), Some("c")).to_string(),
        "space=a tab=b pane=c"
    );
    assert_eq!(Tier::Tab.as_str(), "tab");
    assert_eq!(Tier::Space.as_str(), "space");
    assert_eq!(Tier::Any.as_str(), "any");
    assert!(Tier::Tab < Tier::Space && Tier::Space < Tier::Any);
}
