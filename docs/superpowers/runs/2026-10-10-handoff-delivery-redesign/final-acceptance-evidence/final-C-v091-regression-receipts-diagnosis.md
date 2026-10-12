# Final C receipt regression diagnosis (source only)

Confirmed fixture regression caused by the phase-6 Main36 composition. Candidate supplied by parent: `3d23527675170cacaec470aa13b3baf53688d4ba`. This diagnosis ran no Git, Cargo, SQL/database, daemon, Herdr, tracker, or test commands; production and test source files were not edited. Only this diagnosis and the scratch patch were written.

## Actual evidence

- `/private/tmp/ht-final-C-d8427582/focused-results.json:3618–3621` records binary `herdr-threads`, exact key `store::receipts::tests::receipt_discovery_upgrade_combined_reset_warning_and_cursors_roll_back_together`, status **PASS**. This verifies the exact case, not the entire prior matrix.
- `/private/tmp/ht-final-C-v091-3d235276/nextest-run.log:776–792` records the same key **FAIL**, at `tests/store/receipts.rs:4867:147`: `CHECK constraint failed: (available_at IS NULL) = (deadline_at IS NULL)`. The failure is the fixture INSERT, before `initialize`, the abort-warning trigger, or any rollback assertion.
- The archived pre-Main fixture in `docs/superpowers/runs/2026-10-10-handoff-delivery-redesign/task-12-actual-composition.diff:17779–17859` and the current named function were compared as text: function bodies are byte-identical after removing diff `+` prefixes. Thus the old semantic cutoff remained 37.
- `resume-v091-composition-report.md:3,7,59–67` in this workspace records composition `495703bd262843558cedd6b974b0d0b9216c40e6`, parents integration `9dc3b52fa1265443d6dcf8c5f649cb1369674b61` and Main `20e7ca73e41a9449376169ab6875c0675da02edc`, exact Main1–36 preservation, and R100 feature renames. In particular receipt-send-time moves 0037→0038 and deadline-backfill 0039→0040. `resume-v091-composition-light-review.scratchdiff:338–374,1143–1160` independently contains that source diff. `sweep-fix-final-review.md:5` records pre-Main tip `f4b74f311b28c39eec0c1a4a8d726c1fbd8e5c10`; this diagnosis did not independently inspect that Git object. The actual pass claim comes solely from the exact d842 test result above.

## Causal source path

1. `tests/store/receipts.rs:4426–4444` builds a new empty SQLite file, reads/sorts migration files, executes precisely `take(version)` real SQL files, then stamps that actual prefix. It does not construct a latest-schema image and relabel it.
2. The failing test at `4865` requests prefix37. With Main36 present, 37 ends at `0037_invitation_confirmation.sql`; the send-time rebuild has not occurred.
3. `migrations/0001_initial.sql:428–442` creates `receipt_state` with the paired-null CHECK. The test's INSERT provides `deadline_at=5000` and omits `available_at` (NULL), violating that genuine pre-send-time constraint. The preceding physical receipt is valid (`discovery_receipt`, `tests/store/receipts.rs:3657–3660`, supplies both values).
4. `migrations/0038_receipt_send_time.sql:89–107` drops/rebuilds `receipt_state` with independently nullable availability/deadline fields; no paired-null CHECK remains. This is the unchanged receipt-send-time production behavior, previously numbered37. `src/store/schema.rs:713–726,5143–5185` now performs the rebuild at37→38, then body bounds39 and deadline capture40.
5. Genuine prefix38 therefore restores exactly the original intended historical receipt state: send-time schema is present; neither `receipt_deadline_backfill` (40) nor `receipt_discovery_backfill`/sparse projection/origin triggers (43) exists yet. After seeding physical and sparse rows, real `initialize` creates both backfill cursors with populated high waters. `migrations/0040_receipt_deadline_backfill.sql` captures physical/prepared bounds; `0043_receipt_warning_origins.sql:27–40` captures physical/sparse bounds.
6. `src/store/receipts.rs:716–874` performs producer repair, combined completion/reset, and warning insertion in one decision transaction; the in-memory cursor is assigned only after success. The abort-warning trigger still exercises rollback of all this work. Advancing directly to latest or supplying an artificial `available_at` would instead change the historical precondition and is not proposed.

## Minimal proposed correction

`final-C-v091-regression-receipts.patch` changes only this named test's real historical cutoff37→38 and adds one explanatory comment. No shared helper or neighboring test changes are needed to restore this case. There is no explicit user_version assertion elsewhere within this named test to renumber. All original INSERTs, trigger injection, reset/cursor checks, producer completion rollback, zero derived rows, absent warning marker, retry warning count, completed reset, origin count, and durable reopen condition count remain byte-for-byte intact. All remaining file bytes—including unrelated late/NoEffects controls—are unchanged. Main1–36 and every production file remain untouched.

Static patch checks: exactly one matching named-function prefix was replaced; archived/current original function bodies match; proposal changes only the cutoff/comment. Original `tests/store/receipts.rs` SHA256: `2a37766240f6348f104550f27e0e0913795cb856c636bee194453ae24f5d4b49`.

No corrected runtime outcome is claimed. The parent owns applying the patch and its authorized verification; this is a confirmed previously-green regression diagnosis and source-only minimal proposal, not a new review/fix loop.
