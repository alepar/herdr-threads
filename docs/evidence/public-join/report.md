# Public CLI self-join — frozen feature handoff

Implemented on `feature/public-join` in `.worktrees/public-join`, from actual main
`4f7cad2ddadf0f3e9bf917a36917624821b3be77` (schema 25). Main owns merge,
release, local installation, and feature-tab closure. No upstream branches were
pushed; main's sources and index were not edited.

## User-visible contract

Discover with `herdr-threads thread list --all --search TEXT`; enroll with
`herdr-threads join THREAD` (same exact ID/name resolver as other thread commands).
The wire operation is `join`, its result is `joined` with the canonical thread ID,
and the daemon advertises `thread.join_v1`. New and retried CLI joins require that
capability before submitting. The durable journal freezes the resolved ID and exact
cooperative claim; response/output loss retains a retryable intent.

- Active threads in the caller's instance allow voluntary self-join without an invite.
- Pending ordinary invitations require explicit acceptance or exact rejection first.
  Pending service-required invitations require exact-revision `accept-required`.
- Already joined is a settled no-op, with no duplicate event or interval. Archived
  threads refuse a fresh join, including a fresh operation by a joined caller;
  a member/service owner must reopen. Historical completed retries remain readable.
- Leaving and rejoining creates a fresh monotonic membership episode. Rejected or
  released-and-cancelled invitations retain their evidence; join invents no acceptance.
- The deciding daemon transaction validates the existing canonical occupant, instance,
  held/resolved/nonretired seat and cooperative permit. Subagents are refused.
- A native-authored info event records `action:join`, seat, binding generation and
  existing `cooperative_top_level`/`operator_human` provenance at the same decision
  sequence as its interval. Human read/follow displays the joined notice.
- Join creates no binding, invitation, acceptance or ACK, and leaves frozen historical
  recipients/obligations unchanged. It invalidates an obsolete prepared send snapshot.
  An accepted service requirement remains accepted and still prevents leave.

## Implementation and integration

CLI parser/selector, semantic journal/retry result classification, capability gate,
wire types, daemon dispatch, store mutation/permit routing, operation-status result ID,
control transaction, event author helper and human transcript rendering are connected.
README, public CLI reference, embedded skill and TRUST-POLICY document the operation.
The source inventory contains hashes for the changed source/test/reference files.

No migration, schema-version change, historical migration edit, dependency or lockfile
change. No dependencies on unlanded lazy26, handoff27, adapters or lazy latency work.
Main need not reconcile a migration number for this branch.

## Verification

The clean baseline lifecycle test passed before implementation. Test-first failures
showed `unrecognized subcommand 'join'` and wire `unknown variant join`. A later
attribution regression observed null native author fields before the attributed helper.
Independent review identified the human transcript filter; its regression failed with
empty output before the fix and passes afterward.

Final commands (logs beside this report):

| Check | Result |
| --- | --- |
| `nice cargo test --locked --all-features --lib public_join` | 12 passed |
| `nice cargo test --locked --all-features --lib store::control::tests::` | 93 passed |
| `nice cargo test --locked --all-features --lib cli::commands::tests::` | 60 passed |
| `nice cargo test --locked --all-features --lib cli::irc::tests::` | 17 passed |
| `nice cargo test --locked --all-features --lib cli::cooperative_tests::` | 40 passed outside sandbox |
| `nice cargo test --locked --all-features --lib protocol::capabilities::tests::` | 14 passed outside sandbox |
| `nice cargo clippy --locked --all-targets --all-features -- -D warnings` | exit 0; 57.328 seconds |
| `nice scripts/check-default-features` | exit 0; 39.271 seconds |
| `nice cargo build --locked --all-features` | exit 0; 18.492 seconds |
| built binary `join --help` | exit 0; correct command and selector guidance |
| `cargo fmt --check`; `git diff --check` | exit 0 |
| `scripts/check-no-leaked-processes --root <isolated worktree> --run-id 8a8e3d83-d620-4e6f-8f6e-82ebca55c8ec` | exit 0; no leaked test processes |

Six groups total 236 passing test executions (233 distinct tests; three new CLI tests
also appear in their adjacent groups). The 12 new tests cover authority refusals,
rejoin/replay/restart, event attribution, human provenance, invitation retention,
pending/accepted required consent, receipt frontiers, prepared-send invalidation,
actual CLI journal recovery against the real store, and compatibility new/retry gates.

## Independent review

Native reviewer agent `/root/join_review` reviewed the final production diff read-only.
The confirmed P2 transcript defect was reproduced and fixed. Final verdict:
"Ready from independent code review, subject to the separately running required checks.
No remaining important correctness issues found." Those checks subsequently passed.
The reviewer did not independently execute tests or live Herdr integration.

## Honest limitations

The full suite was intentionally not run under the repository's focused-test policy
while the flakiness work remains unlanded. No live Claude/Codex harness, installed
shared daemon, or real-user config mutation was exercised. Response-loss coverage
uses the actual CLI journal/retry layer against the real store, rather than the full
socket transport; daemon capability dispatch and existing socket regressions pass.

The first broader cooperative run had 25 passes and 15 sandbox socket-binding failures;
its failing log is retained. The permitted isolated socket rerun passed all 40 tests.
Sandboxed `nice` emitted a priority-setting refusal, but child checks executed and
returned success; outside-sandbox socket tests successfully used `nice`. Build/lint
measurements are from this isolated cache, not a controlled contention benchmark.

No private Herdr server or helper daemon was started. Socket fixture threads ended
with their test processes; every command session is complete and cleanup passed.
No coordination thread join, inbox/check-in, ACK, push, stash or shared server stop
was performed. A direct Herdr prompt is reserved for the final merge-ready milestone.
