//! Level-0 ledger records (spec §5): the daemon's instruction prefill, the
//! records a validated submission becomes, and the ledger-only fallback.
use crate::protocol::{
    results::MessageKind,
    summary::{
        BundleMessage, Identifier, ItemBody, LedgerItem, Level0Records, PREFILL_TEXT_MAX_BYTES,
        Submission, Transition, is_priority,
    },
};
use std::collections::HashMap;

/// One `user_instruction` per priority message, in sequence order of `messages`.
/// `text` is verbatim up to `PREFILL_TEXT_MAX_BYTES`, else `text_ref` names the
/// message's sequence. Info/warn event lines are never prefilled.
pub fn prefill(messages: &[BundleMessage]) -> Vec<LedgerItem> {
    messages
        .iter()
        .filter(|m| m.kind == MessageKind::Ordinary && is_priority(m.author_role, m.relays_user))
        .map(|m| {
            let verbatim = m.text.len() <= PREFILL_TEXT_MAX_BYTES;
            LedgerItem {
                id: format!("i.{}", m.sequence),
                seq: m.sequence,
                body: ItemBody::UserInstruction {
                    author_seat: m.author.clone(),
                    author_role: m.author_role,
                    relays_user: m.relays_user,
                    user_intent: m.user_intent,
                    text: verbatim.then(|| m.text.clone()),
                    text_ref: (!verbatim).then_some(m.sequence),
                    message_id: Some(m.message.clone()),
                },
            }
        })
        .collect()
}

/// The records a level-0 block stores for a validated `submission`: the prefill,
/// then model items numbered `<chunking_version>.<chunk_index>.<n>` (`n` from 1,
/// over `new_decisions` then `new_open_items`, each in array order), the
/// daemon's identifiers, and the transitions with same-submission `ref` targets
/// rewritten to the assigned ids (bundle-fold ids pass through).
pub fn level0_records(
    prefill: &[LedgerItem],
    identifiers: &[Identifier],
    submission: &Submission,
    chunking_version: &str,
    chunk_index: u64,
) -> Level0Records {
    let mut items = prefill.to_vec();
    let mut assigned: HashMap<&str, String> = HashMap::new();
    let mut n = 0u32;
    let mut next_id = || {
        n += 1;
        format!("{chunking_version}.{chunk_index}.{n}")
    };
    for decision in &submission.new_decisions {
        let id = next_id();
        assigned.insert(decision.reference.as_str(), id.clone());
        items.push(LedgerItem {
            id,
            seq: decision.seq,
            body: ItemBody::Decision {
                by_seat: decision.by_seat.clone(),
                text: decision.text.clone(),
            },
        });
    }
    for open in &submission.new_open_items {
        let id = next_id();
        assigned.insert(open.reference.as_str(), id.clone());
        items.push(LedgerItem {
            id,
            seq: open.seq,
            body: ItemBody::OpenItem {
                kind: open.kind,
                from_seat: open.from_seat.clone(),
                to_seat: open.to_seat.clone(),
                text: open.text.clone(),
            },
        });
    }
    let transitions = submission
        .transitions
        .iter()
        .map(|t| Transition {
            target_id: assigned
                .get(t.target.as_str())
                .cloned()
                .unwrap_or_else(|| t.target.clone()),
            new_status: t.new_status,
            cite_seq: t.cite_seq,
            rule_change: t.rule_change,
        })
        .collect();
    Level0Records {
        items,
        identifiers: identifiers.to_vec(),
        transitions,
    }
}

/// Ledger-only fallback block records: prefill and identifiers, nothing
/// model-proposed (spec §3).
pub fn fallback_records(prefill: &[LedgerItem], identifiers: &[Identifier]) -> Level0Records {
    Level0Records {
        items: prefill.to_vec(),
        identifiers: identifiers.to_vec(),
        transitions: Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::{
        ids::{MessageId, SeatId},
        summary::{
            AuthorRole, IdentifierKind, NewDecision, NewOpenItem, NewStatus, OpenItemKind,
            ProposedTransition, SUBMISSION_SCHEMA,
        },
        time::UtcMillis,
    };

    fn msg(seq: u64, role: Option<AuthorRole>, relays: bool, text: &str) -> BundleMessage {
        BundleMessage {
            sequence: seq,
            message: MessageId::new(format!("m{seq}")),
            kind: MessageKind::Ordinary,
            author: Some(SeatId::new("sa")),
            author_role: role,
            relays_user: relays,
            user_intent: None,
            created_at: UtcMillis(0),
            text: text.into(),
        }
    }

    #[test]
    fn prefill_has_full_priority_recall() {
        let big = "x".repeat(PREFILL_TEXT_MAX_BYTES + 1);
        let exact = "y".repeat(PREFILL_TEXT_MAX_BYTES);
        let mut event = msg(9, Some(AuthorRole::Human), true, "joined");
        event.kind = MessageKind::Info;
        let messages = vec![
            msg(1, Some(AuthorRole::Human), false, "do the thing"),
            msg(2, Some(AuthorRole::Agent), false, "ok"),
            msg(3, Some(AuthorRole::Agent), true, "user said ship it"),
            msg(4, None, false, "backfilled"),
            msg(5, None, true, "relayed, no role"),
            msg(6, Some(AuthorRole::Service), false, "notice"),
            msg(7, Some(AuthorRole::Human), false, &big),
            msg(8, Some(AuthorRole::Human), false, &exact),
            event,
        ];
        let items = prefill(&messages);
        let ids: Vec<&str> = items.iter().map(|i| i.id.as_str()).collect();
        assert_eq!(ids, ["i.1", "i.3", "i.5", "i.7", "i.8"]);
        assert!(items.iter().all(|i| i.id == format!("i.{}", i.seq)));
        let ItemBody::UserInstruction {
            author_seat,
            author_role,
            relays_user,
            user_intent: None,
            text,
            text_ref,
            message_id,
        } = &items[0].body
        else {
            panic!("not an instruction");
        };
        assert_eq!(author_seat, &Some(SeatId::new("sa")));
        assert_eq!(*author_role, Some(AuthorRole::Human));
        assert!(!relays_user);
        assert_eq!(text.as_deref(), Some("do the thing"));
        assert_eq!(*text_ref, None);
        assert_eq!(message_id, &Some(MessageId::new("m1")));
        // Every prefilled item, verbatim or spilled, names its source message.
        for item in &items {
            let ItemBody::UserInstruction { message_id, .. } = &item.body else {
                panic!()
            };
            assert_eq!(message_id, &Some(MessageId::new(format!("m{}", item.seq))));
        }
        // Oversized instruction spills to text_ref; exactly-at-limit stays verbatim.
        let ItemBody::UserInstruction { text, text_ref, .. } = &items[3].body else {
            panic!()
        };
        assert_eq!((text.as_deref(), *text_ref), (None, Some(7)));
        let ItemBody::UserInstruction { text, text_ref, .. } = &items[4].body else {
            panic!()
        };
        assert_eq!((text.as_deref(), *text_ref), (Some(exact.as_str()), None));
    }

    fn submission() -> Submission {
        Submission {
            submission_schema: SUBMISSION_SCHEMA,
            narrative: "n".into(),
            new_decisions: vec![
                NewDecision {
                    reference: "d1".into(),
                    seq: 31,
                    by_seat: SeatId::new("sa"),
                    text: "use sqlite".into(),
                    quote: None,
                },
                NewDecision {
                    reference: "d2".into(),
                    seq: 32,
                    by_seat: SeatId::new("sb"),
                    text: "no wal".into(),
                    quote: None,
                },
            ],
            new_open_items: vec![NewOpenItem {
                reference: "o1".into(),
                seq: 33,
                kind: OpenItemKind::Ask,
                from_seat: SeatId::new("sa"),
                to_seat: None,
                text: "who owns it?".into(),
                quote: None,
            }],
            transitions: vec![
                ProposedTransition {
                    target: "d1".into(),
                    new_status: NewStatus::Superseded,
                    cite_seq: 32,
                    rule_change: None,
                    quote: None,
                },
                ProposedTransition {
                    target: "o1".into(),
                    new_status: NewStatus::Resolved,
                    cite_seq: 34,
                    rule_change: None,
                    quote: None,
                },
                ProposedTransition {
                    target: "i.31".into(),
                    new_status: NewStatus::Done,
                    cite_seq: 34,
                    rule_change: None,
                    quote: None,
                },
            ],
            prompt_version: "p".into(),
            model: "m".into(),
        }
    }

    #[test]
    fn level0_records_number_model_items_in_submission_order() {
        let pre = prefill(&[msg(31, Some(AuthorRole::Human), false, "go")]);
        let ids = vec![Identifier {
            value: "src/a.rs".into(),
            kind: IdentifierKind::Path,
            seqs: vec![31],
        }];
        let records = level0_records(&pre, &ids, &submission(), "cv1-x", 3);
        let got: Vec<(&str, u64)> = records
            .items
            .iter()
            .map(|i| (i.id.as_str(), i.seq))
            .collect();
        assert_eq!(
            got,
            [
                ("i.31", 31),
                ("cv1-x.3.1", 31),
                ("cv1-x.3.2", 32),
                ("cv1-x.3.3", 33)
            ]
        );
        assert!(matches!(records.items[1].body, ItemBody::Decision { .. }));
        assert!(matches!(records.items[3].body, ItemBody::OpenItem { .. }));
        let targets: Vec<&str> = records
            .transitions
            .iter()
            .map(|t| t.target_id.as_str())
            .collect();
        // refs rewritten to assigned ids; the bundle-fold id kept as is.
        assert_eq!(targets, ["cv1-x.3.1", "cv1-x.3.3", "i.31"]);
        assert_eq!(records.transitions[1].cite_seq, 34);
        assert_eq!(records.transitions[1].new_status, NewStatus::Resolved);
        assert_eq!(records.identifiers, ids);
    }

    #[test]
    fn fallback_holds_prefill_only() {
        let pre = prefill(&[msg(4, Some(AuthorRole::Human), false, "go")]);
        let ids = vec![Identifier {
            value: "ht-12".into(),
            kind: IdentifierKind::BeadId,
            seqs: vec![4],
        }];
        let records = fallback_records(&pre, &ids);
        assert_eq!(records.items, pre);
        assert_eq!(records.identifiers, ids);
        assert!(records.transitions.is_empty());
        assert!(
            records
                .items
                .iter()
                .all(|i| matches!(i.body, ItemBody::UserInstruction { .. }))
        );
    }
    fn intent_messages() -> Vec<BundleMessage> {
        use crate::protocol::summary::UserIntent::{Query, Request, Rule};
        let mut messages = Vec::new();
        for role in [AuthorRole::Human, AuthorRole::Agent] {
            for intent in [None, Some(Query), Some(Request), Some(Rule)] {
                let seq = messages.len() as u64 + 1;
                let mut message = msg(
                    seq,
                    Some(role),
                    role == AuthorRole::Agent,
                    &"x".repeat(PREFILL_TEXT_MAX_BYTES + 1),
                );
                message.user_intent = intent;
                messages.push(message);
            }
        }
        messages.push(msg(
            9,
            Some(AuthorRole::Agent),
            false,
            "quoted: always test",
        ));
        let mut event = msg(10, Some(AuthorRole::Human), true, "joined");
        event.kind = MessageKind::Info;
        messages.push(event);
        messages
    }

    #[test]
    fn user_intent_prefill_stable_source_and_spill() {
        let messages = intent_messages();
        let items = prefill(&messages);
        assert_eq!(items.len(), 8);
        let sliced: Vec<_> = messages.chunks(3).flat_map(prefill).collect();
        assert_eq!(items, sliced);
        for (item, message) in items.iter().zip(&messages) {
            assert_eq!(item.id, format!("i.{}", message.sequence));
            assert_eq!(item.seq, message.sequence);
            assert_eq!(
                item.body,
                ItemBody::UserInstruction {
                    author_seat: message.author.clone(),
                    author_role: message.author_role,
                    relays_user: message.relays_user,
                    user_intent: message.user_intent,
                    text: None,
                    text_ref: Some(message.sequence),
                    message_id: Some(message.message.clone()),
                }
            );
        }
        let mut short = messages[1].clone();
        short.text = "question?".into();
        assert!(matches!(&prefill(&[short])[0].body,
            ItemBody::UserInstruction { text: Some(text), text_ref: None, .. } if text == "question?"));
    }

    #[test]
    fn user_intent_fallback_preserves_classification() {
        let pre = prefill(&intent_messages());
        let fallback = fallback_records(&pre, &[]);
        assert_eq!(fallback.items, pre);
        assert!(fallback.transitions.is_empty());
        assert_eq!(
            fallback
                .items
                .iter()
                .filter(|i| matches!(
                    i.body,
                    ItemBody::UserInstruction {
                        user_intent: Some(_),
                        ..
                    }
                ))
                .count(),
            6
        );
        let mut submitted = submission();
        submitted.new_decisions.clear();
        submitted.new_open_items.clear();
        submitted.transitions = vec![ProposedTransition {
            target: "i.4".into(),
            new_status: NewStatus::Superseded,
            cite_seq: 8,
            rule_change: Some(crate::protocol::summary::RuleChange::Replaced),
            quote: Some("exact submission-only evidence".into()),
        }];
        let stored = level0_records(&pre, &[], &submitted, "cv", 0);
        assert_eq!(stored.items, pre);
        assert_eq!(
            stored.transitions[0].rule_change,
            Some(crate::protocol::summary::RuleChange::Replaced)
        );
        assert!(
            !serde_json::to_string(&stored)
                .unwrap()
                .contains("submission-only")
        );
    }
}
