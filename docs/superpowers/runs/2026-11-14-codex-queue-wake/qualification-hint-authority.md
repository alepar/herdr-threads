# Canonical hint authority is separate from legacy activity

Astra268 approved this correction after the frozen6ca run completed; main's next
full-run disposition is relayed in Astra269. The failed6ca suite remains immutable
in qualification-final-hint-failure.*:9778PASS/1FAIL/64profile skips, first scoped
cleanup clean. It does not qualify this corrected source.

Root cause: service reused update.edge.is_some as permission to publish host wake
hints. Restoring cooperative legacy activity on canonical absence inadvertently
widened hint admission. The fix adds explicit internal NativeTurnUpdate.hint_eligible.
Default/no-canonical constructors set false. The canonical helper sets true only
for an already accepted Some(edge); stale/refused edges stay false. Service records
the legacy memory edge independently, then gates note_wake_hint on the explicit
canonical flag. Peer/instance/role checks and wake_hint_subject remain unchanged.
All constructors/mocks were enumerated; the field is internal, not serialized,
not a new wire field/default and not client-supplied authority.

The causal test first changed only the recorded expectation to true and asserted
Busy memory, retaining the original unregistered hints-empty invariant. Old code
then failed that actual hint assertion (1RED,0.83s), establishing the real defect
rather than hiding it by changing an expectation. Corrected focused matrix:
59PASS4.091s,196scompile, all targets/features/profileci -j4. It covers the named
service handler, registered harness PID4242 instead of reporting hook77, unregistered,
wrong-host, recovery-hold and subagent negatives, native identity/refusal semantics,
legacy memory refresh, turn-state and fallback/selection controls. Fresh e9d4713b
first scoped process/temp checks CLEAN; JUnit retained. The causal-red cleanup was
clean as well.

Clippy all-targets/all-features PASS23.52s; default-feature, fmt, neutral, wire,
docs and whitespace checks PASS. No new model-visible text, wake content, schema
or configuration behavior. Existing P1 ht-d2jq.8 and soft-poke disclosure remain.

This correction is frozen for narrow source/evidence acceptance. Only afterward
main's conditional authorization permits exactly one corrected unfiltered cap4
suite with a fresh run ID and first scoped cleanup, using distinct raw filenames.
No live run or broad review is requested, and no corrected-full PASS is claimed yet.
