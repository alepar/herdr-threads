# Q4 SQL and retirement-refusal controls — scoped quality review

2026-10-11. Static review only, design roast OFF; counters unchanged. Parent identifies frozen candidate 89fce, clean with gates PASS and runtime inactive. I independently read the two complete scratch patches, author rationale/seals, actual production SQL/callers, complete-schema migration constraints/triggers, existing fixture/encoding/confirmation paths and relevant TRUST accounting. No Cargo, runtime/SQL execution, source/test/index/Git/Herdr write or nested agent. This report is the only written artifact.

| Scratch patch | Verified SHA-256 | Scoped verdict |
|---|---|---|
| resume-v090-q4-query-controls.patch | 81c92843e18fb546f19162b6f42b0c08589558c766e1d9d6281e739c6ad7c211 | **REVISE — Q4-SQ1** |
| resume-v090-q4-retirement-refusal-controls.patch | 282a560ccb99083c60b1b63e83621bf23b9ee6886b52d68e07f378ce64f2da5d | **PASS, static** |

## Q4-SQ1 — complete the cardinality witness and distinguish point searches from indexed range searches

Location: proposed tests/store/query_work_q4.rs, seeded_body lines83–95 and indexed lines132–149 (same lines in the scratch resume-v090-q4-query-controls.rs).

The 128/8192 noise loop correctly grows same-selected-seat pairs, current memberships/invitations, retained older episodes and timeline events. It does **not** grow occupant_bindings, binding_cursors, lazy_recipients or send_manifests: the measured binding remains the only binding, the cursor relation is empty, and selected recipients/manifests remain fixed at100 (one for the tail test). This leaves a material hole in the stated whole-composed-point SQL witness and the proposal's explicit unrelated-binding/recipient-key dimension.

The plan oracle requires an indexed SEARCH for each expected alias and no SCAN/temp tree, but never checks the complete equality key. An indexed prefix/range walk is still SEARCH, and FullscanStep can remain0. For example a binding_cursors access constrained only by binding_id still looks like SEARCH lc USING PRIMARY KEY and has no data to expose the missing thread point. A relation walk over a fixed100 recipient/manifest rows can likewise have equal work in both populations. Thus alias counts plus same-branch VM equality currently cannot certify that each counted access is an exact point.

Minimal correction:
- Grow unrelated binding identities and cursor rows, including many cursor rows for the **selected binding on unrelated threads**, while leaving its selected candidate threads cursorless. Grow published recipient/manifest keys as well, including selected-seat recipient rows positioned outside the fixed measured100 candidate slice. Keep selected IDs, canonical scalars, branches and cutoff inputs equal between populations; split the point/due fixture from the raw100 fixture if necessary to preserve its exact scan slice.
- Assert the actual plan's full equality constraints (or equivalent unique-key identity) per alias: both binding/thread columns for each cursor access; the full selected recipient key; manifest preparation/message unique lookup; exact binding ID; membership pair; direct-thread unique thread; seat ID; invitation thread/seat/episode; message/event unique key. Avoid depending on SQLite-generated autoindex numbering.
- Keep the same100 literal results, per-key VM/FullscanStep checks, once-only trace, and separate conservative counters. A prefix SEARCH must not become acceptable merely because it uses an index. Parent should obtain a causal failure for an omitted point-key constraint or broadened indexed lookup before claiming this physical regression is covered.

This is a controls gap, not evidence that the current production query scans those relations. The reviewed source uses the intended exact keys.

## Query controls that pass static review

The module imports the **actual** production constants for fresh eligibility, scalarized reconciliation, canonical message lookup, continuation scope, active episode and event keys. It does not duplicate the SQL predicate in a test model. Canonical message mode/thread/sequence/publication is read in the same transaction; literal independent facts are checked before scalar use. The mismatch matrix distinguishes no addressed row from Some(false), and correctly does not invent a second sequence validation that the scalar-provenance contract excludes.

The four selected25-key groups have independent expected outcomes. Full migrated schema setup keeps FK/CHECK/trigger enforcement; preparation/message IDs match the schema's deterministic mapping, Lazy rows precede sealing/publication, fixed current episode10000 is above all8192 older episodes, and retired-seat setup sets state and retired_at consistently. The three-event robustness fixture is honestly labeled as a retained-state fixture rather than a public duplicate-writer claim. Actual floor execution requires90000.

Actual private raw discovery asserts100 visited,300 extra scope charges and25 rendered outputs; exact-ID due calls the real helper and asserts14 in its own counter, with setup3 outside this invocation and no fresh/reconciliation double charge. Tail capture uses a real long UTF-8 body and actual encoded continuation; a retained own cursor must not erase that captured tail, while modern scope loss retires exactly that body. First/followup traces require respectively [1,0] and [0,1] executions of the actual fresh/scope SQL, including fitting. One physical continuation visit and three extra scope charges remain separate from ordinary continuation probes.

The trace observer is connection-local, owns a stable boxed callback state, and unregisters before freeing it on normal return or unwind. It is not process-global and does not conflict with the separately scoped connection-local progress handler. No test starts a process. The reused queries fixture retains its existing unmanaged temporary database-file behavior; this patch does not supply a new cleanup guard or a no-temporary-files claim.

Even after Q4-SQ1 correction, these controls alone do not establish the full256-key caller/reconciliation envelope, public-query/completion bounds, elapsed-time/disk-I/O bounds or all retained-cursor/joined/legacy behavior. Those remain separately owned controls, as the author reports.

## Retirement/refusal controls: PASS, static

All five functions (thirteen isolated cases) start from the existing public Ensure/Send fixture and actual private Lazy prefix. They serialize actual production Watch lines and confirm the actual token when present without inventing a display completion. Actual Reject/Invite and fresh Send establish old publication < new episode creation < new publication, modern legacy_pending0, absent own/sibling cursors and current pending invitation. Querying the actual scope SQL is corroboration of these independently established facts.

The malformed-accumulator cases distinguish invalid identity, valid-but-wrong identity, wrong byte count, wrong skipped body and a non-UTF-8 cursor boundary. Guard-specific error details prevent a generic earlier failure from being mistaken for retirement refusal. The long truncated body uses the existing emitted/skip path; the UTF-8 test changes its captured offset without substituting a different body.

Publication/address cases retain the captured body while removing or contradicting exact authority. The foreign-instance case changes both message and manifest to the same isolated foreign instance, so the captured-instance guard must reject agreement in the wrong instance. Named immutable triggers are read from sqlite_schema, dropped solely for the isolated fault and restored from their exact definition before production reads; even a returned mutation error restores the trigger before unwrap. No FK or CHECK disabling is introduced. Missing manifest deletion is not blocked by the actual retained-message trigger. These are explicitly corrupt fixtures, not new supported producer states.

Public InboxBatchV2 and the explicit legacy_pending1 fixture both require strict InvalidCursor audience refusal, rather than accidentally exercising private modern retirement. The public path retains the real claim and captured cursor. The legacy fixture is correctly described as compatibility state, not a fresh public producer or historical migration witness.

The final-save failure trigger targets the exact Watch binding and an actual change of ordinary_cursor. The expected trigger detail distinguishes tentative continued progress from a passive/context-only write. Every refusal compares full private consumer/item/proof/fence rows and canonical cursor/binding/recipient/receipt/invitation/membership/wake/notice/publication/instance bookends, taken **after** intentional corruption, and separately requires cursor absence plus pending/no-receipt/no-wake state. This is meaningful transaction rollback evidence when run. It does not claim successful recovery/retry, all-database-table equality or physical SQL bounds. Existing positive Q4 retirement and Q1 refusal/rollback controls remain complementary.

The child uses BodyFixture's isolated canonical temp path and cleanup and starts no process. Runtime must still validate that each setup reaches its named guard; no actual refusal or rollback result was produced by this review.

## Verified production and mount provenance

- src/store/lazy_delivery.rs: d271124b66b1515e3b5d8f6df159a6cf18c933867591e221a4f3c18f1b1a1b7c
- src/store/invitation_content.rs: 07937cfaf68524f5b47f903470c4aa836766eaa0bd51b539c3897ccf90451e87
- src/store/queries.rs: 8aa9b787f9a939eadf61f8634dde4937a332421847ac68700c9572a8104030d2
- tests/store/queries.rs mount preimage: be00df205deaaa6dc115a84a48161d03ff7ec19424fecff058e00b06b9738395
- tests/service/q4_body_controls.rs mount preimage: 29401c48f920c6296971166ef9a25c4722907e1e5ea48ae651c4d8812524313f

No other material issue found. Parent owns applying any corrected patch, compilation, actual SQL/VM/trace results, causal attribution and runtime gates.

