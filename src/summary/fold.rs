//! The ledger fold and its single renderer (spec §5). The fold is computed by
//! the daemon over level-0 records in sequence order; rollups add no records,
//! so rolling up never changes it. Pure: dropped transitions are returned for
//! the caller to log.
use crate::protocol::summary::{
    Fold, FoldDisplay, FoldEntry, Identifier, ItemBody, ItemStatus, LedgerItem, Level0Records,
    NewStatus, RuleChange, SeqRange, Transition, UserIntent,
};
use crate::summary::identifiers;
use std::collections::{BTreeMap, HashMap};

pub struct BlockRecords<'a> {
    pub range: SeqRange,
    pub records: &'a Level0Records,
}

/// Fold result before display rules: every introduced entry with its status,
/// the merged identifiers and the transitions the guards dropped.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FoldState {
    pub entries: Vec<FoldEntry>,
    pub identifiers: Vec<Identifier>,
    pub dropped: Vec<(Transition, &'static str)>,
}

/// Which statuses a transition may set, by item kind (spec §5).
pub fn status_allowed(body: &ItemBody, status: NewStatus) -> bool {
    match body {
        ItemBody::UserInstruction { user_intent, .. } => match user_intent {
            None => matches!(status, NewStatus::Done | NewStatus::Superseded),
            Some(UserIntent::Query | UserIntent::Request) => status == NewStatus::Resolved,
            Some(UserIntent::Rule) => status == NewStatus::Superseded,
        },
        ItemBody::OpenItem { .. } => matches!(status, NewStatus::Resolved | NewStatus::Superseded),
        ItemBody::Decision { .. } => status == NewStatus::Superseded,
    }
}

/// Structural evidence shared by submit validation and persisted-record folding.
/// `priority` must describe an ordinary canonical human or relayed message.
/// Exact rule quotes are checked only at submission and are not persisted.
pub(super) fn evidence_allowed(
    body: &ItemBody,
    status: NewStatus,
    rule_change: Option<RuleChange>,
    priority: bool,
) -> Result<(), &'static str> {
    let rule = matches!(
        body,
        ItemBody::UserInstruction {
            user_intent: Some(UserIntent::Rule),
            ..
        }
    );
    if rule && rule_change.is_none() {
        return Err("rule supersession needs rule_change");
    }
    if !rule && rule_change.is_some() {
        return Err("rule_change is allowed only for a rule");
    }
    if matches!(body, ItemBody::UserInstruction { .. })
        && status == NewStatus::Superseded
        && !priority
    {
        return Err("superseded instruction needs a priority citing message");
    }
    Ok(())
}

fn item_status(status: NewStatus) -> ItemStatus {
    match status {
        NewStatus::Done => ItemStatus::Done,
        NewStatus::Resolved => ItemStatus::Resolved,
        NewStatus::Superseded => ItemStatus::Superseded,
    }
}

/// Introductions and transitions are applied in message-sequence order,
/// independently of block arrival or cumulative source blocks' ranges.
/// Equal-sequence introductions precede transitions; ties retain range order
/// and then each record array's order. Identifiers merge independently.
///
/// Every transition keeps its own block's citation range. Its target must be
/// introduced strictly below the cite, remain open/active, accept the proposed
/// status, and satisfy the shared structural evidence guards. Priority reads
/// ordinary canonical human/relayed sources, never text or system events.
pub fn compute(
    blocks: &[BlockRecords<'_>],
    up_to_seq: u64,
    priority_at: &dyn Fn(u64) -> bool,
) -> FoldState {
    let mut ordered: Vec<&BlockRecords<'_>> = blocks
        .iter()
        .filter(|b| b.range.first_seq <= up_to_seq)
        .collect();
    ordered.sort_by_key(|b| (b.range.first_seq, b.range.last_seq));

    enum Event<'a> {
        Introduce(&'a LedgerItem),
        Transition(SeqRange, &'a Transition),
    }
    let mut events = Vec::new();
    let mut identifier_sets: Vec<&[Identifier]> = Vec::new();
    for block in ordered {
        identifier_sets.push(&block.records.identifiers);
        for item in &block.records.items {
            if item.seq <= up_to_seq {
                events.push(Event::Introduce(item));
            }
        }
        for transition in &block.records.transitions {
            events.push(Event::Transition(block.range, transition));
        }
    }
    events.sort_by_key(|event| match event {
        Event::Introduce(item) => (item.seq, 0),
        Event::Transition(_, transition) => (transition.cite_seq, 1),
    });

    let mut entries: Vec<FoldEntry> = Vec::new();
    let mut index: HashMap<String, usize> = HashMap::new();
    let mut dropped = Vec::new();
    for event in events {
        let (range, transition) = match event {
            Event::Introduce(item) => {
                if index.contains_key(&item.id) {
                    continue;
                }
                let status = match item.body {
                    ItemBody::Decision { .. }
                    | ItemBody::UserInstruction {
                        user_intent: Some(UserIntent::Rule),
                        ..
                    } => ItemStatus::Active,
                    _ => ItemStatus::Open,
                };
                index.insert(item.id.clone(), entries.len());
                entries.push(FoldEntry {
                    item: item.clone(),
                    status,
                    closed_at_seq: None,
                    display: FoldDisplay::Full,
                });
                continue;
            }
            Event::Transition(range, transition) => (range, transition),
        };
        let cite = transition.cite_seq;
        let verdict = if cite < range.first_seq || cite > range.last_seq {
            Err("cite_seq outside block")
        } else if cite > up_to_seq {
            Err("cite_seq beyond the fold frontier")
        } else if let Some(&at) = index.get(&transition.target_id) {
            let entry = &entries[at];
            if entry.item.seq >= cite {
                Err("target not introduced below cite_seq")
            } else if !matches!(entry.status, ItemStatus::Open | ItemStatus::Active) {
                Err("target not open")
            } else if !status_allowed(&entry.item.body, transition.new_status) {
                Err("new_status not allowed for target kind")
            } else {
                evidence_allowed(
                    &entry.item.body,
                    transition.new_status,
                    transition.rule_change,
                    priority_at(cite),
                )
                .map(|()| at)
            }
        } else {
            Err("unknown target")
        };
        match verdict {
            Ok(at) => {
                entries[at].status = item_status(transition.new_status);
                entries[at].closed_at_seq = Some(cite);
            }
            Err(reason) => dropped.push((transition.clone(), reason)),
        }
    }

    entries.sort_by(|a, b| (a.item.seq, &a.item.id).cmp(&(b.item.seq, &b.item.id)));
    FoldState {
        entries,
        identifiers: identifiers::merge(&identifier_sets),
        dropped,
    }
}

/// One log line's worth of the fold's dropped transitions (spec §5: "the drop
/// is logged"), or `None` when nothing dropped. Format:
/// `"{n} dropped transition(s): {reason} x{count} (#{seq}, #{seq}, …); {reason} ..."`
/// with reasons in sorted order, each listing up to 8 citing sequences in
/// ascending order (a trailing `…` marks elided ones) and the count of all
/// of them. Only daemon-checked `cite_seq` numbers appear, never `target_id`
/// or any other model-supplied text.
pub fn drop_report(dropped: &[(Transition, &'static str)]) -> Option<String> {
    const LISTED: usize = 8;
    if dropped.is_empty() {
        return None;
    }
    let mut by_reason: BTreeMap<&str, Vec<u64>> = BTreeMap::new();
    for (transition, reason) in dropped {
        by_reason
            .entry(reason)
            .or_default()
            .push(transition.cite_seq);
    }
    let parts: Vec<String> = by_reason
        .into_iter()
        .map(|(reason, mut seqs)| {
            seqs.sort_unstable();
            let count = seqs.len();
            let mut listed: Vec<String> =
                seqs.iter().take(LISTED).map(|s| format!("#{s}")).collect();
            if count > LISTED {
                listed.push("…".into());
            }
            format!("{reason} x{count} ({})", listed.join(", "))
        })
        .collect();
    Some(format!(
        "{} dropped transition(s): {}",
        dropped.len(),
        parts.join("; ")
    ))
}

/// The one renderer (Ready, rollup bundles, level-0 bundles). Open and active
/// entries are shown in full (`TextRef` when an instruction's text spilled to
/// `text_ref`); an entry closed at or after `window_first_seq` is one line;
/// earlier closures are omitted. An open instruction is never omitted.
///
/// Windows: Ready passes the first seq of the cover's level-0 window; a
/// level-0 bundle the chunk's `first_seq`; a rollup bundle the parent's.
pub fn render(state: &FoldState, window_first_seq: u64) -> Fold {
    let entries: Vec<FoldEntry> = state
        .entries
        .iter()
        .filter_map(|entry| {
            let display = match (entry.status, entry.closed_at_seq) {
                (ItemStatus::Open | ItemStatus::Active, _) => match &entry.item.body {
                    ItemBody::UserInstruction {
                        text: None,
                        text_ref: Some(_),
                        ..
                    } => FoldDisplay::TextRef,
                    _ => FoldDisplay::Full,
                },
                (_, Some(closed)) if closed >= window_first_seq => FoldDisplay::OneLine,
                _ => return None,
            };
            Some(FoldEntry {
                display,
                ..entry.clone()
            })
        })
        .collect();
    // Every serialized byte the caller receives except this field itself.
    let serialized_len = |bytes: serde_json::Result<Vec<u8>>| bytes.map_or(0, |b| b.len());
    let rendered_bytes = (serialized_len(serde_json::to_vec(&entries))
        + serialized_len(serde_json::to_vec(&state.identifiers))) as u32;
    Fold {
        entries,
        identifiers: state.identifiers.clone(),
        rendered_bytes,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::{
        ids::SeatId,
        summary::{AuthorRole, IdentifierKind, LedgerItem, OpenItemKind},
    };

    fn instruction(seq: u64) -> LedgerItem {
        LedgerItem {
            id: format!("i.{seq}"),
            seq,
            body: ItemBody::UserInstruction {
                author_seat: None,
                author_role: Some(AuthorRole::Human),
                relays_user: false,
                user_intent: None,
                text: Some(format!("do {seq}")),
                text_ref: None,
                message_id: None,
            },
        }
    }
    fn open_item(id: &str, seq: u64) -> LedgerItem {
        LedgerItem {
            id: id.into(),
            seq,
            body: ItemBody::OpenItem {
                kind: OpenItemKind::Ask,
                from_seat: SeatId::new("sa"),
                to_seat: None,
                text: format!("ask {seq}"),
            },
        }
    }
    fn decision(id: &str, seq: u64) -> LedgerItem {
        LedgerItem {
            id: id.into(),
            seq,
            body: ItemBody::Decision {
                by_seat: SeatId::new("sa"),
                text: format!("decide {seq}"),
            },
        }
    }
    fn tr(target: &str, status: NewStatus, cite: u64) -> Transition {
        Transition {
            target_id: target.into(),
            new_status: status,
            cite_seq: cite,
            rule_change: None,
        }
    }
    #[test]
    fn drop_report_is_none_without_drops() {
        assert_eq!(drop_report(&[]), None);
    }

    #[test]
    fn drop_report_counts_per_reason() {
        let dropped = [
            (tr("a", NewStatus::Done, 40), "target not open"),
            (tr("b", NewStatus::Done, 7), "unknown target"),
            (tr("c", NewStatus::Done, 12), "target not open"),
        ];
        assert_eq!(
            drop_report(&dropped).as_deref(),
            Some("3 dropped transition(s): target not open x2 (#12, #40); unknown target x1 (#7)")
        );
    }

    #[test]
    fn drop_report_caps_listed_sequences() {
        let dropped: Vec<_> = (1..=10)
            .rev()
            .map(|seq| (tr("a", NewStatus::Done, seq), "target not open"))
            .collect();
        assert_eq!(
            drop_report(&dropped).as_deref(),
            Some(
                "10 dropped transition(s): target not open x10 (#1, #2, #3, #4, #5, #6, #7, #8, …)"
            )
        );
    }

    #[test]
    fn drop_report_never_prints_target_ids() {
        let dropped = [(tr("secret-target", NewStatus::Done, 3), "unknown target")];
        let report = drop_report(&dropped).unwrap();
        assert!(!report.contains("secret-target"), "{report}");
        assert!(report.contains("#3"), "{report}");
    }

    fn records(items: Vec<LedgerItem>, transitions: Vec<Transition>) -> Level0Records {
        Level0Records {
            items,
            identifiers: vec![],
            transitions,
        }
    }
    fn range(a: u64, b: u64) -> SeqRange {
        SeqRange {
            first_seq: a,
            last_seq: b,
        }
    }
    fn status_of(state: &FoldState, id: &str) -> (ItemStatus, Option<u64>) {
        let e = state.entries.iter().find(|e| e.item.id == id).unwrap();
        (e.status, e.closed_at_seq)
    }
    fn ids(fold: &Fold) -> Vec<&str> {
        fold.entries.iter().map(|e| e.item.id.as_str()).collect()
    }
    const NONE_PRIORITY: &dyn Fn(u64) -> bool = &|_| false;

    #[test]
    fn transition_guards() {
        // Block A (1..10) introduces an open item at 3 and an instruction at 4.
        let a = records(vec![open_item("a.0.1", 3), instruction(4)], vec![]);
        let cases: Vec<(Transition, bool, Option<&str>)> = vec![
            (tr("a.0.1", NewStatus::Resolved, 15), true, None),
            (
                tr("a.0.1", NewStatus::Resolved, 25),
                false,
                Some("cite_seq outside block"),
            ),
            (
                tr("a.0.1", NewStatus::Resolved, 5),
                false,
                Some("cite_seq outside block"),
            ),
            (
                // introduced at 16, cited at 16 (not below)
                tr("b.1.1", NewStatus::Resolved, 16),
                false,
                Some("target not introduced below cite_seq"),
            ),
            (
                tr("nope", NewStatus::Resolved, 15),
                false,
                Some("unknown target"),
            ),
            (
                tr("a.0.1", NewStatus::Done, 15),
                false,
                Some("new_status not allowed for target kind"),
            ),
            (
                tr("i.4", NewStatus::Superseded, 15),
                false,
                Some("superseded instruction needs a priority citing message"),
            ),
            (tr("i.4", NewStatus::Done, 15), true, None),
        ];
        for (transition, applies, reason) in cases {
            let b = records(vec![open_item("b.1.1", 16)], vec![transition.clone()]);
            let blocks = [
                BlockRecords {
                    range: range(1, 10),
                    records: &a,
                },
                BlockRecords {
                    range: range(11, 20),
                    records: &b,
                },
            ];
            let state = compute(&blocks, 20, NONE_PRIORITY);
            assert_eq!(state.dropped.len(), usize::from(!applies), "{transition:?}");
            if applies {
                assert_eq!(
                    status_of(&state, &transition.target_id).1,
                    Some(transition.cite_seq)
                );
            } else {
                assert_eq!(state.dropped[0].0, transition);
                assert_eq!(Some(state.dropped[0].1), reason, "{transition:?}");
            }
        }

        // A superseding instruction citing a priority message applies; a
        // non-priority one drops (same transition, different message).
        let b = records(vec![], vec![tr("i.4", NewStatus::Superseded, 15)]);
        let blocks = [
            BlockRecords {
                range: range(1, 10),
                records: &a,
            },
            BlockRecords {
                range: range(11, 20),
                records: &b,
            },
        ];
        let state = compute(&blocks, 20, &|seq| seq == 15);
        assert!(state.dropped.is_empty());
        assert_eq!(status_of(&state, "i.4"), (ItemStatus::Superseded, Some(15)));

        // Already closed: the second closure drops.
        let b = records(
            vec![],
            vec![
                tr("a.0.1", NewStatus::Resolved, 12),
                tr("a.0.1", NewStatus::Superseded, 14),
            ],
        );
        let blocks = [
            BlockRecords {
                range: range(1, 10),
                records: &a,
            },
            BlockRecords {
                range: range(11, 20),
                records: &b,
            },
        ];
        let state = compute(&blocks, 20, NONE_PRIORITY);
        assert_eq!(state.dropped.len(), 1);
        assert_eq!(state.dropped[0].1, "target not open");
        assert_eq!(status_of(&state, "a.0.1"), (ItemStatus::Resolved, Some(12)));
        assert_eq!(
            drop_report(&state.dropped).as_deref(),
            Some("1 dropped transition(s): target not open x1 (#14)")
        );
    }

    #[test]
    fn transitions_cannot_reach_beyond_the_frontier_or_a_decisions_wrong_status() {
        let a = records(
            vec![decision("d", 2)],
            vec![
                tr("d", NewStatus::Resolved, 5),
                tr("d", NewStatus::Superseded, 9),
            ],
        );
        let blocks = [BlockRecords {
            range: range(1, 10),
            records: &a,
        }];
        // Frontier 7: the cite at 9 is beyond it.
        let state = compute(&blocks, 7, NONE_PRIORITY);
        let reasons: Vec<&str> = state.dropped.iter().map(|d| d.1).collect();
        assert_eq!(
            reasons,
            [
                "new_status not allowed for target kind",
                "cite_seq beyond the fold frontier"
            ]
        );
        assert_eq!(status_of(&state, "d").0, ItemStatus::Active);
        let state = compute(&blocks, 10, NONE_PRIORITY);
        assert_eq!(status_of(&state, "d").0, ItemStatus::Superseded);
    }

    #[test]
    fn same_block_closure_via_ref() {
        let b = records(
            vec![open_item("c.1.1", 12)],
            vec![tr("c.1.1", NewStatus::Resolved, 14)],
        );
        let blocks = [BlockRecords {
            range: range(11, 20),
            records: &b,
        }];
        let state = compute(&blocks, 20, NONE_PRIORITY);
        assert!(state.dropped.is_empty());
        assert_eq!(status_of(&state, "c.1.1"), (ItemStatus::Resolved, Some(14)));
        // Citing the introducing seq itself is not "below".
        let b = records(
            vec![open_item("c.1.1", 12)],
            vec![tr("c.1.1", NewStatus::Resolved, 12)],
        );
        let blocks = [BlockRecords {
            range: range(11, 20),
            records: &b,
        }];
        let state = compute(&blocks, 20, NONE_PRIORITY);
        assert_eq!(state.dropped.len(), 1);
        assert_eq!(status_of(&state, "c.1.1").0, ItemStatus::Open);
    }

    #[test]
    fn order_across_blocks_is_by_sequence() {
        let a = records(vec![open_item("a.0.1", 3)], vec![]);
        let b = records(vec![], vec![tr("a.0.1", NewStatus::Resolved, 15)]);
        let in_order = [
            BlockRecords {
                range: range(1, 10),
                records: &a,
            },
            BlockRecords {
                range: range(11, 20),
                records: &b,
            },
        ];
        let shuffled = [
            BlockRecords {
                range: range(11, 20),
                records: &b,
            },
            BlockRecords {
                range: range(1, 10),
                records: &a,
            },
        ];
        let x = compute(&in_order, 20, NONE_PRIORITY);
        let y = compute(&shuffled, 20, NONE_PRIORITY);
        assert_eq!(x, y);
        assert!(x.dropped.is_empty());
        assert_eq!(status_of(&x, "a.0.1"), (ItemStatus::Resolved, Some(15)));
        // Blocks past the frontier contribute nothing.
        let z = compute(&in_order, 10, NONE_PRIORITY);
        assert_eq!(status_of(&z, "a.0.1"), (ItemStatus::Open, None));
    }

    #[test]
    fn display_rule() {
        // One block 1..=20 holds every item and closure.
        let all = records(
            vec![
                instruction(2),
                open_item("o.open", 3),
                open_item("o.old", 4),
                open_item("o.new", 5),
                instruction(6),
                instruction(7),
                decision("d.act", 8),
                decision("d.old", 9),
            ],
            vec![
                tr("o.old", NewStatus::Resolved, 6),
                tr("o.new", NewStatus::Resolved, 18),
                tr("i.6", NewStatus::Done, 7),
                tr("i.7", NewStatus::Done, 15),
                tr("d.old", NewStatus::Superseded, 16),
            ],
        );
        let blocks = [BlockRecords {
            range: range(1, 20),
            records: &all,
        }];
        let state = compute(&blocks, 20, NONE_PRIORITY);
        assert!(state.dropped.is_empty(), "{:?}", state.dropped);
        let fold = render(&state, 11);
        // o.old (closed 6) and i.6 (done 7) are omitted; o.new (18), i.7 (15)
        // and d.old (16) are one line; open instruction i.2 is never omitted.
        assert_eq!(
            ids(&fold),
            ["i.2", "o.open", "o.new", "i.7", "d.act", "d.old"]
        );
        let display = |id: &str| {
            fold.entries
                .iter()
                .find(|e| e.item.id == id)
                .unwrap()
                .display
        };
        assert_eq!(display("i.2"), FoldDisplay::Full);
        assert_eq!(display("o.open"), FoldDisplay::Full);
        assert_eq!(display("d.act"), FoldDisplay::Full);
        assert_eq!(display("o.new"), FoldDisplay::OneLine);
        assert_eq!(display("i.7"), FoldDisplay::OneLine);
        assert_eq!(display("d.old"), FoldDisplay::OneLine);
        // A window starting at 1 shows every closure as one line.
        assert_eq!(render(&state, 1).entries.len(), state.entries.len());
        // Boundary: closed exactly at window_first is one line.
        assert!(ids(&render(&state, 15)).contains(&"i.7"));
        assert!(!ids(&render(&state, 16)).contains(&"i.7"));
    }

    #[test]
    fn open_instruction_with_spilled_text_renders_as_text_ref() {
        let mut item = instruction(3);
        if let ItemBody::UserInstruction { text, text_ref, .. } = &mut item.body {
            *text = None;
            *text_ref = Some(3);
        }
        let a = records(vec![item], vec![]);
        let blocks = [BlockRecords {
            range: range(1, 10),
            records: &a,
        }];
        let fold = render(&compute(&blocks, 10, NONE_PRIORITY), 1000);
        assert_eq!(fold.entries[0].display, FoldDisplay::TextRef);
    }

    #[test]
    fn merge_across_two_levels() {
        // 16 level-0 blocks of 10 messages each (two rollups' worth); one
        // instruction per block, and one open item raised in block 2 and closed
        // in block 5 (inside rollup 1's range 1..=80).
        let mut recs: Vec<Level0Records> = Vec::new();
        for n in 0..16u64 {
            let first = n * 10 + 1;
            let mut items = vec![instruction(first)];
            let mut transitions = vec![];
            if n == 1 {
                items.push(open_item("cv.1.1", first + 2));
            }
            if n == 4 {
                transitions.push(tr("cv.1.1", NewStatus::Resolved, first + 3));
            }
            if n == 12 {
                transitions.push(tr("i.1", NewStatus::Done, first + 4));
            }
            recs.push(Level0Records {
                items,
                identifiers: vec![Identifier {
                    value: "src/lib.rs".into(),
                    kind: IdentifierKind::Path,
                    seqs: vec![first],
                }],
                transitions,
            });
        }
        let blocks: Vec<BlockRecords<'_>> = recs
            .iter()
            .enumerate()
            .map(|(n, r)| BlockRecords {
                range: range(n as u64 * 10 + 1, n as u64 * 10 + 10),
                records: r,
            })
            .collect();
        let state = compute(&blocks, 160, NONE_PRIORITY);
        assert!(state.dropped.is_empty(), "{:?}", state.dropped);
        // Rollups add no records: computing over only the first 8 blocks and
        // then all 16 agree on everything the first 8 decided.
        let first_half = compute(&blocks[..8], 80, NONE_PRIORITY);
        assert_eq!(
            status_of(&first_half, "cv.1.1"),
            status_of(&state, "cv.1.1")
        );

        // Rollup 1 (parent first_seq 1): the closure is inside its children, one line.
        let rollup1 = render(&state, 1);
        let line = |fold: &Fold, id: &str| {
            fold.entries
                .iter()
                .find(|e| e.item.id == id)
                .map(|e| e.display)
        };
        assert_eq!(line(&rollup1, "cv.1.1"), Some(FoldDisplay::OneLine));
        // Rollup 2 (parent first_seq 81): the earlier closure is dropped, the
        // open instructions stay, the instruction done at 125 is one line.
        let rollup2 = render(&state, 81);
        assert_eq!(line(&rollup2, "cv.1.1"), None);
        assert_eq!(line(&rollup2, "i.1"), Some(FoldDisplay::OneLine));
        assert_eq!(line(&rollup2, "i.11"), Some(FoldDisplay::Full));
        assert_eq!(line(&rollup2, "i.151"), Some(FoldDisplay::Full));
        // Identifiers merge by value across all 16 blocks.
        assert_eq!(state.identifiers.len(), 1);
        assert_eq!(state.identifiers[0].seqs.len(), 16);
    }

    #[test]
    fn rendered_bytes_counts_entries_and_identifiers_and_is_stable() {
        let mut a = records(vec![instruction(2), open_item("o", 3)], vec![]);
        a.identifiers = vec![Identifier {
            value: "src/lib.rs".into(),
            kind: IdentifierKind::Path,
            seqs: vec![2],
        }];
        let blocks = [BlockRecords {
            range: range(1, 10),
            records: &a,
        }];
        let state = compute(&blocks, 10, NONE_PRIORITY);
        let first = render(&state, 1);
        let second = render(&compute(&blocks, 10, NONE_PRIORITY), 1);
        assert_eq!(first, second);
        assert_eq!(first.identifiers.len(), 1);
        let expected = (serde_json::to_vec(&first.entries).unwrap().len()
            + serde_json::to_vec(&first.identifiers).unwrap().len()) as u32;
        assert_eq!(first.rendered_bytes, expected);
        assert!(first.rendered_bytes > 0);
        // Omitting entries shrinks it.
        let empty = render(&compute(&[], 10, NONE_PRIORITY), 1);
        assert_eq!(empty.rendered_bytes, 2 + 2);
    }
    fn classified(seq: u64, intent: crate::protocol::summary::UserIntent) -> LedgerItem {
        let mut item = instruction(seq);
        if let ItemBody::UserInstruction {
            user_intent,
            message_id,
            ..
        } = &mut item.body
        {
            *user_intent = Some(intent);
            *message_id = Some(crate::protocol::ids::MessageId::new(format!("m{seq}")));
        }
        item
    }

    #[test]
    fn user_intent_status_matrix() {
        use crate::protocol::summary::{
            RuleChange,
            UserIntent::{Query, Request, Rule},
        };
        let cases = [
            (
                instruction(1),
                ItemStatus::Open,
                vec![NewStatus::Done, NewStatus::Superseded],
            ),
            (
                classified(1, Query),
                ItemStatus::Open,
                vec![NewStatus::Resolved],
            ),
            (
                classified(1, Request),
                ItemStatus::Open,
                vec![NewStatus::Resolved],
            ),
            (
                classified(1, Rule),
                ItemStatus::Active,
                vec![NewStatus::Superseded],
            ),
            (
                open_item("o", 1),
                ItemStatus::Open,
                vec![NewStatus::Resolved, NewStatus::Superseded],
            ),
            (
                decision("d", 1),
                ItemStatus::Active,
                vec![NewStatus::Superseded],
            ),
        ];
        for (item, initial, allowed) in cases {
            let rec = records(vec![item.clone()], vec![]);
            let blocks = [BlockRecords {
                range: range(1, 9),
                records: &rec,
            }];
            assert_eq!(
                status_of(&compute(&blocks, 9, &|_| true), &item.id),
                (initial, None)
            );
            for status in [NewStatus::Done, NewStatus::Resolved, NewStatus::Superseded] {
                let applies = allowed.contains(&status);
                assert_eq!(
                    status_allowed(&item.body, status),
                    applies,
                    "{:?} {status:?}",
                    item.body
                );
                let mut transition = tr(&item.id, status, 8);
                if matches!(
                    item.body,
                    ItemBody::UserInstruction {
                        user_intent: Some(Rule),
                        ..
                    }
                ) {
                    transition.rule_change = Some(RuleChange::Withdrawn);
                }
                let rec = records(vec![item.clone()], vec![transition]);
                let blocks = [BlockRecords {
                    range: range(1, 9),
                    records: &rec,
                }];
                let state = compute(&blocks, 9, &|_| true);
                assert_eq!(state.dropped.len(), usize::from(!applies));
                assert_eq!(
                    status_of(&state, &item.id),
                    if applies {
                        (item_status(status), Some(8))
                    } else {
                        (initial, None)
                    }
                );
            }
        }
    }

    #[test]
    fn user_intent_fold_uses_source_sequence_not_block_arrival() {
        use crate::protocol::summary::UserIntent::Query;
        let mut seed = records(vec![classified(2, Query)], vec![]);
        seed.identifiers.push(Identifier {
            value: "src/a.rs".into(),
            kind: IdentifierKind::Path,
            seqs: vec![2],
        });
        let late_a = records(vec![classified(2, Query), classified(2, Query)], vec![]);
        let b = records(vec![], vec![tr("i.2", NewStatus::Resolved, 8)]);
        // Duplicate local introductions and a cumulative seed share one ID,
        // independently of block input order.
        let blocks = [
            BlockRecords {
                range: range(8, 8),
                records: &b,
            },
            BlockRecords {
                range: range(2, 2),
                records: &late_a,
            },
            BlockRecords {
                range: range(1, 9),
                records: &seed,
            },
        ];
        let state = compute(&blocks, 9, NONE_PRIORITY);
        let reversed: Vec<_> = blocks.into_iter().rev().collect();
        assert_eq!(state, compute(&reversed, 9, NONE_PRIORITY));
        assert_eq!(state.entries.len(), 1);
        assert_eq!(status_of(&state, "i.2"), (ItemStatus::Resolved, Some(8)));
        assert_eq!(state.identifiers[0].seqs, vec![2]);
        // The source seed spans #1..9 but its local display range sorts after
        // B; introductions must use source sequence instead of that range.
        let cumulative = records(
            vec![instruction(1), classified(2, Query), classified(9, Query)],
            vec![],
        );
        let blocks = [
            BlockRecords {
                range: range(8, 8),
                records: &b,
            },
            BlockRecords {
                range: range(8, 9),
                records: &cumulative,
            },
        ];
        let state = compute(&blocks, 9, NONE_PRIORITY);
        assert_eq!(
            state.entries.iter().filter(|e| e.item.id == "i.2").count(),
            1
        );
        assert_eq!(status_of(&state, "i.2"), (ItemStatus::Resolved, Some(8)));
        assert_eq!(status_of(&state, "i.9"), (ItemStatus::Open, None));
        let reversed: Vec<_> = blocks.into_iter().rev().collect();
        assert_eq!(state, compute(&reversed, 9, NONE_PRIORITY));

        // The first closure in message sequence wins, even when the broader
        // block containing the later closure sorts first.
        let later = records(
            vec![classified(2, Query)],
            vec![tr("i.2", NewStatus::Resolved, 9)],
        );
        let earlier = records(vec![], vec![tr("i.2", NewStatus::Resolved, 8)]);
        let blocks = [
            BlockRecords {
                range: range(1, 9),
                records: &later,
            },
            BlockRecords {
                range: range(8, 8),
                records: &earlier,
            },
        ];
        let state = compute(&blocks, 9, NONE_PRIORITY);
        assert_eq!(status_of(&state, "i.2"), (ItemStatus::Resolved, Some(8)));
        assert_eq!(state.dropped[0].0.cite_seq, 9);
        for cite in [1, 2, 10] {
            let bad = records(
                vec![classified(2, Query)],
                vec![tr("i.2", NewStatus::Resolved, cite)],
            );
            let blocks = [BlockRecords {
                range: range(1, 9),
                records: &bad,
            }];
            let state = compute(&blocks, 9, NONE_PRIORITY);
            assert_eq!(state.dropped.len(), 1);
            assert_eq!(status_of(&state, "i.2"), (ItemStatus::Open, None));
        }
    }

    fn worker_keeps_open(items: Vec<LedgerItem>, narrative: &str) -> Level0Records {
        let submission = crate::protocol::summary::Submission::parse(&serde_json::json!({
            "submission_schema": crate::protocol::summary::SUBMISSION_SCHEMA,
            "narrative": narrative, "prompt_version": "p", "model": "m",
            "transitions": [],
        }))
        .unwrap();
        crate::summary::ledger::level0_records(&items, &[], &submission, "cv", 0)
    }

    #[test]
    fn user_intent_partial_ack_no_transition_stays_open() {
        use crate::protocol::summary::UserIntent::{Query, Request};
        for intent in [Query, Request] {
            for reply in ["partial answer", "promise to finish", "ACK"] {
                let messages = worker_keeps_open(vec![classified(1, intent)], reply);
                let blocks = [BlockRecords {
                    range: range(1, 2),
                    records: &messages,
                }];
                assert_eq!(
                    status_of(&compute(&blocks, 2, NONE_PRIORITY), "i.1"),
                    (ItemStatus::Open, None),
                    "{reply}"
                );
            }
        }
    }

    #[test]
    fn user_intent_rule_compliance_no_transition_stays_active() {
        use crate::protocol::summary::UserIntent::Rule;
        let rec = worker_keeps_open(
            vec![classified(1, Rule)],
            "Released after running tests; ACK received",
        );
        let blocks = [BlockRecords {
            range: range(1, 3),
            records: &rec,
        }];
        assert_eq!(
            status_of(&compute(&blocks, 3, NONE_PRIORITY), "i.1"),
            (ItemStatus::Active, None)
        );
    }

    #[test]
    fn user_intent_closed_display_window() {
        use crate::protocol::summary::{
            RuleChange,
            UserIntent::{Query, Request, Rule},
        };
        let mut items = vec![
            classified(1, Query),
            classified(2, Request),
            classified(3, Rule),
            classified(4, Query),
            classified(5, Request),
            classified(6, Rule),
        ];
        for item in &mut items[3..] {
            if let ItemBody::UserInstruction { text, text_ref, .. } = &mut item.body {
                *text = None;
                *text_ref = Some(item.seq);
            }
        }
        let mut rule_close = tr("i.3", NewStatus::Superseded, 9);
        rule_close.rule_change = Some(RuleChange::Replaced);
        let rec = records(
            items.clone(),
            vec![
                tr("i.1", NewStatus::Resolved, 7),
                tr("i.2", NewStatus::Resolved, 8),
                rule_close,
            ],
        );
        let blocks = [BlockRecords {
            range: range(1, 10),
            records: &rec,
        }];
        let state = compute(&blocks, 10, &|_| true);
        assert!(state.dropped.is_empty());
        let recent = render(&state, 7);
        assert_eq!(
            recent.entries.iter().map(|e| e.display).collect::<Vec<_>>(),
            [
                FoldDisplay::OneLine,
                FoldDisplay::OneLine,
                FoldDisplay::OneLine,
                FoldDisplay::TextRef,
                FoldDisplay::TextRef,
                FoldDisplay::TextRef
            ]
        );
        let old = render(&state, 10);
        assert_eq!(
            old.entries.iter().map(|e| &e.item).collect::<Vec<_>>(),
            items[3..].iter().collect::<Vec<_>>()
        );
    }

    #[test]
    fn user_intent_rule_persisted_evidence_guards() {
        use crate::protocol::summary::{
            RuleChange,
            UserIntent::{Query, Request, Rule},
        };
        let rule = classified(2, Rule);
        for (change, priority, applies) in [
            (None, true, false),
            (Some(RuleChange::Withdrawn), false, false),
            (Some(RuleChange::Withdrawn), true, true),
            (Some(RuleChange::Replaced), true, true),
        ] {
            let mut transition = tr("i.2", NewStatus::Superseded, 8);
            transition.rule_change = change;
            let rec = records(vec![rule.clone()], vec![transition]);
            let blocks = [BlockRecords {
                range: range(1, 9),
                records: &rec,
            }];
            let state = compute(&blocks, 9, &|_| priority);
            assert_eq!(state.dropped.len(), usize::from(!applies));
            assert_eq!(
                status_of(&state, "i.2"),
                if applies {
                    (ItemStatus::Superseded, Some(8))
                } else {
                    (ItemStatus::Active, None)
                }
            );
        }
        for (item, status) in [
            (instruction(2), NewStatus::Done),
            (classified(2, Query), NewStatus::Resolved),
            (classified(2, Request), NewStatus::Resolved),
            (open_item("o", 2), NewStatus::Resolved),
            (decision("d", 2), NewStatus::Superseded),
        ] {
            let mut transition = tr(&item.id, status, 8);
            transition.rule_change = Some(RuleChange::Replaced);
            let rec = records(vec![item.clone()], vec![transition]);
            let blocks = [BlockRecords {
                range: range(1, 9),
                records: &rec,
            }];
            let state = compute(&blocks, 9, &|_| true);
            assert_eq!(state.dropped.len(), 1, "{:?}", item.body);
            assert_eq!(state.dropped[0].1, "rule_change is allowed only for a rule");
            assert_eq!(status_of(&state, &item.id).1, None);
        }
    }
}
