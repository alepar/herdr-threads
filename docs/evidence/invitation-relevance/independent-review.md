# Independent code review

Reviewed 2026-10-07 against actual HEAD `36d86ad9d69472d5c29ed4d827e858f67ea35975`: all tracked working changes and `migrations/0025_warning_notice_delivery.sql`. Pending broader handoff design/evidence was excluded. This reviewer read AGENTS.md and TRUST-POLICY.md first and made no source, index, Git, Herdr, shared-server or user-configuration mutations. No additional agents or test processes were started.

## Findings

**Critical:** none found.

**Important:** none found.

**Minor:** none outstanding after follow-up review.

The initial review found two nonblocking nits: 1,001 retained delivered transitions could saturate the legacy pending count into `(0, true)`, and the guide's retry wording conflicted with immutable operation replay. Both are resolved by the follow-up delta reviewed below.

## Assessment

**Final merge verdict: approve**, subject to the coordinator's final mandatory verification. No outstanding Critical, Important or Minor finding remains in the reviewed patch.

The invitation change supplies the complete canonical goal, uses terminal-safe escaping, and omits an individually oversized goal only after testing its standalone page fit; the compact fallback carries pinned read-only inspection routing. Ordinary invitation acceptance is conditional on both topic and goal versus role/remit, including addressed-message invitations. Required invitations retain their separate command/procedure; inbox, subagent restrictions, membership authority and display ACK behavior are unchanged.

The warning change reuses the unique recipient/event delivery projection and exact occupant frontier. Settlement verifies the oldest-first carried prefix inside the deciding transaction. Migration backfills existing physical and published manifest-backed transitions, projects future attribution, and audits the added trigger while retaining the previous projection audit. Condition insertion precedes production recipient attribution; clear recipient cutoffs and fanout remain unchanged. The relevant equality probes use existing unique keys, and delivery-page probes use seat-leading ordinal indexes with bounded walks. Fresh history removal does not rewrite immutable stored operation results; successor generations intentionally get their own informational offers.

The separate advertised `attention.notice_delivery_v1` result preserves the v1 digest wire shape and publication tokens. Capability support defaults false, the selected client forwards the cached socket client's capabilities, and the delivery hint handles both later attribution and pages remaining behind an unchanged logical token. Wake selection checks exact informational delivery before accepting the older coarse offer cutoff; late attribution still kicks its existing producer. Read-only query results share a canonical read transaction.

## Evidence and limits

Reviewed regression source and retained RED/GREEN logs for invitation metadata, built-in transitions, late attribution/wake, manifest-backed warnings and tool-boundary delivery. The final focused log available during review reports **338 passed**, and the retained clippy log finishes successfully in **27.42s**. These are observed coordinator evidence; this reviewer did not independently execute tests or run the full suite. The existing >16 service-notice test and new built-in tests cover independent recipients, successor occupants, exact carried pages, preserved history, migration backfill and trigger tampering. Existing response-loss/historical-operation tests continue to exercise the unchanged operation journal.

The five BEFORE/five AFTER invitation pressure reports are read-only simulations. BEFORE did not reproduce redundant human confirmation on clearly relevant invitations; all five BEFORE runs held the broader operational invitation. All five AFTER runs accepted it after considering operational role as well as narrow remit. This supports the role clarification and does not establish native-model reliability. Some reports misattribute rules to the fixed top-level instruction; assessment.md acknowledges that limitation and the later shortened conditional header.

## Follow-up review and exact fingerprint

The follow-up modifies migration 0025 to remove the shared legacy open-warning projection parent only after the canonical transition has a generalized delivery row. The existing legacy close-projection trigger removes its recipient projections. This is safe under either recipient-trigger order: if the old trigger runs first, parent deletion removes what it inserted; if the new trigger runs first, the old trigger's parent lookup finds nothing to insert. Other recipients retain pending attribution-job backlog until their own generalized delivery rows publish. The backfill cleanup explicitly checks canonical transition identity, so noncanonical legacy warnings retain their previous path. Immutable event, condition and recipient history is retained. The guide now distinguishes fresh offers from exact operation retries.

Reviewed the two new real-store tests: 1,001 delivered events must leave `(0, false)` and the same pending work-step count as the small case while retaining all canonical/delivery rows; a first attributed observer removes the shared parent while the affected seat still receives its independently pending late attribution. Also reviewed updated hook-entrypoint tests: conditional ordinary accept labels and remaining informational pages now trigger fresh offers despite an unchanged publication token.

Observed follow-up evidence: cleanup regressions **2 passed in 2.12s** after their retained RED; all built-in warning/cleanup regressions **9 passed in 2.74s**; cooperative-checkin module **52 passed in 2.52s**; schema selection **115 passed in 5.81s**; three hook-entrypoint rechecks **3 passed in 5.497s**. These remain coordinator runs, not independent reviewer execution.

Fingerprint at follow-up review (before commit/rebase):

- Base HEAD: `36d86ad9d69472d5c29ed4d827e858f67ea35975`.
- SHA-256 of exact `git diff --binary HEAD` bytes, covering tracked production/tests/policy/guide changes: `1ede646d41fe6c19ecb8c36a4f79dab0043302928d9ef6e3508c77186f87ccd9`.
- SHA-256 of untracked `migrations/0025_warning_notice_delivery.sql` bytes: `be0ff728fb17015517364604d2d2fe55a5a826665fb1f34e23ecf2f854431417`.

The fingerprints exclude untracked design/evidence documents, including this review report. Changes after these fingerprints require the coordinator to assess whether further review is needed.
