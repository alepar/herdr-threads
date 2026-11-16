# Retained queue fallback and default-off qualification

Basis: `2bc6f9713d0805ddd3473d83e40af2c17139fa95` on `feat/codex-queue-control`.
Main remains `4dabf06ef90c8c9e8c459b524b509af3a2e98b31`; migration 0018 remains
unreleased and next free. This is scoped implementation evidence, with integration
still held for bounded design/integration review and the required final frozen-tree
unfiltered cap-4 suite. It is not a release qualification or a new live run.

## Resulting behavior

Main48 relayed the user's default-off decision; Astra242/244/246 supplied the
remaining design checkpoints. An absent/false `programmatic_wake` setting keeps
native typed wake. `true` opts in only new programmatic adds through existing node
settings and startup semantics. Retained fences and recovery, including cleanup
helpers, continue while off. Message bodies and ACKs still use hooks.

A retained Active Accepted/Unknown intent can allow native fallback after 30 seconds
from its successful immutable arm timestamp and positive current-helper quiescence.
Armed, in-flight, uncertain cleanup and missing timestamp remain fenced. No timeout
cancels or consumes the queued item: physical phase, IDs, logical status and the
programmatic fence remain. The old queued marker can still run later and cause an
extra turn; this is the explicit engineering limit documented in TRUST-POLICY.

The writer atomically claims every applicable origin/cohort blocker with an exact
reservation/owner/ticket and bounded member count. Coverage is a componentwise
high-water over the seven frontier scalars, comparing each sequence/offset pair in
publication order. Disappeared old components remain covered; a genuinely newer
canonical frontier can earn one new native effect. Submitted, Unknown, generic
error, cancellation, crash and missing callback retain coverage. Only typed
pre-submit NoEffect restores the exact prior coverage for the full matching group.
A stale/partial completion cannot release a newer claim. Completion has its own
bounded noncancelled metadata budget; failed release remains spent.

Reserved lazy p0 work without an ACK request exposed a reachability gap: after the
queue reservation, ordinary discovery omitted that unchanged frontier entirely.
The new internal `programmatic_fallback_due` is read-only selection evidence from
the same pure predicate used by the final writer claim. Discovery and reservation
use an explicit decision clock. The existing wake lane preserves spacing/backoff,
leases, idle checks, final lazy owed checks, Composer verification and closed-loop
unreceived behavior. No wake_work row, frontier, publication or contact is erased or
fabricated to make the retry possible. Empty native-blocker ledgers avoid a binding
census; retained cohorts are resolved through adapter policy, without process/helper
I/O in discovery. Qualified new EndFirst may fulfill purpose without qualifying a
matched queue-admission baseline.

The sender projection can report `wake=queue_unreceived`; the new marker is 16 bytes
inside the existing field. Its explanation lives once in the skill. This is a
diagnostic, never receipt evidence or a fabricated native outcome. No other new
model-visible delivery framing or wake content is added.

## Release routing and isolation

The candidate's existing `doctor --json` resolves the canonical node under the
original operator environment before wake isolation. Its private output is bounded
at 256 KiB and its execution at 5 seconds (plus bounded owned-process cleanup).
A bounded read of node/settings.json accepts an exact boolean; absent file/key is
false. Invalid, ambiguous duplicate-key, unreadable or unresolved configuration
blocks automatic mode selection instead of silently skipping opted-in proof.
Unrelated doctor health warnings are not a routing failure when its canonical node
is valid. Explicit `--queue-proof` bypasses discovery and forces only the owned
qualification settings before daemon start; operator config is never copied,
written or restarted. Effective Linux proof is setting-on OR explicit flag.
Non-Linux ordinary mode stays native and refuses the explicit flag before work.

The direct unflagged wake driver keeps its owned instance off; the release wrapper
forwards the flag when the selected install opted in. Linux bounded transport
observation remains separate: actual Accepted uses 15 seconds, native uses 6,
Unknown fails, and required queue proof demands Accepted. Existing UI/resume gates
remain unchanged. The owned-setting test checks the value before daemon start and
preserves the fake operator settings byte-for-byte.

## Verification

- Causal lazy-frontier RED: a real p0 lazy publication without ACK was queued,
  completed normally and left unread; at 30 seconds discovery returned 0, expected 1.
  `fallback-lazy-red.log` retains that behavioral failure.
- Final affected run: 248 passed in 19.283 seconds; 132-second compile. Includes all
  five new lazy controls: exact time boundary, actual default-off runtime and native
  dispatcher, NoEffect retry, newer publication, failed metadata release, stale
  read/threshold selection, Armed/quarantine denial and Unknown eligibility.
  The actual ScheduledStore/WakeRunner path first refuses retained spacing, then
  reaches guarded native once. No test deletes wake_work in these controls.
- Schema/settings/discovery-cost/contracts run: 210 passed in 19.878 seconds;
  0.03-second compile. Includes historical candidate refusal, final fresh/upgrade
  schema audit, default-off parsing, external candidate shape and native discovery
  cost controls. Counts are separate test executions, not a unique combined count.
- Scoped process and temp checks were clean immediately after both Rust runs.
  Retained repeat-check logs show no processes and no remaining temp ledger after
  the initial sweeps. Run IDs are in the JSON manifest.
- Offline wake driver: 298 passed in 102.904 seconds. Release routing/probe tests:
  14 passed in 0.157 seconds, including duplicate-key causal RED then GREEN.
  Scoped process checks are clean; these Python runs have no temp ledger.
- All-target/all-feature Clippy passed in 67 seconds; default-feature guard,
  formatting, harness neutrality, wire compatibility, docs, Python 3.9 syntax and
  whitespace checks passed. The two post-test lint edits only annotate the explicit
  eight-argument discovery boundary and move a helper before its test module.

Initial fixture failures are preserved as intermediate logs: mismatched immutable
priority sources, adding a trigger then reopening through strict schema audit, and
an incorrectly escaped Python test string. They were corrected in test setup; no
schema audit or immutable-source guard was relaxed. Earlier store/host/default
behavioral RED/GREEN logs remain separate. Failed compilation/fixture logs are not
presented as behavioral RED evidence.

## Scope and retained evidence

No new authenticated agents, real-profile helper experiment, installation or release
ran in this pass. Historical 72 unique live rows remain tied to their original SHAs;
S4 populated-copy qualification and accepted B1/N1/S2-S5 fixes remain separately
attributed in the earlier qualification artifacts. The 15fd full-suite failure also
remains history; it does not qualify this tree. Final unfiltered suite and fresh
scoped cleanup follow review settlement. No five-minute or incremental speed-budget
pass is claimed; performance remains the separately assigned nonblocking lane.

`qualification-native-fallback-default.json` pins current source and raw log hashes.
Raw logs remain in the owned worktree's `target/queue-qualification/` for reviewers.
The warm worktree/build remains needed until integration; cleanup follows confirmed
merge, under the existing ownership rules.
