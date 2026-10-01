# Failure and native delivery validation

## Goal

Demonstrate the plugin's durable messaging and receipt guarantees across crash boundaries and real native Codex/Claude configurations, preserving independent evidence and all unrelated user sessions.

Parent: [root design](2026-09-27-herdr-threads-design.md). Bead: ht-4is.11. Earlier siblings: caller-attribution, store, seat-identity, scheduler, daemon, CLI, harness and package designs. This suite consumes production composition; it does not substitute mocks for native receipt evidence.

## Adopted shared-contract detail

The [adopted shared-contract amendment, revision 4](shared-contract-amendment-adopted.md) is normative for the exact types, schema, algorithms and ownership described below. Its adoption is a design decision, not implemented or native-tested evidence. Existing acceptance remains required. Logical publication is authoritative; bounded physical projection cannot hide committed receipt/warning obligations. F6 is resolved by the [trust policy](../../../TRUST-POLICY.md) (2026-10-01), which is normative for continuity and attribution; preserve the BOTH-harness gate `ht-910`; shared types, fake ports and store tests cannot satisfy that gate. Formal design review counters and original task review histories are unchanged by these edits.

## Approach

Run deterministic real-store/service fault tests first, then a small isolated native matrix. The earlier attribution probe remains the early feasibility gate; this suite verifies actual packaged end-to-end operation. Test artifacts record executable SHA, exact versions/configuration, timestamps, accepted message IDs/recipient sets, explicit model-issued API calls and SQLite receipt results. A prompt API success, terminal prose or transcript phrase alone is never a receipt.

Provide scripts with a private run directory under the repository's ignored validation workspace or an explicit temporary path. Native scripts create only their own panes/sessions, record IDs as they are returned, and clean up only recorded owned resources. Preserve failed evidence before cleanup. Use isolated Herdr configuration/endpoint/state for host restart/restore and plugin registration tests; never restart the shared server. Existing global hooks and prior PoCs remain untouched. Codex starts with --no-daemon and verifies actual inherited context in a non-login shell; Claude runs natively. Use supported per-session hcom isolation already proven in the research, never blanket approval disabling.

## Deterministic failure matrix

Use real SQLite through the production connection factory, fake clock/host and the composed service. Explicit test-only failpoints can pause/crash subprocesses at these exact boundaries; release builds cannot activate them via arbitrary agent request fields.

| Boundary or race | Required result |
|---|---|
| Before send commit | No accepted operation/orphan receipt; retry can commit once. |
| After commit before response | Durable CLI/API key recovers original message/recipient snapshot. |
| Before wake; after reservation before prompt | Restart reconstructs work; persisted retry spacing holds. |
| After prompt before attempt record | Possible repeated hint recorded honestly; never duplicate mail or inferred ACK. |
| ACK commit before response | Retry returns original actor/result; one success event. |
| ACK/accept decision at or after deadline with daemon downtime | One durable missed-deadline event precedes late settlement. |
| Timely ACK concurrent with due scan | No false warning after a receipt decided before the deadline and successfully committed. |
| Join/leave/send concurrency | One immutable recipient snapshot, invited explicit target included once. |
| Archive/retire/rebind during pending work | Archive allows late settlement; retire never ACKs; repair invalidates old contexts. |
| More than one history/inbox page, output failure and partial UTF-8 body | Every obligation remains discoverable; exact continuation, no implicit receipt. |
| Disk-full/lock/corrupt-schema/unknown-version failure | Explicit failure without false durable acceptance or overwriting history. |
| Lost events/socket denial/reconnect | Snapshot recovery; no retirement inferred from an error. |

Fault tests assert authoritative DB facts, client observed outcomes and pending work separately. Use deterministic transaction barriers for concurrency instead of timing-only sleeps. A test may label an operation ambiguous to the caller while proving it committed; that is why intent recovery exists.

## Native configuration smoke (before final sweep)

Exercise one complete prelaunch flow for each of the four initial configurations: Codex manual, Codex managed, Claude manual, Claude managed. Persist invite and assignment before launch, deliberately omit the initial prompt in one flow per harness, then prove startup/check-in exposes the thread and the root calls read, explicit ACK and separate accept. An empty initial transcript must not destroy the assignment. Retain IDs/calls/DB joins linking each outcome.

For both native harnesses, then exercise:

- Idle ordinary message and deadline-warning wake; successful ACK info causes no independent turn.
- A controlled active tool-boundary arrival in the same native turn; no inference-only interruption claim.
- Native child read succeeds; child ACK and accept fail without advancing top-level offered generation. Parent then settles successfully. Include concurrent child activity and the supported code-mode path if present.
- Clear/new conversation and restart/resume in the same pane retain membership/pending work while rejecting stale predecessor proof and retaining original receipt provenance.
- Recognized approval/question UI is untouched. A controlled human partial-composer-input scenario documents detected/deferred behavior or the accepted residual host race, without sending into real user input.
- A small burst above one default page (at least 25 messages) across two threads drains through continuation and batch exact ACK, coalescing hints. Sender success output remains compact.

Use native agents to ACK the actual IDs they read. Do not have the coordinator ACK on their behalf or edit receipt rows to finish a scenario. Failed/missing receipt remains pending and fails that scenario; manual rescue can be documented separately but cannot turn unattended delivery into a pass. Hooks/wake retries are allowed as designed and measured separately from new logical sends.

## Host and daemon recovery

In an isolated Herdr instance exercise rename/tab reorder, observed pane move, missed move event followed by complete snapshot, actual pane close, host stop/restore with changed terminal IDs, ambiguous restored address and explicit operator rebind. Assert automatic continuity only where evidence establishes it; a successful operator repair is labelled repair. A plain CLI exit preserves seat. Validate independent instance namespaces and no mapping by native transcript equality alone.

Kill only the test daemon at prescribed fault points and rerun ensure. Deadlines and pending work recover without duplicate warnings/writers. Test simultaneous ensure and update with a running older software version using the package suite's fixtures. Native restart tests must never target the shared w4 server.

## Reporting and completion criteria

Report per scenario: PASS/FAIL/UNSUPPORTED with evidence paths. Unsupported required native attribution or launch configuration blocks release; a documented residual optimistic prompt race is an accepted limitation, not a fabricated safety pass. Receipt accounting reports accepted message-recipient pairs, ACKed, pending, retired, duplicate logical records and repeated transport hints separately. Every accepted pair must be accounted for, and success scenarios require all expected model receipts.

Measure marker, tool result and hook-output bytes/token estimates separately under matched workloads. Do not compare the earlier unequal hcom/hmail totals as a new controlled benchmark. Archive sanitized metadata/fixtures; raw native evidence remains private and linked by local run identifiers. Report exact test commands and final tested code SHA. Later code fixes invalidate the stamp and require the relevant verification again; expensive final whole-branch verification belongs after code-roast fixes under super-auto.

## Decomposition

Create independent composed-service fault tests; native Codex delivery suite; native Claude delivery suite; isolated host identity/recovery suite; and evidence reconciliation/report tooling. The two harness suites each own their manual/managed smoke configurations and run as soon as packaged entrypoints exist, before terminal integration sweep. Reporting consumes their manifests but introduces no new model session. Shared native fixture/setup helpers are a small separate prerequisite artifact if needed; they own resource cleanup and never perform hidden receipts.

## Configured wake spacing

The deterministic composed-service suite executes default and higher minimum wake spacing through actual fake-host prompt calls and persistent reservations. It tests new attention before eligibility, restart after reservation, and invalid configuration rejection before any wake loop starts.

## Design roast 1 regression matrix

These cases consume the revised store, scheduler, host, CLI and harness contracts. Use the same public routes as production wherever possible; direct SQLite assertions establish facts but never manufacture receipts.

- **F2/F3:** Populate at least 205 rows for every collection, including participants, delivery recipients, pending operations and seat history. Traverse all pages using returned arguments. Find an old pending message after 250 newer acknowledged messages through `pending-receipts`; include invited, left and archived obligations and unstarted receipt timers. Concurrent appends/removals obey keyset/high-water rules; relevant mutable-filter changes return explicit cursor-stale results. Exercise `read --recent` continuation beyond its first page.
- **F4/F9:** Recover a zero-joined thread with an owner-UID operator invitation; the target still explicitly accepts. Test active versus archived threads, existing pending invitations without deadline reset, an accept/invite race, retired targets, wrong UID and each forbidden operator operation. Same-UID scripts deliberately using administrator mode are permitted; do not claim human identity. Replays preserve original administrative provenance.
- **F7/F11:** Search 30,000 sparse/no-match candidates with deterministic work limits and read-worker barriers proving writes/hooks continue. Empty pages expose progress and continuation. Validate actual encoded byte bounds, escaped data, long paths and continuation framing; an impossible budget returns a typed error. Hostile topics, labels and message text remain marked quoted data in the native hook wrapper, never interpolated instructions. Required topic/stats recovery must pass on both native harnesses; omission reports degraded/unsupported behavior.
- **F8/F10/F12:** Inject indefinitely delayed, late and reordered host responses, subscription loss, partial snapshots and changed host incarnation. Prove bounded calls, cancellation, finite queues/leases, no host wait under the database writer, and rejection of old response epochs. Events invalidate observations but cannot directly retire a seat. Exercise replacement before fresh observation, during preparation and before the transaction decision. A fresh RPC returning cached execution metadata is insufficient proof. Record the explicitly accepted gap after observation separately from stale-proof rejection; do not claim atomicity between native process replacement and SQLite commit.
- **F13/F17:** Pause before and after the store's decision-time sample, across a warning deadline, before physical COMMIT and before response. Due scans re-read authoritative rows. ACK, accept and retirement share overdue preservation, including equality at the deadline, mixed valid/invalid ACK batches and unstarted deadlines. Retirement rollback is per fence/cleanup quantum, with classification frozen at the successful fence; earlier committed cleanup remains after a later failure. Long preparation delays expire the mutation permit and cause no effects; successful boundary-crossing tests stay inside its remaining lifetime or reacquire proof. A post-sample pause does not reclassify the decision by physical commit time.
- **F14:** Assert actual fake-host call times under forward/backward UTC jumps, default and raised minimum spacing, attention arriving during an attempt, completion delays, reason clearing, reservation crash and repeated restart. Monotonic completion/reservation anchors and conservative boot guards enforce elapsed spacing; UTC is diagnostic for wake eligibility. A two-second completion with a 30-second interval permits the next attempt no earlier than second 32.
- **F15:** For each native harness fail the only startup check-in, restore service while the recognized root is idle and unregistered, then prove a generic hint triggers fresh root registration and ordinary read/explicit ACK/accept without manual rescue. Hints alone neither register the occupant nor start receipt timers. A permanently stopped unsupervised daemon requires ensure and is reported separately.

F6 (resolved by the trust policy) keeps these fixtures: startup before repair on a recovery-baseline target, addresses that changed, several plausible targets, persistent holds across daemon restart, explicit rebind to an unowned target, explicit operator fresh-role choice, and refusal to replace an already owned target. Snapshots do not allocate seats. Under unbounded restore ambiguity all unclaimed baseline targets remain held; newly observed panes after an authoritative baseline follow normal allocation rules. Add: hold lift after the last repair, unique session-id reattachment (and refusal on no or multiple matches), and both abandonment resolutions for an owned target.

## Design roast 2 retirement regression matrix

R2-F1 changes a whole-backlog transaction into a terminal fence plus bounded durable cleanup. Use real SQLite/composed service and deterministic barriers; these are required implementation tests, not passing evidence.

- Seed at least 10,000 pending receipts plus invitations across many threads, and a separate case with one wide thread (at least 1,000 members). Include overdue, exact-cutover, future, unstarted, already-ACKed/accepted and previously warned obligations. Add a large retained settled history so indexed cleanup cannot hide a full history scan.
- After only the fence, reject send/invite/ACK/accept/check-in/rebind and timer starts for that retired seat as applicable; send by others excludes it, pending queries omit it, inspection exposes effective terminal status/progress, and last-member orphan recovery works. Pre-fence prepared permits/advisory scan candidates cannot bypass the fence. Original ACK/accept provenance and history remain intact.
- Freeze cutover, advance UTC past all future deadlines and restart repeatedly between quanta. Exactly cutover-owed warnings appear once; future/unstarted obligations gain none. Preserve earlier warnings across backward UTC jumps. Every newly materialized warning precedes its seat/thread audit despite concurrent unrelated timeline events.
- Crash before/after the fence, within a quantum before COMMIT, after a committed quantum and after finalization before response. Verify atomic fence or quantum rollback, retained earlier progress, exact once audit/warning keys and complete eventual cleanup with no new host observation, client poll or user action. Disk failure exposes pending/degraded state and cannot undo the fence.
- Instrument every quantum: at most 16 fixed-size units, no new unit after five ms, bounded indexed row operations/structured fields, no membership fanout or message-body loading inside a unit. Assert retirement warning creation enqueues no new wake reasons; existing reasons for settled conditions suppress even before lazy pruning. No queued whole-backlog loop or writer ownership across quanta.
- Queue unrelated valid ACK and fresh check-in between quanta while health/read requests continue. Assert foreground receives a turn after at most one retirement quantum and background after at most eight foreground decisions, including continuous foreground arrivals and several competing cleanup jobs. With deterministic bounded per-unit clock costs, prove permits remain inside 250 ms and ordinary hooks inside 1.5 seconds; do not extend their deadlines. Also record real observed writer/ACK/check-in latency on the large-backlog fixture. Explicit storage stalls retain typed expiry/unknown-outcome behavior rather than a hard real-time claim.

This punch-list change preserves F6's original parked dissent and all positive native attribution/recovery gates. It is no new roast/adjudication or claim of native support.

## Adopted shared-contract regression matrix

These additions supplement every original matrix above; the adopted amendment is normative for exact types and task ownership. Use real production SQLite/service paths and deterministic physical-row/unit counters, not elapsed-time-only tests. Keep fake-port evidence separate from actual native capability and preserve F6/ht-910.

- Stage a wide uncapped send in multiple bounded quanta while unrelated ACK/check-in progress. Publication is all-or-none across the full logical receipt and unavailable-warning set. No final recipient scan is allowed. Timeout/disconnect leaves staging invisible; only a fresh authorized exact-key retry may publish. Expired or reused proof never survives staging.
- Change a late-page recipient by registration, replacement, unavailable/exit, mapping uncertainty/repair, retirement or host epoch invalidation before publication. Scalar eligibility/membership/lifecycle/timeline/config fences force exact refreshed preparation or no message. Instrument constant final work as recipient count grows. Test sealed count/dedup integrity, checked sequence-range overflow and rollback with no partial timeline/operation.
- Commit two competing preparations for one unavailable episode. Exactly the first published send supplies one canonical immutable warning ID/payload/sequence. Send→check-in→continued history/detail/search/wake before materialization discovers the warning; an earlier stuck materializer cannot hide it. Projection/restart across a cursor changes neither identity, position nor count; there is no duplicate public event or empty history slot.
- Freeze event membership, then join/leave/rejoin/retire/settle while warning attribution advances in <=16-unit/five-ms quanta. Historical recipient set stays exact; settled conditions suppress present wake. Retirement warnings create no recipient job/fanout. Crash/rollback at each job boundary preserves prefix and eventual complete attribution.
- A checks in through C, old warning E<=C is still being attributed, then B replaces A. B sees unoffered E until B's own valid check-in; old-generation check-in cannot advance B, same-execution repeated hooks do not reset, and retained reservation spacing never resets. Budget failure producing offer metadata returns error and cannot advance an empty frontier.
- Thousands of unstarted receipts immediately derive first eligible immutable anchor time after check-in while timer indexing remains bounded. Receipts sent between several registration/replacement anchors use their earliest qualifying event, never latest registration or materialization time. ACK/retirement before physical indexing uses that same deadline and frozen duration, including equality and UTC jumps.
- Indexed pending-unwarned scans reach new due records behind at least 30,000 settled/already-warned rows without visiting that history. Bound physical visits before high-water filtering. Permanently failing invitation phase still permits receipts and vice versa; healthy phase restarts independently. Marker/event/job and partial committed progress survive reopen without duplicates or skipped retry positions.
- Traverse >205 compound inspection/journal entries with same-command exact continuation. Test scoped topic/body search and logical event order, explicit system detail, ordinary body continuation, exact selected text/JSON bytes and independent full wire cap. Exact aggregate/orphan validation interruption releases/rolls back the writer; no partial count is called zero or exact.
- Wrong expected instance never invokes a handler; request/version/instance/boot correlation is mandatory. Real commit-then-response-loss returns typed unknown_outcome with key/intent retained; failed connect returns its distinct pre-submission code. Explicit exact-key recovery returns original result once; no automatic second submission. Journal paging works daemon-down without body reads.
- Managed launch rejects missing/occupied/unknown-empty/held/changed target, inspects owned hook configuration and preserves native argv/permissions with Codex --no-daemon once. Observe bounded startup/unknown outcome without any registration/timer/offer/ACK side effect. Native support remains contingent on actual supported hook/attribution evidence.

The shared substrate, encoder checkpoint, bounded materializers and closed-Task9/16 adaptation follow-ups must be reviewed before their consumers claim these cases. Closed original tasks retain acceptance and fix histories; final composition/recovery/native reports and sweep consume the new artifacts. New runtime changes invalidate affected earlier evidence under the existing refresh rule. These spec amendments do not increment or reset a formal roast counter or assert passing results.

## Shared-contract adoption record

2026-09-27: Reconciled this canonical spec with the adopted revision-4 shared contract. Detailed normative algorithms/types and preserved revision responses are in the [adopted shared-contract amendment, revision 4](shared-contract-amendment-adopted.md). This is specification work before the next formal design review; no source implementation or native-support completion is asserted.

## Post-Implementation Notes

*As this design is implemented and iterated on — bug fixes, adjustments, anything that diverged from the assumptions above — append a dated note here, whether or not a formal debugging skill was used.*
