# Interactive Codex side conversations (ht-i9a7)

Measured 2026-10-17 on installed Codex 0.163.0, starting from 7962bc703
(ht-36fl). Version is historical evidence, not an admission requirement.

Two isolated runs used a private Herdr server and socket, a scratch working
directory and CODEX_HOME, public `setup codex --with-permissions`, and a private
mode-0600 copy of auth.json. The installed Codex ran with `--no-daemon`,
`--approve-for-me`, `--dangerously-bypass-hook-trust` (only the captured fixture
hooks), and shell snapshots disabled. No reporter session or production store
was touched. Both runs reported `CLEANUP []`; the owned workspace, daemon,
server, scratch configuration and credential copy were removed. Raw redacted
controller evidence remains at `/tmp/ht-i9a7-evidence` and
`/tmp/ht-i9a7-evidence-2` on the reproducing machine.

## Before the change

1. The main conversation's `SessionStart` uses `source: startup`, its own native
   session ID and a rollout path. Its binding is `top_level`, connected, with
   `cooperative_top_level` observation provenance.
2. Entering `/side` fires `SessionStart` with `source: fork`, a distinct native
   session ID, `transcript_path: null`, and no agent ID or agent type. The
   [sanitized native payload](../../../tests/fixtures/codex-0.163.0-side/session-start-fork.json)
   retains this shape. The hook rejects it:

   ```text
   herdr-threads hook: unsupported hook payload: Invalid
   ```

3. The side's `UserPromptSubmit` and Bash `PreToolUse` events use its own session
   ID, no agent ID/type, and a null transcript path. Tools receive
   `CODEX_SESSION_ID == CODEX_THREAD_ID == <side-native-session>`; neither is the
   main conversation's ID. No SubagentStart event accompanies `/side`.
4. Side `inbox` and `send side-test --body SIDE-SEND` both succeed on this base,
   including a send after `sleep 8`. They create/use a distinct top-level
   `cooperative_env_session` binding and the side's own deterministic execution.
   Consequently the originally reported read/send refusal was **not reproduced**
   on ht-36fl. However the side binding subsequently becomes disconnected with
   reason `superseded`, while the main stays connected. This loses connected
   receipt authority; successful ordinary sends alone do not establish it.
5. Closing the side with Ctrl-C and prompting the main produces UserPromptSubmit,
   not another SessionStart. Treating `fork` as an ordinary startup would thus
   disconnect the parent without a return lifecycle event to restore it.

## Native parent communication

The side was explicitly asked to call native `collaboration.send_message` once,
with target `/root` and body `SIDE-NATIVE-RELAY`. The UI printed
`Interacted with /root`, the call returned an empty result, and the same side
conversation received a native message from `/root` containing that marker.
After closing the side, the main explicitly confirmed that no such native input
arrived. Its `herdr-threads inbox` was empty too.

This action is native collaboration inside the side's own agent tree, not an
ht send and not a route to the main conversation. No automatic bridge should
be inferred from its UI label. An ht send records the side's sender binding and reaches the thread's other
seats. The author seat is excluded from the authored audience
(`src/store/messages.rs`), so it does **not** automatically enter the main
sibling's inbox. The main can explicitly read that shared thread's history;
for automatic delivery a peer on another seat must relay back. A direct send
to one's own seat is refused. Adding a session-addressed relay would require
a separate delivery/audience design; this fix does not change those semantics.

## Regression checks

The focused integration test `codex_side_fork_reads_sends_and_settles_with_its_own_binding`
failed before implementation at the exact fork refusal above. It uses the production
`hook-event codex SessionStart` entrypoint and asserts distinct execution IDs,
`cooperative_top_level` provenance, both connected bindings, ordinary side send
attribution, own text-inbox receipt settlement, unchanged main cursor, main's own
later display, and refusal of mismatched native session/thread writes. A genuine
main UserPromptSubmit remains Current and leaves the side connected.

Store/worker tests cover retained concurrency through ordinary sibling lifecycle
replacement and own resume, exclusion from same-process liveness competition,
stale worker verdict rejection, and normal process-exit disconnection. A schema-2
upgrade test preserves an existing binding with the bit off and checks the
older production schema reader refuses the newer store. Canonical check-in tests
cover borrowed execution refusal, expected-generation refusal, exact fork replay,
and ignored parent-lineage hints; existing Hermes lineage tests retain their
successor behavior.

## Native verification after implementation

The same private-server driver ran again with the fixed candidate
(`/tmp/ht-i9a7-evidence-final`). The fork's SessionStart returned context without
an error. After the side's own inbox call, `sleep 8`, and successful `SIDE-SEND`,
the canonical store held both main and side connected with
`cooperative_top_level` provenance, distinct native session IDs, and concurrency
bits 0 and 1 respectively. Ctrl-C close emitted no SessionEnd, so its binding
remained connected until the whole harness process exited at cleanup; no
UI-based disconnection is inferred. Closing the side and prompting main produced a
successful `MAIN-RETURN-SEND`; both bindings remained connected. No SubagentStart
was observed for `/side`. All captured hook stderr files were empty. Cleanup
again reported `CLEANUP []`; the private server and scratch credential/config
root were removed. Later review tightened the independent-fork lineage-hint
check and the public context encoder; their regressions are covered by the
focused library tests.

## Focused validation commands and results

All commands below exited 0. No full suite was run; release qualification remains
with the coordinator.

| Command | Result |
| --- | --- |
| `nice cargo test --locked --all-features --lib independent_fork -- --nocapture` | 4 passed |
| `nice cargo test --locked --all-features --lib harness::codex_tests` | 26 passed, 1 pre-existing ignored installed-binary observation |
| `nice cargo test --locked --all-features --lib harness::bridge_tests` | 49 passed |
| `nice cargo test --locked --all-features --lib store::bindings::tests` | 81 passed |
| `nice cargo test --locked --all-features --lib liveness_worker_tests` | 18 passed |
| `nice cargo test --locked --all-features --lib legacy_child_execution` | 2 passed |
| `nice cargo test --locked --all-features --lib hermes_lineage` | 7 passed |
| `nice cargo test --locked --all-features --lib concurrent_session_delta` | 1 passed |
| `nice cargo test --locked --all-features --lib enrollment_policy_maps_decoded_events_to_kinds` | 1 passed |
| `nice cargo test --locked --all-features --test integration codex_side_fork_reads_sends_and_settles_with_its_own_binding -- --nocapture` | 1 passed |
| `nice cargo test --locked --all-features --test integration config2_codex_embedded_and_managed_daemon` | 1 passed |
| `nice cargo test --locked --all-features --test integration config3_hermes_interactive_and_a_one_shot_in_a_claude_pane` | 1 passed |

The private-server integration tests ran outside the sandbox because the sandbox
refuses their private Herdr socket. The first added regression failed before the
fix with the captured native fork error. Review regressions independently failed
on late lineage linking and the public encoder's fork refusal, then passed after
those fixes. The CLI-first fixture uses ordinary text inbox; JSON inbox remains
read-only and does not enroll that previously unknown binding.

The required `nice cargo clippy --locked --all-targets --all-features -- -D warnings`,
`scripts/check-harness-neutral`, `cargo fmt --check`, `git diff --check` and
`nice scripts/check-default-features` also passed. Independent read-only review
found no remaining significant issues after the two review fixes.
