# Task15 independent final schema review preparation

Status: PREPARATION / IMPORTANT OPEN; no final source verdict. Reviewer `/root/code/d_schema_final` performed read-only source inspection, no Cargo, source edits, git/bd changes, or threads operations. Writer was still preparing proofs. Final verdict requires author freeze, cumulative package and exact-source delta recheck.

## Point-in-time consulted source

D checkout `.worktrees/super-auto-handoff-delivery-redesign--task-ht-0lo.3.2`, branch task-ht-0lo.8.4, baseline ee1e598c3ee96566ddc9b397185cdc97c8bf6218. SHA256:

- src/store/invitation_content.rs: 404f6d49010080312d3a69df150c4840145120e0137f518e265096ec4d39e8c1
- src/store/presentation_relevance.rs: 0a3826265cdae73f956200badaecbea11f78444ae6c2b93e127508b7e1ddc2b2
- src/store/schema.rs: 770fd1962356a23236ca679b1268b591bb85eeccf5ae9b376742364e1ab726eb
- migrations/0040_invitation_presentation.sql: 83e7348a425907dc7ad55021960812e933f4126cdaacefbb7f90d28382118326
- migrations/0041_presentation_relevance.sql: 0142aa5a67146fbc5bd77380609a86b7f892b32704fb4a2f32f36ba35f3523c7
- TRUST-POLICY.md: 33a1b6683a4ffb7ebfe4f8445721a6870155ce28a6460c316b5e280f4ad6bb5e
- src/protocol/ids.rs: 98bc681eb6231a834e189142638a6ba8abca4507d929453e9ab0e842c577530b

## Important I1: private reads do not fully enforce bounded preallocation

`invitation_content::load_raw` (135–157) guards many bounded TEXT fields before String/serde allocation, but omits four private thread cursors: context_after_thread, context_thread, mod_context_after_thread, mod_context_thread. These are unrestricted SQL TEXT and deserialize into unrestricted String. Canonical producers use an empty sentinel or ThreadId, whose existing parser requires <=128 ASCII-graphic bytes. An oversized/malformed repaired cursor can therefore be allocated and accepted/preserved by passive Refresh rather than refused. This is a fixed-private-state geometry issue, not adversarial caller verification.

The instance_id field is also uncapped at this loader, but no UUID36 constraint is justified: CallerClaim uses String and fixtures use non-UUID instances. The selected relevance validator already requires <=128 ASCII-graphic instance scope; determine canonical producer compatibility before deriving a shared loader constraint. Do not invent a new instance admission policy.

`presentation_relevance::load` (319–346) uses `r.get::<_,String>(proof_json)` before checking PROOF_BYTES32768. SQL CHECK protects ordinary writes but cannot enforce the declared preallocation bound on repaired/check-bypassed corrupt rows. Use an exact borrowed ValueRef TEXT-length guard before allocation/serde, without changing cap/schema/authority.

Additional private item read paths: `item` reads context_key/execution as String; Refresh reads mod_key/mod_execution, including up to10 cleanup rows, as String before the SQL4096/128 caps are repeated. The same borrowed bounded-text primitive should cover these fixed private item records; this does not extend review to arbitrary canonical body/goal fields.

Required actual evidence: reachable corrupt >128/malformed thread cursor under Pull and Refresh refuses before durable private/canonical mutation. An oversized proof fixture demonstrates rejection, while preallocation ordering additionally needs source inspection or instrumentation: ordinary API outcome alone already rejects after allocation and cannot prove this fix. Author/coordinator informed immediately; no reviewer source edit.

## Source-backed observations, not test PASS claims

- Compared every migration1..39 byte-for-byte with baseline: no differences. 40/41 remain untracked/unlanded additions at inspection time.
- 40 adds epoch DEFAULT0 without CHECK, indexes the existing pending projection, and creates bounded fixed-consumer state/item tables. 41 adds exactly four checked scalar columns and confirming-invitation bootstrap, not message publication fanout/backfill.
- Parsed all fence trigger bodies: exactly87 hooks on29 tables, all87 bodies byte-identical guarded singleton increments. Missing/negative/MAX guard occurs before revision update. The inventory includes warning_close_sweeps and excludes private consumer/proof/seen bookkeeping, avoiding self-invalidation.
- Source confirms exactly one relevance advance before replay/control/no-output selection. Confirm calls no relevance walker/revalidation. Source ranges charge physical LIMIT1 rows before typed judgment. Snapshot highwaters consume13 of26; source/fallback reserve2/2 and3/3; keyed microstep limit64; witness limit64; shared source+fallback100. Direct positive witnesses are evaluated independently of fence churn; global fence gates absence only. Membership fallback intersects captured/current highwater and checks overrun before range query.
- Close coverage uses indexed first successor, then tests `(after,through]`, charging the seek and returned row. Existing canonical producer in schema.rs appends after=MAX prior through only when high>after. Final review must inspect all writers and the same-commit retained-chain TRUST text in cumulative package.
- Publication helper captures fixed manifest highwater, reads one LIMIT100 range before recipient filtering, uses <=200 keyed probes, <=3 head/range seeks, and advances irrelevant rows without version/output/wake. Overflow uses checked_add in the deciding transaction. Pull maintains before replay; Refresh preserves presentation-owned bytes via load_raw and computes original same-instance owner OR before header changes. Ordinary scan freezes native/passive tuple at START and never overwrites active walk/carry.
- Private carry remains155731; partial49415; last-control stamps <=32 and worst tuple82 bytes gives2657 under4096. Four publication scalars live outside carry. Actual serializer/producer algebra and replay/corruption proofs still require final evidence review.
- Migration dispatcher commits40 separately before41; late41 failure honestly leaves durable40, not an outer atomic39→41 claim. Revised41 catalog verification reconstructs exact ALTER statements atop40. Final actual populated39→41, real40→41, late rollback/reopen/FK/catalog,4S CHECK-work and pending-bootstrap cost evidence remains required.

## Remaining before final verdict

Read frozen complete cumulative source/interface package and governing exact authority/reviews; audit actual VM/query-plan evidence for13 sources, publication100/200/3, projection bootstrap/index work, fixed-private serializer maxima, writer coverage and schema tests. Recheck I1 correction and actual outcomes at frozen hashes. Read latest passive worker/grace/rebind/body/carry/rollback proofs; no proof is presumed passing from implementation source. No CLEAN, readiness, merge, or capability advertisement recommendation is made by this preparation.
