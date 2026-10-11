# Task 8: Invitation CLI and presentation contract

Status: DONE for Task8 owned implementation. Clean own commit and required focused verification complete; independent review and integration remain coordinator-owned. No independent approval claim.

Worktree: `/Users/alepar/AleCode/herdr-threads/.worktrees/handoff-delivery-redesign/.worktrees/super-auto-handoff-delivery-redesign--task-ht-0lo.2.3`.
Branch: `task-ht-0lo.2.3`.
BASE: `a95623dc6a6f5a84d48b2d54e09d8701bc978b25`.
HEAD: `d132e79a35915d4558f79b48fb190a4d1ff5804d` — `feat(cli): default ordinary invitations to lazy confirmation`.

## Implementation

Ordinary root agent invitations now explicitly select Lazy by default, including explicit `--lazy`. `--require-ack` selects RequireAck with the installation default duration; `--deadline SECONDS` implies RequireAck and freezes the requested milliseconds. Lazy conflicts with either timed flag. Invitation durations must fit positive signed canonical milliseconds before journal effects. Human invitations retain absent confirmation and existing deadline semantics; the new agent flags refuse in the immediate human namespace. Operator invitations and required acceptance retain their established command variants.

The requested policy flows through MutationSpec to the existing frozen SemanticMutation and wire command. Existing late capability negotiation is retained. The public pane recipient path had an earlier journaled allocation; it now checks invitation.confirmation_v1 before allocation as well. No journal format, historical bytes, operation key, digest, store lifecycle or scanner code changed.

Watch treats explicit Lazy invitations as bookkeeping without attention; RequireAck and historical None remain attention. Compact inbox output retains exact invitation/thread IDs, topic/goal and ready acceptance commands while showing Lazy versus acceptance-required policy and any actual supplied deadline. Absent historical deadline is not fabricated. Required service presentation is unchanged. JSON carries the existing optional fields unchanged. Lazy invitations remain complete/displayable in hook output; no acceptance, registration receipt or completion claim is generated. README and embedded skill explain the flags and confirmation meaning.

Coordinator approved extra exact surfaces tests/cli/cooperative.rs and src/protocol/output_compact.rs. No other scope expansion. No subagents, Herdr operations, real user configuration writes, daemon/server changes or main merge/push/stash performed.

## TDD evidence

All production changes followed runtime RED. Cargo commands used the own absolute persistent target, CARGO_BUILD_JOBS=4, nice, locked/all-features, and the coordinator's exclusive Cargo lane.

First RED: `nice cargo test --locked --all-features --lib invitation_policy`, exit101. Build2m37s, test0.26s; real158.72/user403.00/sys6.58. Four selected:1 pass/3 fail. Meaningful parser materialization and actual run_cooperative→canonical invitation failures were None versus Some(Lazy)/Some("lazy"). The compact test mistakenly selected default JSON, so that failure is explicitly NOT compact behavior evidence. Watch's name lacked the filter, so it was NOT selected. The conflict test passed because unknown new flags already refused; that was NOT semantic conflict evidence. Corrected these fixtures before changing production.

Corrected RED, same command: exit101,9 selected,8 failed/1 passed, no ignored. Build1m01s/test2.39s; real65.11/user131.91/sys5.04. Actual failures: default absent confirmation; explicit supported --lazy missing; signed-duration overflow accepted; compact Text omits policy; actual watch Lazy raises attention; public explicit-seat request reaches canonical Unauthorized instead of capability refusal; actual run_caller_scoped pane request reaches current-target preparation after journaled allocation instead of capability refusal; canonical stored default remains None. The exact historical omitted-field retry passed: a real pre-submit transport failure retained bytes/digest/key, then canonical retry succeeded with absent confirmation. No same-call self-recovery fixture was used.

GREEN: same command,11/11 pass,0 ignored. Build51.74s/test0.86s; real53.54/user160.02/sys3.42. Includes corrected9 plus modern failed-submission exact frozen mode/duration/request replay and Lazy hook display/recoverability guards. The latter guards cover existing preservation behavior and introduced no additional producer changes.

The canonical fixture reuses existing complete caller instance/seat/observed target/binding/cooperative context, with joined caller and same-instance recipient. It asserts persisted policy, frozen duration, creation-based deadline and original inviter. Duplicate calls test both Lazy→timed and timed→Lazy requests without changing the original episode/policy/deadline/origin. Legacy retry verifies absent wire field, exact original caller, operation key and duration; the retained request bytes and digest survive failure unchanged. Capability refusal tests assert no invitation/allocation dispatch and no new intent effects.

## Focused verification

Initial verified source before the guide line-only correction:

- `--lib invitation_policy`:11 pass,0 ignored (above).
- `--lib cli::commands::tests`:87 pass,0 ignored; build0.04s/test0.25s,real0.37s. Includes operator variant, required acceptance and namespace validation.
- `--lib cli::watch::tests`:42 pass,0 ignored; build0.04s/test0.36s,real0.45s.
- `--lib cli::output::tests`:8 pass,0 ignored; build0.03s/test0.00s,real0.08s.
- `--lib cli::human::guidance_tests`:selected0; explicitly NOT evidence. Actual module is cli::human::guidance; covering run pending.
- `--lib cli::skill::tests`:14 pass/1 fail; existing <200-line bound caught the guide expanding to200 lines. Corrected only the guide layout to198 lines, retaining all requirements and the unchanged test/bound. Recompile/check pending.

Final corrected guide/frozen source checks:

- `--lib cli::skill::tests`:15/15 pass,0 ignored; build20.85s/test0.01s,real21.80/user22.42/sys1.58. Existing line bound unchanged; compiled guide198 lines.
- `--lib cli::human::guidance`:4/4 pass,0 ignored; build0.03s/test0.01s,real0.09s.
- `--lib main539_composition`:9/9 pass,0 ignored; build0.04s/test1.16s,real1.24s. Covers actual canonical invitation modes/duplicates, historical/modern retry, capability refusal and preserved human/agent join recovery.
- `--lib recipient_panes_reads_do_not_allocate_and_invite_uses_guarded_resolution`:1/1 pass; build0.03s/test0.08s,real0.16s.
- `--lib operator_pane_invite_runtime_freezes_guarded_seat_and_operator_journal`:1/1 pass; actual public runtime with isolated socket/host fixture and joined in-process threads, build0.06s/test0.12s,real0.23s. Existing operator semantics/journal namespace retained.
- `--lib required_acceptance_journal_retains_revision_across_recovery`:1/1 pass; build0.03s/test0.03s,real0.11s.
- Mandatory `nice cargo clippy --locked --all-targets --all-features -- -D warnings`:exit0,42.03s,real42.11/user66.05/sys2.26; zero diagnostics.
- `cargo fmt --check`:exit0. `git diff --check` and committed `git diff HEAD^ HEAD --check`:exit0. Own `git status --short` empty after commit.
- `scripts/check-no-leaked-processes --root "$PWD" --run-id 979D4B53-0544-4687-A99B-981D5C791AA8`:exit0,`no leaked test processes`. That exact run ID was exported for the final operator/required focused commands; all new tests and earlier focused checks used in-process fixtures without child processes. Checker also scopes own target artifacts through explicit root. No foreign processes were killed.

179 passing GREEN test executions across the focused commands (168 distinct tests; the initial11 invitation checks also appear in the broader modules). No newly ignored/skipped test or weakened assertion/budget. Earlier failures and zero-selection are recorded above and excluded from passing evidence. No routine full suite or default-feature check (parent owns final check after roast).

## Self-review and limitations

Scoped source review found the public earlier allocation gap and preserved the existing late guard rather than duplicating its implementation. Existing recipient fake now advertises supported invitation/send-time capabilities and expects explicit Capabilities→ResolveSeat; its read/no-allocation and caller-separation assertions remain. Historical retry tests do not invent new confirmation fields.

First crate rebuild and corrected RED exceeded the one-minute incremental budget (158.72s and65.11s); GREEN51.74s build and final lint42.03s are within it. No profile/cache/budget changes, new target directories or parallel Cargo were used. No runtime children/daemons/private Herdr sessions were started by new tests; watch fixtures use joined in-process threads. Final scoped cleanup passed as recorded above; no owned runtime process remains.


Final dependency audit: original task-6-seam-review.md retains its superseded CLEAN and later I1/I2/I3 findings honestly. Read the subsequent task-6-seam-rereview-1.md: Spec CLEAN / Quality CLEAN / I1,I2,I3 ADDRESSED for f48857d7. That restoration is in this task BASE ancestry; actual schema source retains Lazy→NULL overdue suppression, frozen inviter-origin projection and passive departed-inviter clear. Task8 does not claim to reverify Task6's whole lifecycle/shared-driver guarantees.

Files changed (all admitted): README.md; integrations/skill/SKILL.md; src/cli/commands.rs; src/cli/mod.rs; src/cli/watch.rs; src/protocol/output_compact.rs; tests/cli/commands.rs; tests/cli/cooperative.rs; tests/cli/output.rs; tests/cli/watch.rs. The shared report is coordinator-owned documentation outside the source commit. No unresolved implementation concerns; broad final integration/PR/default-feature readiness remains parent-owned.
