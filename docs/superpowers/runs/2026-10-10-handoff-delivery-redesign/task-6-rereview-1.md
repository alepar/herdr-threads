Spec Needs fixes
Quality Needs fixes

I1: NOT ADDRESSED. The separate open-origin source fixes actual attention calculation, mod fingerprint calculation and native offer gating, but ordinary native candidate discovery still drops the seat before reaching those corrected stages.

### Strengths

- `src/store/attention.rs:1146` selects a bounded, instance/seat-indexed open-origin warning source independently of notice offers. `src/store/attention.rs:1202` merges it after informational warning narrowing, so direct native attention and mod fingerprints retain the open warning without reviving delivered notices.
- `src/store/wake.rs:197` removes the legacy offer gate while an origin-addressed invitation condition remains open; unchanged reserve authority validation remains in place. The actual candidate/reserve regression exercises that corrected stage (`tests/store/control.rs:10873`).
- `migrations/0031_invitation_warning_origin.sql:8` backfills only current open invitation conditions with matching invitation/thread/warning source evidence, accepts recorded or exact legacy origin only with same-instance seat evidence, and leaves unknown origins null. Its partial index excludes closed history; its trigger freezes the paired address on the retained condition row (`:24`, `:27`). New conditions resolve their own origin at INSERT (`src/store/schema.rs:3920`).
- The two post-offer regressions retain settled informational delivery, check native/mod attention, and explicitly accept late to prove clearing is passive (`tests/store/control.rs:10791`). The real prior-schema1..30 fixture covers stored, exact legacy and unknown origins, unchanged event bytes and a fresh episode's distinct origin (`tests/store/invitation_due.rs:619`).

### Issues

Critical: None.

Important — I1 remains incomplete at native discovery: `src/store/attention.rs:1202` now adds the independent source to `wake_warnings`, but the public candidate producer's unchanged `discover_wake_candidates` calls `seat_has_pending_rows` first (`src/store/mod.rs:1121`). `WAKE_PROBES` still contains only six older sources (`src/store/attention.rs:1051`): invitation/receipt rows, legacy open-warning projections, and programmatic notices above `notice_frontier`. It has no probe of `warning_conditions_invitation_origin`. After the left inviter's notice is committed, the programmatic probe is false; migration25's transition trigger has removed the warning's legacy open projection (`migrations/0025_warning_notice_delivery.sql:4`). With no unrelated pending work, discovery skips this seat and never invokes the fixed attention calculation or `load_candidate`. The new native test manually calls those later functions (`tests/store/control.rs:10873`) and therefore passes without establishing production discovery. Add a bounded instance/seat-leading existence probe of the open-origin partial index to native discovery, and extend the post-offer regression to exercise `discover_wake_candidates` or the public `wake_candidates` API with the inviter otherwise idle. The retained open condition must be found after a committed offer and cease to be found after explicit acceptance; passive observers/clears and existing host/composer/working guards must remain unchanged.

Minor: None.

### Checks and scope

- Reviewed original I1, Task6 brief, updated report and the complete36075-byte fix diff for ae922aba412bf69f457818ecba8341e224f339b4..50c397f4196002cca4e8ee65557da049f974fd02. Initial combined report output was truncated; recovered the complete appended fix-round evidence separately. No source/index/branch mutation; only this report written.
- Mandatory test-change audit ran the fix-range `git diff --stat ... -- tests` and `git diff ... -- tests` capped400lines. Output fit within400lines: no truncation. Three files,303 insertions/2 deletions. The only removed lines are catalog count literals143/183 replaced by144/184. No assertions weakened, tests deleted, or new ignores/skips introduced. `tests/store/attention.rs` remains unchanged; its previously ignored benchmark inventory is enumerated in the author report.
- Named outside-diff risk: **the new source may be unreachable from public native discovery despite corrected direct attention tests**. Focused call-site lookup found `seat_has_pending_rows`'s sole production use, then inspected that exact discovery gate (`src/store/mod.rs:1089`..`:1143`) and its old six-probe definition (`src/store/attention.rs:1051`..`:1103`). This source trace establishes the Important finding without test reruns.
- Named outside-diff risk: **native admission or mod narrowing may still suppress the newly merged source**. Checked the shared mod call (`src/store/seats.rs:4642`) and native reserve's current-candidate/authority revalidation (`src/store/wake.rs:473`). The fix reaches mod through the shared `wake_warnings`; direct native offer bypass reaches reserve while retaining authority checks. The remaining failure is the earlier discovery gate described above.
- Author evidence: actual3RED followed by3GREEN; lifecycle74pass/1pre-existing ignore; projection17pass/15enumerated pre-existing ignores; metadata1pass; catalog1pass. First style lint failed on field_reassign_with_default, then the equivalent initializer correction passed required lint in30.20s with zero diagnostics and fmt clean. No reviewer Cargo/test/full-suite/config/Herdr actions were run. Reported bounded EXPLAIN evidence uses the origin index and primary-key message joins without a temporary sort; no global Task7 receipt-driver guarantee is inferred from this fix.
- Migration31 remains honestly provisional after local30. Global renumbering, fresh overlapping schema review after rebase, and Task7/8/10 integration obligations remain coordinator-owned.

### Assessment

Spec Needs fixes
Quality Needs fixes
I1 NOT ADDRESSED: direct attention, mod and reservation behavior are corrected, but actual native discovery can still discard the still-open inviter condition after informational settlement.
