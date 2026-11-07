# Final review, cluster 8 (integrations-hermes), epic ht-ma6i

## Scope and evidence

Reviewed the three slices (`integrations/hermes/README.md`, `__init__.py`, `test_bridge.py`) at head 1b058307490f29ece8bb4e3fe7a862f8356d992f against base cf13ed76c1c2871b321ad290d03229cb785c400e and the spec `docs/superpowers/runs/2026-11-04-graph-clone-identity/2026-11-04-graph-clone-identity-design.md`. Both SHAs verified with `git rev-parse --verify`. The caller checkout stayed clean at the reviewed head throughout; the only caller write besides this artifact is the authorized ledger binding.

Opened beyond the cluster, for the named B1 risk only: `TRUST-POLICY.md` (the Hermes limit), the prior review `reviews/inv-4ebdbb23-.../final-review-inv-4ebdbb23-....cluster-3.md`, `run.md`, `bd show ht-ma6i.23`, the source design `herdr-graph/docs/reviews/2026-10-29-seat-review-H2-identity-design.md`, herdr-graph at 0dbe477, and upstream Hermes source read through `gh api`.

Run here:

- `test_bridge.py` at head, with TMPDIR and HOME in a scratch directory and the Herdr variables unset: 50 tests, OK.
- Mutation in a scratch copy, `HERDR_THREADS_CONTINUITY_KEY` removed from `PANE_ENV`: `test_gateway_child_takes_the_session_continuity_key_not_the_gateway_key` fails with `'gateway-key' != 'clone-1'`. The entry is live code and the test pins it.

No live Hermes ran; `hermes` is not installed here. Upstream behaviour below is source-read, not executed.

## Requirement baseline trace

- B1 (continuity key), classic Hermes: `pane_env()` returns None, `Popen(env=None)` inherits the process environment, and the hook child gets the key. Delivered as far as this cluster goes. From the hook onward the path is the seam reviewer's.
- B1, Hermes gateway: the key cannot reach the hook child. `pane_env()` removes every `PANE_ENV` name from the gateway's environment and restores only what `bound_client_env()` holds, and upstream drops the name on both sides. At v0.22.7, `collect_client_env` and `accept_client_env` go through `_kept`, which requires `_travels`, which rejects any name containing a `_SECRET_SUBSTRINGS` entry, `KEY` first, "even when listed" (`agent/session_client_env.py:48-54`, `tools/code_execution_env.py:29-35`).
- B1, whether that gateway gap fails the row: the source text of D1 is "The graph launches every clone agent with" the key, so the row's producer is the graph. herdr-graph at 0dbe477 has `enum Harness { Shell, Claude, Codex }` (`src/model/harness.rs:8`), and outside `third_party/` and `research/` its tree does not name Hermes. No supported producer launches a keyed Hermes clone, so the gateway is not a condition this request promises, and the limit is a boundary of the threads spec's wider carrier statement. I do not call B1 undelivered on this ground, and no Must fix follows from it.
- That is a judgment on a row whose text names no harness. The prior final review reached the opposite conclusion and required a filter-passing name or a human scope change row. At this head the baseline still lists "Human scope changes: none", and `run.md` parks the item as a degraded verdict with "no human scope row". The Deferred OK section carries it so the human sees it, and a graph that adds a Hermes harness reopens it.
- The prior review's other objection, a false document telling a launcher to list the key, is resolved. README, `src/harness/hermes.rs` and the setup golden no longer promise delivery through the passthrough, and `TRUST-POLICY.md:2254-2258` records the limit in the same commit (54d12a213).
- B2, B3 and B4: this cluster makes no close, retirement or Herdr change, so nothing here bears on them. B2's and B3's paths are the seam reviewer's.

## Bridge analysis

Sound on the paths checked:

- The gateway's own key never reaches another pane's hook child. It is stripped by the `key not in PANE_ENV` comprehension, the new test's second half asserts it is absent for a session whose client env has none, and the mutation above shows the entry is what enforces this.
- A client env holding only a key is refused. The `opaque(...)` guard requires a pane, a name or a full terminal pair first, so a key alone seats nothing.
- An empty or malformed key string passes the `isinstance(..., str)` copy and reaches the child. Spec 4.2 has the client treat an invalid value as absent, which is the Rust side and leaves this cluster.
- The subagent `pre_llm_call` path returns before `pane_env()`, so it starts no child and presents no key. Forwarded `subagent_start` and `subagent_stop` events do carry the environment; the rule that they do not present the key is in the Rust hook, which is the seam reviewer's.
- The fixture's `OwnedChild` allowlist and the stub's `pane` capture gained the variable, so the assertions observe the real child environment rather than a mock.
- The bridge is embedded with `include_bytes!` and `status` compares its digest (`src/harness/hermes/assets.rs:22,908`), so this change marks an installed bridge not current.
- No model-visible text changes in this cluster, and the bridge stays adapter-owned.

## Upstream facts checked

- Tool subprocesses on a gateway keep the gateway's own key. `_make_run_env` starts from `toolchain_environ(os.environ.copy())`. That function removes only names in the session's rule list, and `names` is itself filtered by `_travels`, so a `KEY` name is neither removed nor replaced (v0.22.7). A session's tool subprocess therefore runs with its client's `HERDR_PANE_ID` and the gateway's key.
- `agent/session_client_env.py` is absent at tags v0.22.1 and v0.22.2 (404, and `agent/` at v0.22.1 lists no such file). It is present at v0.22.3 through v0.22.7 with one byte size throughout; only v0.22.7's content was read.

## Verdict
ready

## Must fix before landing
- none

## Untested scope
- No live Hermes run exercises any of this, and `hermes` is not installed here: the gateway filter and the tool subprocess environment are source-read at v0.22.7 only, and the README's and TRUST-POLICY's limit is not pinned by any test, so a Hermes that relaxes or tightens the filter goes unnoticed.
- The bridge has no test that a classic Hermes child inherits the key (`pane_env()` returning None); `test_classic_cli_child_inherits_the_process_pane` checks only the pane variables, and the behaviour is plain inheritance.
- No test takes a keyed bridge child through the real `herdr-threads hook hermes` binary: `tests/integration/hermes_enrollment_smoke.rs` and `tests/fixtures/hermes` do not name the key. Whether the integration clusters cover the hook's handling of a classic Hermes key, including SessionStart enrollment, is the seam reviewer's to confirm.

## Deferred OK
- B1 (Hermes gateway), recorded for the human: a keyed Hermes gateway session cannot reattach by key, so a restored pane gets a new seat or stays seatless, and there is no human scope change row; it is not a Must fix here because D1's producer is the graph and `herdr-graph` `src/model/harness.rs:8` has no Hermes harness, and if the human reads B1 as covering every threads harness, or the graph adds Hermes, the fix is a variable name that passes `_SECRET_SUBSTRINGS`, coordinated with the graph session, since Hermes documents no other route for it.
- `integrations/hermes/README.md:67-68` says "the session runs keyless" and `TRUST-POLICY.md:2256-2258` says a keyed clone "behaves as a keyless one", but on a gateway started from a keyed pane the tool subprocesses keep the key (see Upstream facts checked), so a model's plain `seat resolve` or `me init` in another pane presents that pane's id with the first pane's key through `own_pane_continuity_key` (`src/cli/site_capability.rs:108`), and per the prior review's trace a reserved unbound key binds to the wrong seat; it needs a keyed Hermes launch that has no producer today, and the fix is one sentence in the limit stating this outcome, or the filter-passing name above, which closes it.
- `integrations/hermes/README.md:67` and `TRUST-POLICY.md:2254` date the filter "from 0.22.1", but `agent/session_client_env.py` first appears at tag v0.22.3, so on 0.22.1 and 0.22.2 a gateway session has no client env at all and runs no hook; the outcome for the key is the same, and only the version in the wording is off.
- `integrations/hermes/README.md:70` "The pane's `hermes` client sends them" now follows the inserted key paragraph, so "them" no longer points at the two names; moving the key paragraph after the passage that ends "Start a new `hermes` session after changing the setting" repairs it.
- `integrations/hermes/__init__.py:37` explains why the key stays empty but not why it stays listed, which reads as a dead entry when it is what scrubs the gateway's own key from other panes' hook children; one clause fixes it, and the test catches a removal meanwhile.
- `test_gateway_child_takes_the_session_continuity_key_not_the_gateway_key` (`integrations/hermes/test_bridge.py:480`) still stubs a client env key that Hermes v0.22.7 cannot produce, as task 15 recorded; the half that asserts the gateway's key is dropped is real production behaviour, so the test is worth keeping, and only its first half and its name overstate.
- A stale installed bridge has the old `PANE_ENV`, so a gateway started from a keyed pane would pass its key to other panes' hook children until setup refreshes it; `bridge_current` reports that state, and no keyed Hermes launch has a producer today.
