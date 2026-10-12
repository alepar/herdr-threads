use super::*;

// Kills: a normal, positive-capacity standalone fit failure retaining the same
// unoffered carry head forever, starving later canonical Lazy messages.
// This is admitted service/Watch input; NUL prefix is not an OS argv claim.
#[test]
fn presentation_normal_unrenderable_head_preserves_pending_and_serves_later_lazy() {
    normal_unrenderable_head_control(crate::protocol::commands::DeliveryMode::Ordinary);
}

#[test]
fn presentation_normal_unrenderable_lazy_head_preserves_pending_and_serves_later_lazy() {
    normal_unrenderable_head_control(crate::protocol::commands::DeliveryMode::Lazy);
}

fn normal_unrenderable_head_control(mode: crate::protocol::commands::DeliveryMode) {
    use crate::protocol::{
        commands::DeliveryMode,
        delivery::{DeliveryPullReplyV2, PresentationItemV2},
    };

    let window = OfferWindowFixture::new(true);
    let binding = binding_for_claim(&window.f, &window.recipient);
    assert!(
        window
            .f
            .state
            .lock()
            .get(&binding)
            .is_none_or(|entry| entry.offers.is_empty()),
        "normal discovery starts without prior logical offers"
    );
    let cursor = || {
        window
            .f
            .db
            .query_row::<i64, _, _>(
                "SELECT COALESCE((SELECT seq FROM binding_cursors WHERE binding_id=?1 AND thread_id='t'),0)",
                [binding.as_str()],
                |row| row.get(0),
            )
            .unwrap()
    };
    let original_cursor = cursor();
    let body = format!("{}é", "\0".repeat(65534));
    assert_eq!(body.len(), 65536);
    let blocked = window.send(0, mode, &body);
    let mut request = window.watch_request();
    request.marker_prefix = vec!["\0".repeat(512); 8];
    request.validate().unwrap();
    // Independent lower bound: 8192 NUL body bytes JSON-encode to 49152;
    // one quoted 4096-NUL prefix JSON-encodes to at least 20480 more.
    // Lazy has one continuation command; Ordinary has two. Both exceed
    // 65536 before any metadata, so an empty page cannot admit this head.

    let mut later: std::collections::BTreeMap<MessageId, String> =
        std::collections::BTreeMap::new();
    let mut seen = std::collections::BTreeSet::new();
    let mut quiet = false;
    for pull_number in 0..16 {
        let CommandResult::DeliveryPulledV2(reply) = window
            .f
            .service
            .handle(Command::DeliveryPullV2(request.clone()), peer(), &budget())
            .unwrap()
        else {
            panic!("admitted registered Watch pull");
        };
        match &reply {
            DeliveryPullReplyV2::Page(page) => {
                assert_capacity_watch_page_bound(page);
                assert!(!page.offered.contains(&blocked));
                assert!(!page.truncated.contains(&blocked));
                for item in &page.items {
                    if let PresentationItemV2::Message { id, lazy, content } = item {
                        assert_ne!(id, &blocked, "unrenderable head is never emitted");
                        let expected = later.get(id).expect("only later canonical messages");
                        assert!(*lazy);
                        assert!(!content.ack_required && !content.truncated);
                        assert_eq!(&content.body, expected);
                        assert!(seen.insert(id.clone()), "no early duplicate offers");
                        assert!(page.offered.contains(id));
                    }
                }
                // Confirms only actual nonmessage controls; no message ACK,
                // body display, invitation acceptance or synthetic offer.
                window.confirm_nonmessage_presentation(&reply);
                quiet = !page.more;
            }
            DeliveryPullReplyV2::Unchanged { .. } => quiet = true,
            other => panic!("admitted Watch input refused: {other:?}"),
        }
        let work = window
            .f
            .service
            .presentation
            .last_work
            .lock()
            .unwrap()
            .unwrap();
        assert!(work.output_passes <= 352, "sizing work: {work:?}");
        assert!(
            work.initial + work.context + work.revisit + work.ordinary + work.notices <= 433,
            "fresh discovery work: {work:?}"
        );
        assert!(work.ordinary <= 100);
        let carry_bytes: i64 = window
            .f
            .db
            .query_row(
                "SELECT length(CAST(ordinary_pending AS BLOB)) FROM invitation_presentation_consumers WHERE binding_id=?1 AND consumer='watch'",
                [binding.as_str()],
                |row| row.get(0),
            )
            .unwrap();
        assert!(carry_bytes <= 155731, "bounded private carry");
        let offers = window.offers();
        assert!(!offers.contains_key(&blocked), "no offer from a failed fit");
        assert!(offers.len() <= 256);
        assert!(
            offers
                .values()
                .all(|offer| offer.count <= MAX_OFFERS_PER_GENERATION)
        );
        assert_eq!(
            cursor(),
            original_cursor,
            "pulls do not claim this binding read"
        );
        let saved_body: String = window
            .f
            .db
            .query_row(
                "SELECT body FROM messages WHERE id=?1",
                [blocked.as_str()],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(saved_body, body, "full canonical body remains available");
        assert_normal_unrenderable_pending(&window, &blocked, mode);
        for id in later.keys() {
            let state: String = window
                .f
                .db
                .query_row(
                    "SELECT state FROM lazy_recipients WHERE message_id=?1 AND seat_id=?2",
                    params![id.as_str(), window.recipient.seat.as_str()],
                    |r| r.get(0),
                )
                .unwrap();
            assert_eq!(state, "pending", "emitting a Lazy body is not an ACK");
        }
        if pull_number == 0 {
            // Publish successors only after the first blocked capture. This
            // isolates carry starvation from a one-page batching failure.
            assert!(seen.is_empty() && offers.is_empty());
            for n in 1..=4 {
                let text = format!("later canonical Lazy body {n}");
                let id = window.send(n, DeliveryMode::Lazy, &text);
                later.insert(id, text);
            }
            quiet = false;
        } else if quiet && seen.len() == later.len() {
            break;
        }
    }
    assert_eq!(
        seen.len(),
        4,
        "normal unrenderable {mode:?} carry must not starve later Lazy messages within 16 pulls"
    );
    assert!(quiet, "positive-capacity traversal must become quiet");
    // V2 pulls explicitly retry normal capture even after a quiet pass. An
    // independently initiated pull need not take a V1 Unchanged fast path;
    // its bounded pass must finish quietly without emitting a new offer.
    assert_normal_watch_empty_pass_finishes(&window, request.clone(), 16);
    assert_normal_unrenderable_pending(&window, &blocked, mode);
    assert_eq!(cursor(), original_cursor);
    assert_eq!(window.offers().len(), 4);
    assert!(window.offers().values().all(|offer| offer.count == 1));

    // A new canonical publication independently admits another capture after
    // the completed/quiet pass. A smaller supported prefix now fits the blocked
    // body. This is not an endless immediate retry, ACK, owner replay, due offer
    // or manual cursor advancement standing in for delivery.
    let signal = window.send(5, DeliveryMode::Lazy, "independent next capture");
    request.marker_prefix.clear();
    let mut recovered = false;
    let mut signal_seen = false;
    for _ in 0..16 {
        let CommandResult::DeliveryPulledV2(reply) = window
            .f
            .service
            .handle(Command::DeliveryPullV2(request.clone()), peer(), &budget())
            .unwrap()
        else {
            panic!("independently admitted normal capture");
        };
        if let DeliveryPullReplyV2::Page(page) = &reply {
            assert_capacity_watch_page_bound(page);
            for item in &page.items {
                if let PresentationItemV2::Message { id, lazy, content } = item {
                    if id == &blocked {
                        assert!(!recovered, "one actual retry offer");
                        assert_eq!(*lazy, mode == DeliveryMode::Lazy);
                        // The source body is byte-chunked, so it deliberately
                        // carries no full-display ACK candidate. The canonical
                        // Ordinary obligation remains independently Pending.
                        assert!(
                            !content.ack_required,
                            "partial source body has no display ACK candidate"
                        );
                        assert_normal_unrenderable_pending(&window, &blocked, mode);
                        if mode == DeliveryMode::Ordinary {
                            let required: bool = window.f.db.query_row(
                                "SELECT pr.ack_required=1 FROM send_manifests sm JOIN prepared_recipients pr ON pr.preparation_id=sm.preparation_id WHERE sm.message_id=?1 AND pr.seat_id=?2",
                                params![blocked.as_str(), window.recipient.seat.as_str()],
                                |r| r.get(0),
                            ).unwrap();
                            assert!(required, "canonical Ordinary ACK obligation is unchanged");
                        }
                        assert!(content.truncated);
                        assert_eq!(content.body_len, 65536);
                        assert!(content.body.starts_with(&"\0".repeat(8192)));
                        assert!(page.truncated.contains(&blocked));
                        recovered = true;
                    } else if id == &signal {
                        assert!(*lazy && !content.truncated && !content.ack_required);
                        assert_eq!(content.body, "independent next capture");
                        signal_seen = true;
                    } else {
                        panic!("already offered successors must not reappear early");
                    }
                }
            }
            window.confirm_nonmessage_presentation(&reply);
        } else {
            assert!(matches!(reply, DeliveryPullReplyV2::Unchanged { .. }));
        }
        assert!(
            window
                .f
                .service
                .presentation
                .last_work
                .lock()
                .unwrap()
                .unwrap()
                .output_passes
                <= 352
        );
        assert_eq!(
            cursor(),
            original_cursor,
            "retry emission claims no full display"
        );
        if recovered && signal_seen {
            break;
        }
    }
    assert!(
        recovered && signal_seen,
        "canonical pending head is revisited when a newly admitted capture fits"
    );
    let offer = window.offers()[&blocked];
    assert_eq!(
        offer.count, 1,
        "failed original fit was never a logical offer"
    );
    assert!(offer.truncated);
    let saved_body: String = window
        .f
        .db
        .query_row(
            "SELECT body FROM messages WHERE id=?1",
            [blocked.as_str()],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(saved_body, body);
    assert_normal_unrenderable_pending(&window, &blocked, mode);
}

// Ordinary sends can remain manifest-first without a materialized receipt row.
// Read canonical effective authority; never create a receipt to satisfy a test.
fn assert_normal_unrenderable_pending(
    window: &OfferWindowFixture,
    message: &MessageId,
    mode: crate::protocol::commands::DeliveryMode,
) {
    if mode == crate::protocol::commands::DeliveryMode::Lazy {
        let state: String = window
            .f
            .db
            .query_row(
                "SELECT state FROM lazy_recipients WHERE message_id=?1 AND seat_id=?2",
                params![message.as_str(), window.recipient.seat.as_str()],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(state, "pending", "rendering cannot finish Lazy display");
    } else {
        let receipt = crate::store::effective::effective_receipt(
            &window.f.db,
            message.as_str(),
            window.recipient.seat.as_str(),
        )
        .unwrap()
        .expect("canonical ordinary recipient obligation exists");
        assert_eq!(
            receipt.state,
            crate::store::effective::EffectiveReceiptState::Pending,
            "rendering cannot ACK the canonical ordinary obligation"
        );
    }
}

// Kills treating a nonempty-page fit failure as standalone-unrenderable: the
// withheld candidate fits alone and must retain its next normal opportunity.
#[test]
fn presentation_normal_mixed_page_retains_alone_fitting_lazy_successor() {
    use crate::protocol::{
        commands::DeliveryMode,
        delivery::{DeliveryPullReplyV2, PresentationItemV2},
    };
    let window = OfferWindowFixture::new(true);
    let binding = binding_for_claim(&window.f, &window.recipient);
    let cursor = || {
        window.f.db.query_row::<i64, _, _>(
            "SELECT COALESCE((SELECT seq FROM binding_cursors WHERE binding_id=?1 AND thread_id='t'),0)",
            [binding.as_str()],
            |r| r.get(0),
        ).unwrap()
    };
    let original_cursor = cursor();
    // Source JSON body payload is 8193 ASCII + 49152 escaped control bytes,
    // below its 65536 quantum. Watch adds the admitted prefix to the first
    // truncated body's continuation: at least8192+20480+49152=77824 combined.
    // Both actual Watch singletons fit; only their combined presentation fails.
    let first_body = "a".repeat(8193);
    let second_body = "\0".repeat(8192);
    let mut request = window.watch_request();
    request.marker_prefix = vec!["\0".repeat(512); 8];
    request.validate().unwrap();
    let pull = || {
        let CommandResult::DeliveryPulledV2(reply) = window
            .f
            .service
            .handle(Command::DeliveryPullV2(request.clone()), peer(), &budget())
            .unwrap()
        else {
            panic!("same registered Watch request");
        };
        window.confirm_nonmessage_presentation(&reply);
        reply
    };
    let first = window.send(0, DeliveryMode::Lazy, &first_body);
    let second = window.send(1, DeliveryMode::Lazy, &second_body);
    let DeliveryPullReplyV2::Page(page) = pull() else {
        panic!("actual registered Watch first page");
    };
    assert_capacity_watch_page_bound(&page);
    let first_work = window
        .f
        .service
        .presentation
        .last_work
        .lock()
        .unwrap()
        .unwrap();
    assert!(first_work.output_passes <= 352);
    assert!(
        first_work.initial
            + first_work.context
            + first_work.revisit
            + first_work.ordinary
            + first_work.notices
            <= 433
    );
    assert!(page.offered.is_empty());
    assert_eq!(page.truncated, vec![first.clone()]);
    let first_content = page
        .items
        .iter()
        .find_map(|item| match item {
            PresentationItemV2::Message { id, lazy, content } if id == &first => {
                assert!(*lazy && content.truncated && !content.ack_required);
                Some(content)
            }
            _ => None,
        })
        .expect("first truncated body actually emitted");
    assert_eq!(first_content.body_len, 8193);
    assert!(first_content.body.starts_with(&"a".repeat(8192)));
    assert!(
        page.more,
        "withheld normal candidate has another opportunity"
    );
    let carry: String = window.f.db.query_row(
        "SELECT ordinary_pending FROM invitation_presentation_consumers WHERE binding_id=?1 AND consumer='watch'",
        [binding.as_str()],
        |r| r.get(0),
    ).unwrap();
    assert!(carry.len() <= 155731);
    let retained: serde_json::Value = serde_json::from_str(&carry).unwrap();
    assert!(
        retained["items"]
            .as_array()
            .unwrap()
            .iter()
            .any(|item| item["content"]["id"] == second.as_str()),
        "nonempty-page failure retains the canonical successor in carry"
    );
    assert_eq!(window.offers().len(), 1);
    assert!(!window.offers().contains_key(&second));
    assert_normal_unrenderable_pending(&window, &first, DeliveryMode::Lazy);
    assert_normal_unrenderable_pending(&window, &second, DeliveryMode::Lazy);
    assert_eq!(cursor(), original_cursor);

    let mut shown = false;
    let mut quiet = false;
    for _ in 0..8 {
        let reply = pull();
        match reply {
            DeliveryPullReplyV2::Page(page) => {
                assert_capacity_watch_page_bound(&page);
                assert!(page.truncated.is_empty());
                for item in &page.items {
                    if let PresentationItemV2::Message { id, lazy, content } = item {
                        assert_eq!(id, &second, "first offer does not repeat early");
                        assert!(!shown, "successor is offered once");
                        assert!(*lazy && !content.truncated && !content.ack_required);
                        assert_eq!(content.body, second_body);
                        assert_eq!(content.body_len, 8192);
                        assert!(page.offered.contains(&second));
                        shown = true;
                    }
                }
                quiet = !page.more;
            }
            DeliveryPullReplyV2::Unchanged { .. } => quiet = true,
            other => panic!("same registered Watch lease refused: {other:?}"),
        }
        let work = window
            .f
            .service
            .presentation
            .last_work
            .lock()
            .unwrap()
            .unwrap();
        assert!(work.output_passes <= 352);
        assert!(work.initial + work.context + work.revisit + work.ordinary + work.notices <= 433);
        assert!(window.offers().len() <= 256);
        assert!(
            window
                .offers()
                .values()
                .all(|offer| offer.count <= MAX_OFFERS_PER_GENERATION)
        );
        assert_normal_unrenderable_pending(&window, &first, DeliveryMode::Lazy);
        assert_normal_unrenderable_pending(&window, &second, DeliveryMode::Lazy);
        assert_eq!(
            cursor(),
            original_cursor,
            "body emission is not full display"
        );
        if shown && quiet {
            break;
        }
    }
    assert!(
        shown && quiet,
        "alone-fitting carry must emit and finish its pass"
    );
    assert_normal_watch_empty_pass_finishes(&window, request, 8);
    assert_normal_unrenderable_pending(&window, &first, DeliveryMode::Lazy);
    assert_normal_unrenderable_pending(&window, &second, DeliveryMode::Lazy);
    assert_eq!(cursor(), original_cursor);
    let offers = window.offers();
    assert_eq!(offers.len(), 2);
    assert_eq!(offers[&first].count, 1);
    assert_eq!(offers[&second].count, 1);
    assert!(offers[&first].truncated && !offers[&second].truncated);
}

// Every externally requested V2 pull can initiate a new finite normal capture.
// Quiet is Page.more=false (or Unchanged), rather than a specific reply variant.
fn assert_normal_watch_empty_pass_finishes(
    window: &OfferWindowFixture,
    request: crate::protocol::delivery::DeliveryPullV2,
    max_pulls: usize,
) {
    use crate::protocol::delivery::{DeliveryPullReplyV2, PresentationItemV2};
    let offers = window.offers();
    for _ in 0..max_pulls {
        let CommandResult::DeliveryPulledV2(reply) = window
            .f
            .service
            .handle(Command::DeliveryPullV2(request.clone()), peer(), &budget())
            .unwrap()
        else {
            panic!("independent finite normal pass");
        };
        let quiet = match &reply {
            DeliveryPullReplyV2::Page(page) => {
                assert_capacity_watch_page_bound(page);
                assert!(page.offered.is_empty() && page.truncated.is_empty());
                assert!(
                    page.items
                        .iter()
                        .all(|item| !matches!(item, PresentationItemV2::Message { .. }))
                );
                window.confirm_nonmessage_presentation(&reply);
                !page.more
            }
            DeliveryPullReplyV2::Unchanged { .. } => true,
            other => panic!("same admitted Watch pull refused: {other:?}"),
        };
        let work = window
            .f
            .service
            .presentation
            .last_work
            .lock()
            .unwrap()
            .unwrap();
        assert!(work.output_passes <= 352);
        assert!(work.initial + work.context + work.revisit + work.ordinary + work.notices <= 433);
        assert_eq!(
            window.offers(),
            offers,
            "independent quiet pass creates no offer or count"
        );
        let carry_bytes: i64 = window.f.db.query_row(
            "SELECT length(CAST(ordinary_pending AS BLOB)) FROM invitation_presentation_consumers WHERE binding_id=?1 AND consumer='watch'",
            [binding_for_claim(&window.f, &window.recipient).as_str()],
            |r| r.get(0),
        ).unwrap();
        assert!(carry_bytes <= 155731);
        if quiet {
            return;
        }
    }
    panic!("independently requested normal capture must finish within its original bound");
}
