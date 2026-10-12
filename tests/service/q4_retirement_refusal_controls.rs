//! Refusal controls for the new modern private Lazy scope-loss retirement path.
//! This child reuses q4_body_controls' canonical producers and actual captured prefix.
use super::*;
use crate::{
    ports::ReadContext,
    protocol::{
        commands::{CreateThread, InboxQuery, Invite, Reject},
        delivery::{DeliveryPresentationConfirmV2, DeliveryPresentationConfirmationV2},
        pagination::{InboxBatchV2CursorState, InboxBatchV2Source, PageRequest},
        results::ErrorCode,
    },
};

struct CapturedScopeLoss {
    f: BodyFixture,
    old: MessageId,
    fresh: MessageId,
    cursor: String,
}

impl CapturedScopeLoss {
    fn new() -> Self {
        let f = BodyFixture::new();
        let old = f.send("refusal-old-capture", &long_utf8_body());
        let initial = f.private_pull(&f.b);
        let DeliveryPullReplyV2::Page(first) = initial.reply else {
            panic!("SETUP: actual old-episode private prefix");
        };
        assert!(first.items.iter().any(|item| matches!(item,
            PresentationItemV2::Message { id, lazy: true, content }
                if id == &old && content.truncated && !content.ack_required)));
        let cursor: String = f.db.query_row(
            "SELECT ordinary_cursor FROM invitation_presentation_consumers WHERE binding_id=?1 AND consumer='watch'",
            [f.binding(&f.b).as_str()], |r| r.get(0),
        ).unwrap();
        let capture =
            InboxBatchV2CursorState::decode_for(&cursor, &f.instance.to_string(), &f.b.seat)
                .unwrap();
        assert_eq!(capture.source, InboxBatchV2Source::Lazy);
        let body = capture.body.expect("SETUP: real captured Lazy tail");
        assert_eq!(body.message, old);
        assert!(body.offset > 0 && long_utf8_body().is_char_boundary(body.offset as usize));
        // Physically render/confirm the actual prefix, without any display completion.
        let mut encoded = Vec::new();
        for line in crate::delivery::presentation::presentation_watch_lines(&first.items) {
            serde_json::to_writer(&mut encoded, &line).unwrap();
            encoded.push(b'\n');
        }
        encoded.flush().unwrap();
        assert_eq!(encoded.len(), first.output_bytes as usize);
        if let Some(token) = &first.presentation_token {
            let confirmation = f
                .store
                .presentation_confirm(
                    &DeliveryPresentationConfirmV2 {
                        claim: f.b.clone(),
                        consumer: PresentationConsumer::Watch,
                        presentation_lease: Some("0123456789abcdef0123456789abcdef".into()),
                        token: token.clone(),
                        emitted_controls: first.emitted_controls.clone(),
                    },
                    &budget(),
                )
                .unwrap();
            assert_eq!(
                confirmation.status,
                DeliveryPresentationConfirmationV2::Confirmed
            );
        }
        let invitation: InvitationId = f.db.query_row(
            "SELECT i.id FROM memberships m JOIN invitations i ON i.thread_id=m.thread_id AND i.seat_id=m.seat_id AND i.episode=m.episode WHERE m.thread_id=?1 AND m.seat_id=?2",
            params![f.thread.as_str(), f.b.seat.as_str()], |r| r.get::<_,String>(0),
        ).map(InvitationId::new).unwrap();
        let rejected = f
            .service
            .handle(
                Command::Reject(Reject {
                    claim: f.b.clone(),
                    thread: f.thread.clone(),
                    invitation: invitation.clone(),
                    reason: "close the captured stream before checking refusal guards".into(),
                    operation: OperationId::new("refusal-reject"),
                }),
                peer(),
                &budget(),
            )
            .unwrap();
        assert!(matches!(rejected, CommandResult::Rejected(_)));
        let reinvited = f
            .service
            .handle(
                Command::Invite(Invite {
                    claim: f.sender.clone(),
                    thread: f.thread.clone(),
                    seat: f.b.seat.clone(),
                    confirmation: Some(InvitationConfirmation::Lazy),
                    deadline_millis: None,
                    operation: OperationId::new("refusal-reinvite"),
                }),
                peer(),
                &budget(),
            )
            .unwrap();
        let CommandResult::Invitation(current) = reinvited else {
            panic!("SETUP: real reinvitation")
        };
        assert_ne!(current, invitation);
        let fresh = f.send(
            "refusal-fresh-body",
            "fresh unread body remains available after refusing old capture",
        );
        let facts: (i64, i64, i64, i64) = f.db.query_row(
            "SELECT (SELECT decision_seq FROM messages WHERE id=?1),(SELECT created_decision_seq FROM invitations WHERE id=?2),(SELECT decision_seq FROM messages WHERE id=?3),(SELECT legacy_pending FROM lazy_recipients WHERE message_id=?1 AND seat_id=?4)",
            params![old.as_str(),current.as_str(),fresh.as_str(),f.b.seat.as_str()],
            |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?)),
        ).unwrap();
        assert!(facts.0 < facts.1 && facts.1 < facts.2);
        assert_eq!(facts.3, 0, "SETUP: actual modern recipient");
        assert_eq!((f.cursor(&f.a), f.cursor(&f.b)), (None, None));
        let visible: bool =
            f.db.query_row(
                crate::store::queries::LAZY_CONTINUATION_SCOPE_SQL,
                params![
                    f.b.seat.as_str(),
                    old.as_str(),
                    f.binding(&f.b).as_str(),
                    f.instance.to_string()
                ],
                |r| r.get(0),
            )
            .unwrap();
        assert!(
            !visible,
            "SETUP: canonical Reject/Invite loses the old cursorless continuation scope"
        );
        f.assert_pending();
        Self {
            f,
            old,
            fresh,
            cursor,
        }
    }

    fn private_result(&self) -> Result<PresentationOutput, ApiError> {
        let f = &self.f;
        let frozen_render = f
            .store
            .presentation_render_selection(&f.b, PresentationConsumer::Watch, &budget())?
            .map(|identity| FrozenPresentationRender {
                identity,
                codec: None,
            });
        f.store.presentation_pull(
            PresentationInput {
                binding_id: f.binding(&f.b),
                owner_replay: None,
                ordinary_due_snapshot: None,
                frozen_render,
                render_profile: PresentationRenderProfile::WatchJsonLinesV1,
                pull: DeliveryPullV2 {
                    claim: f.b.clone(),
                    consumer: PresentationConsumer::Watch,
                    render: PresentationRenderRequest::WatchJsonLinesV1,
                    presentation_lease: Some("0123456789abcdef0123456789abcdef".into()),
                    marker_prefix: vec![],
                },
                attention_version: 0,
                ordinary_version: 0,
                with_attention: false,
                retry_messages: true,
                blocked_messages: vec![],
                known_messages: vec![],
                new_message_slots: 256,
            },
            &budget(),
        )
    }
}

fn rows(f: &BodyFixture, sql: &str) -> Vec<Vec<Value>> {
    let mut statement = f.db.prepare(sql).unwrap();
    let width = statement.column_count();
    statement
        .query_map([], |row| {
            (0..width)
                .map(|column| row.get(column))
                .collect::<rusqlite::Result<Vec<Value>>>()
        })
        .unwrap()
        .collect::<rusqlite::Result<Vec<_>>>()
        .unwrap()
}

fn bookend(f: &BodyFixture) -> Vec<Vec<Vec<Value>>> {
    [
        "SELECT * FROM invitation_presentation_consumers ORDER BY binding_id,consumer",
        "SELECT * FROM invitation_presentation_items ORDER BY binding_id,consumer,invitation_id",
        "SELECT * FROM presentation_warning_proofs ORDER BY binding_id,consumer",
        "SELECT * FROM presentation_warning_fence ORDER BY singleton",
        "SELECT * FROM binding_cursors ORDER BY binding_id,thread_id",
        "SELECT * FROM occupant_bindings ORDER BY binding_id",
        "SELECT * FROM lazy_recipients ORDER BY ordinal",
        "SELECT * FROM receipts ORDER BY message_id,seat_id",
        "SELECT * FROM receipt_state ORDER BY message_id,seat_id",
        "SELECT * FROM invitations ORDER BY id",
        "SELECT * FROM memberships ORDER BY thread_id,seat_id",
        "SELECT * FROM membership_intervals ORDER BY thread_id,seat_id,episode",
        "SELECT * FROM wake_work ORDER BY seat_id",
        "SELECT * FROM warning_offer ORDER BY binding_id",
        "SELECT * FROM digest_notice_offer ORDER BY binding_id",
        "SELECT * FROM messages ORDER BY id",
        "SELECT * FROM send_manifests ORDER BY preparation_id",
        "SELECT * FROM host_instances ORDER BY id",
    ]
    .into_iter()
    .map(|sql| rows(f, sql))
    .collect()
}

fn unchanged_after_error(c: &CapturedScopeLoss, before: &[Vec<Vec<Value>>]) {
    assert_eq!(
        bookend(&c.f),
        before,
        "refusal rolls back every private progress/proof field and leaves canonical display, reader, addressing, receipt and episode state unchanged"
    );
    assert_eq!((c.f.cursor(&c.f.a), c.f.cursor(&c.f.b)), (None, None));
    c.f.assert_pending();
}

fn partial(key: &str, text: &str) -> String {
    serde_json::to_string(&std::collections::BTreeMap::from([(
        key.to_owned(),
        text.to_owned(),
    )]))
    .unwrap()
}

// These malformed records cannot be produced by canonical writers. Fault only the isolated
// private accumulator/cursor after a real capture and real audience transition.
#[test]
fn modern_lazy_scope_loss_cannot_hide_corrupt_accumulator_or_utf8_offset() {
    for fault in [
        "invalid_accumulator_id",
        "wrong_accumulator_id",
        "wrong_accumulator_offset",
        "wrong_skip_id",
        "utf8_cursor_offset",
    ] {
        let c = CapturedScopeLoss::new();
        let f = &c.f;
        let binding = f.binding(&f.b);
        match fault {
            "invalid_accumulator_id" | "wrong_accumulator_id" | "wrong_accumulator_offset" => {
                let key = match fault {
                    "invalid_accumulator_id" => "bad id",
                    "wrong_accumulator_id" => c.fresh.as_str(),
                    _ => c.old.as_str(),
                };
                f.db.execute("UPDATE invitation_presentation_consumers SET ordinary_skip_body=NULL,ordinary_partial=?1 WHERE binding_id=?2 AND consumer='watch'", params![partial(key,"x"),binding.as_str()]).unwrap();
            }
            "wrong_skip_id" => {
                f.db.execute("UPDATE invitation_presentation_consumers SET ordinary_skip_body=?1 WHERE binding_id=?2 AND consumer='watch'", params![c.fresh.as_str(),binding.as_str()]).unwrap();
            }
            "utf8_cursor_offset" => {
                let mut state = InboxBatchV2CursorState::decode_for(
                    &c.cursor,
                    &f.instance.to_string(),
                    &f.b.seat,
                )
                .unwrap();
                let body = state.body.as_mut().unwrap();
                body.offset += 1;
                while long_utf8_body().is_char_boundary(body.offset as usize) {
                    body.offset += 1;
                }
                assert!(body.offset < body.body_len);
                let malformed = state.encode(&f.instance.to_string()).unwrap();
                f.db.execute("UPDATE invitation_presentation_consumers SET ordinary_cursor=?1 WHERE binding_id=?2 AND consumer='watch'", params![malformed,binding.as_str()]).unwrap();
            }
            _ => unreachable!(),
        }
        let before = bookend(f);
        let error = c
            .private_result()
            .expect_err("scope retirement must not bypass malformed captured state");
        let (code, detail) = match fault {
            "invalid_accumulator_id" => {
                (ErrorCode::StoreCorrupt, "invalid ordinary partial record")
            }
            "wrong_skip_id" => (
                ErrorCode::StoreCorrupt,
                "invalid ordinary skipped body cursor",
            ),
            "utf8_cursor_offset" => (
                ErrorCode::InvalidCursor,
                "inbox body changed or invalid boundary",
            ),
            _ => (
                ErrorCode::StoreCorrupt,
                "invalid ordinary continued body accumulator",
            ),
        };
        assert_eq!(error.code, code, "{fault}: {error:?}");
        assert!(error.detail.contains(detail), "{fault}: {error:?}");
        unchanged_after_error(&c, &before);
    }
}

// Public writers forbid these identity changes. Temporarily remove only the named isolated
// fixture guard, restore its exact definition before invoking any production reader.
fn fault_with_restored_guard(
    f: &BodyFixture,
    guard: &str,
    mutate: impl FnOnce() -> rusqlite::Result<usize>,
) {
    let definition: String =
        f.db.query_row(
            "SELECT sql FROM sqlite_schema WHERE type='trigger' AND name=?1",
            [guard],
            |r| r.get(0),
        )
        .unwrap();
    f.db.execute_batch(&format!("DROP TRIGGER {guard};"))
        .unwrap();
    let mutation = mutate();
    f.db.execute_batch(&definition).unwrap();
    assert_eq!(mutation.unwrap(), 1);
}

#[test]
fn modern_lazy_retirement_requires_intact_captured_publication_and_recipient_authority() {
    for fault in [
        "missing_manifest",
        "manifest_decision",
        "foreign_publication_instance",
        "missing_recipient",
        "recipient_thread",
    ] {
        let c = CapturedScopeLoss::new();
        let f = &c.f;
        match fault {
            "missing_manifest" => {
                assert_eq!(
                    f.db.execute(
                        "DELETE FROM send_manifests WHERE message_id=?1",
                        [c.old.as_str()]
                    )
                    .unwrap(),
                    1
                );
            }
            "manifest_decision" => fault_with_restored_guard(f, "send_manifests_immutable", || {
                f.db.execute(
                    "UPDATE send_manifests SET decision_seq=decision_seq+1 WHERE message_id=?1",
                    [c.old.as_str()],
                )
            }),
            "foreign_publication_instance" => {
                let foreign = uuid::Uuid::new_v4().to_string();
                // Explicit corruption fixture: both retained identities agree with each other,
                // but not the instance in the immutable captured continuation.
                f.db.execute("INSERT INTO host_instances(id,created_at,host_boot,host_epoch,decision_seq) VALUES(?1,0,'foreign',1,0)",[&foreign]).unwrap();
                fault_with_restored_guard(f, "messages_immutable", || {
                    f.db.execute(
                        "UPDATE messages SET instance_id=?1 WHERE id=?2",
                        params![foreign, c.old.as_str()],
                    )
                });
                fault_with_restored_guard(f, "send_manifests_immutable", || {
                    f.db.execute(
                        "UPDATE send_manifests SET instance_id=?1 WHERE message_id=?2",
                        params![foreign, c.old.as_str()],
                    )
                });
            }
            "missing_recipient" => {
                fault_with_restored_guard(f, "lazy_recipient_published_retained", || {
                    f.db.execute(
                        "DELETE FROM lazy_recipients WHERE message_id=?1 AND seat_id=?2",
                        params![c.old.as_str(), f.b.seat.as_str()],
                    )
                })
            }
            "recipient_thread" => {
                let other = f
                    .service
                    .handle(
                        Command::CreateThread(CreateThread {
                            name: None,
                            topic: "unrelated fault target".into(),
                            goal: "identity mismatch only".into(),
                            operation: OperationId::new("refusal-other-thread"),
                            claim: f.sender.clone(),
                        }),
                        peer(),
                        &budget(),
                    )
                    .unwrap();
                let CommandResult::ThreadCreated(other) = other else {
                    panic!("canonical unrelated thread")
                };
                fault_with_restored_guard(f, "lazy_recipient_identity_immutable", || {
                    f.db.execute("UPDATE lazy_recipients SET thread_id=?1 WHERE message_id=?2 AND seat_id=?3",params![other.as_str(),c.old.as_str(),f.b.seat.as_str()])
                });
            }
            _ => unreachable!(),
        }
        let before = bookend(f);
        let error = c.private_result().expect_err("missing or inconsistent publication/address authority is refusal, never private retirement");
        assert_eq!(error.code, ErrorCode::InvalidCursor, "{fault}: {error:?}");
        assert!(
            error
                .detail
                .contains("continued body is not captured addressed content"),
            "{fault}: {error:?}"
        );
        unchanged_after_error(&c, &before);
    }
}

#[test]
fn public_lazy_continuation_refuses_real_old_episode_audience_loss() {
    let c = CapturedScopeLoss::new();
    let f = &c.f;
    let before = bookend(f);
    let error = f
        .store
        .query(
            &Command::InboxBatchV2(InboxQuery {
                thread: None,
                seat: Some(f.b.seat.clone()),
                claim: Some(f.b.clone()),
                page: PageRequest {
                    cursor: Some(c.cursor.clone()),
                    limit: 32,
                    max_bytes: 65_536,
                },
            }),
            &ReadContext {
                instance: f.instance.to_string(),
                output: Default::default(),
                operation_scope: None,
            },
            &budget(),
        )
        .expect_err("public continuation cannot use private modern scope retirement");
    assert_eq!(error.code, ErrorCode::InvalidCursor);
    assert!(
        error
            .detail
            .contains("continued body is not captured unread content"),
        "{error:?}"
    );
    unchanged_after_error(&c, &before);
}

#[test]
fn legacy_lazy_tail_keeps_strict_refusal_when_modern_reader_scope_is_lost() {
    let c = CapturedScopeLoss::new();
    let f = &c.f;
    // Explicit migration-compatibility fixture: modern producers cannot create legacy_pending1.
    // All publication/capture/addressing authority remains real and unchanged.
    f.db.execute(
        "UPDATE lazy_recipients SET legacy_pending=1 WHERE message_id=?1 AND seat_id=?2",
        params![c.old.as_str(), f.b.seat.as_str()],
    )
    .unwrap();
    let before = bookend(f);
    let error = c
        .private_result()
        .expect_err("legacy tail cannot retire through the modern-only scope exception");
    assert_eq!(error.code, ErrorCode::InvalidCursor);
    assert!(
        error
            .detail
            .contains("continued body is not captured unread content"),
        "{error:?}"
    );
    unchanged_after_error(&c, &before);
}

#[test]
fn modern_lazy_scope_retirement_rolls_back_when_final_private_save_fails() {
    let c = CapturedScopeLoss::new();
    let f = &c.f;
    // Target the exact captured consumer; require cursor change so this fault follows the
    // tentative retirement, not a passive/context-only write before the continued scan.
    let binding = f.binding(&f.b).as_str().replace('\'', "''");
    f.db.execute_batch(&format!("CREATE TRIGGER q4_refuse_retirement_save BEFORE UPDATE ON invitation_presentation_consumers WHEN NEW.binding_id='{binding}' AND NEW.consumer='watch' AND NEW.ordinary_cursor IS NOT OLD.ordinary_cursor BEGIN SELECT RAISE(ABORT,'q4 final modern retirement save'); END;")).unwrap();
    let before = bookend(f);
    let error = c.private_result().expect_err(
        "late failure rolls back modern Lazy retirement and fresh continuation progress",
    );
    assert!(
        error.detail.contains("q4 final modern retirement save"),
        "{error:?}"
    );
    unchanged_after_error(&c, &before);
}
