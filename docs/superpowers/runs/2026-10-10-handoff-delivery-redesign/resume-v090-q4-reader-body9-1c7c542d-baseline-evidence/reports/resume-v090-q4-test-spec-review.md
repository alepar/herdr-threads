# Q4 nine-control spec review

**Scoped PASS for the nine authored controls; incomplete as a full Q4 verification gate.** Their desired behavior matches approved episode design SHA256 `472159f833dfb65ba4b5096b69fc87fe461ec82c63864dc794e9f2eef7d9f1ca`. No load-bearing wrong expectation or manufactured positive state was found in this source review. The remaining controls below are required before claiming the whole design verified.

Reviewed the actual frozen files at parent-reported clean candidate `1c7c542d`, with production unchanged from `3dd448ef`:

- `tests/store/direct_lazy_readers.rs`: 587 lines; SHA256 `d64222059a216cbcf9a120cc60ec6168069cd411a98b950fd9c46ad01a464567`.
- `tests/service/q4_body_controls.rs`: 630 lines; SHA256 `2f6fa897bb340aa794ad2ad234b5551d5a562bb066515e40c7dc550c279b60d1`.

Read both WS proposal reports. The parent reports that the original baseline failed compilation for a missing LocalService trait import and that `1c7c542d` corrects it. This review neither executed nor independently verified compilation/runtime. Parent's nine-test baseline is pending. No Cargo/runtime/SQL execution, test/source/Git mutation, Herdr or nested agent was used; only this report was written. This is a mandatory scoped spec review, not a design roast; counters unchanged.

## Causal setup and authority

The six store controls use the existing host/seat fixture, then actual DomainService lifecycle CheckIn and returned canonical claims. Their pair, Lazy invitations, direct sends, reject, reinvite, acceptance and display mutations all use canonical commands. The recipient publication witness joins the actual modern Lazy row, manifest and message and requires `legacy_pending=0`. Independent sibling sessions and process fingerprints prevent accidental supersession. No test writes a cursor, membership episode, Lazy progress flag, manifest or consumer to enable the desired result.

The body module seeds only host/seat/observation foundations, enrolls actual bindings, and creates the pair/messages canonically. Its LocalClient delegates results to DomainService. The real CLI parser/run_cooperative path executes with an exact enrolled context and records successful stdout flush before forwarding AdvanceCursors. The short-write/BrokenPipe and flush-error writers exercise actual CLI output failure. Consumer snapshots are created by real presentation pulls and compare every stored column; they are not fabricated private state.

The private pull helper deliberately supplies internal Watch presentation input, a test lease string, empty logical-offer lists and retry=true directly to StorePort. This is an appropriate narrow source/continuation test seam, as disclosed in the proposal. It does not prove public Watch admission, lease ownership, ledger replay throttling or frame-triggered drain behavior. The unchanged actual AGY diagnostic and separate channel controls retain those responsibilities.

## What the assertions establish if their setup reaches the oracle

| Control | Spec assessment and expected baseline interpretation |
| --- | --- |
| Initial published direct Lazy | Exact public own-binding body before acceptance, genuine page exhaustion, absent cursor and unchanged receipt/interval/wake facts. A missing body after successful publication is a meaningful reader RED. |
| Two initial bindings | Explicit completion moves only A; B retains its modern unread body despite shared displayed bookkeeping. Early failure at either initial read proves reader exclusion only; later sibling assertions remain unexecuted until that is fixed. |
| Initial acceptance and display gap | Real accept-before-display and later-body-only AdvanceCursors cannot erase the earlier body. It avoids hardcoding a numeric zero seed and allows the approved invitation-event floor. An existing cursor, if present, must remain exact. |
| Post-join new binding | C inherits the actual seat maximum after A completion while B remains independently unread. This preserves Main's documented rule rather than demanding a new universal no-backlog model. On old production, earlier failure from acceptance erasing B is a meaningful acceptance defect. |
| Rejection negative | Real rejection clears voluntary invitation; fresh cursorless reads and late AdvanceCursors cannot revive old mail. Public body inspection remains readable. This deliberately preserves existing negative behavior and need not fail on baseline. It does not assert that historical CompleteInboxDelivery is refused. |
| Episode two | Genuine Reject/Invite supplies old-publication < invitation-creation < fresh-publication order. Fresh B/C exclude old history; established A retains it. Both CompleteInboxDelivery and AdvanceCursors must pass B/C's excluded old history without settling A. Later acceptance preserves all existing progress. This meaningfully rejects both initial-only eligibility and unconditional historical enrollment. |
| Full flushed UTF-8 body | Follows actual server continuation argv and verifies exact assembled65536-byte content, offsets and UTF-8 boundaries. Prefixes produce no cursor/private changes; only final successful output dispatches one exact-claim AdvanceCursors. Only A's two counters change, preserving every other private field and B. Cursor nonmovement after a recorded valid final mutation is a meaningful AdvanceCursors RED. |
| Read-only/failed output | JSON, machine, bounded prefix, explicit offset, write failure and flush failure produce no mutation, cursor or private change. This is a negative control expected to retain existing semantics, including the documented explicit-offset-continuation limitation. |
| Captured private Lazy tail | Requires an actual returned truncated long body and producer-retained Source::Lazy cursor naming that body at a valid offset; it never edits the cursor. A's real full CLI display cannot alter B. B's next private pull consumes the captured tail, avoids a second physical offer of that body, and reaches the following exact body without display authority. This matches existing skip-body continuation semantics. |

Store helper calls to Message followed by an explicit display mutation are cooperative canonical store controls, not proof of actual terminal flush. That proof is separately supplied by the real CLI tests. The negative rejection test correctly distinguishes AdvanceCursors' new admission from the historically broader exact-ID CompleteInboxDelivery path.

## Runtime classification and coverage limits

Do not label every failing selector an independent causal RED. Compilation, canonical admission/enrollment failure, body page budget failure, and failure to create the required captured cursor are distinct setup outcomes. In particular, on unchanged production the private-tail test can fail because the first page contains no Lazy body; that supports initial eligibility but does not yet isolate the separate continued-Lazy scope gate. After main eligibility is implemented, rerunning the unchanged test can expose the independent continuation defect. The positive full-body CLI test does not depend on automatic inbox eligibility to reach its final mutation oracle, making it useful independent evidence.

The six public controls cover only native invitation creation/reinvitation. Existing refusal tests for third seats/service ownership do not establish ordinary service or operator endpoint invitation-floor behavior. The current episode-two test completes both cursorless B and C before acceptance; it therefore does **not** yet prove episode-two accept-before-display seeding for a missing cursor. Its name's “before or after accept” should not be used to claim that missing scenario.

Still required by the approved design, outside these nine controls:

- The separately owned accept-only quiet Hook/Watch consumer control, including unchanged-cursor and missing-cursor bindings, mixed ordinary/Lazy content, no intervening publication, exact per-consumer increments, payload preservation, disconnected/ended scopes, replay and whole-transaction rollback.
- Episode2+ missing-cursor acceptance before display; accept→leave→reinvite as well as the implemented reject→reinvite; ordinary service and operator floor keys, service duplicate stability, and exact event-key integrity/earliest selection.
- Delayed historical CompleteInboxDelivery after rejection/reinvitation preserving its old authority and zero-floor gap behavior; acceptance/rejection between chunks; valid subagent own AdvanceCursors and foreign/unaddressed-ID refusal controls.
- Physical production-query plans and exact fresh300/due14/reconciliation10 accounting, unchanged1803/579/2560 envelopes, fixed event-floor points and scalar/manifest consistency. Current `ordinary<=100` and `ordinary_due_key_probes==0` assertions do not prove these new relation bounds.
- Remaining inheritance controls: preserved legacy/left/retired/archived streams, generic nonpair join/no-backlog, frozen replay bytes and ordinary ACK-required handoff behavior. Existing relevant tests may supply these; this review does not claim they ran.

These gaps do not invalidate the nine authored tests or require them to become one oversized module. Keep their expected behavior; add the missing independently scoped controls and classify actual baseline results by the first reached assertion. No production change, broad Q4 completion or runtime GREEN is approved by this report.
