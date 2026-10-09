# Claude mod delivery: live stress evidence (ht-j16.9)

Status: LIVE at the final code SHA `573841ed`: all 14 scenarios passed 3/3 (51 require-ACK messages and 3 lazy
messages, every one acked or `displayed`; 1 native wake prompt in the run). Earlier runs are kept below: the
targeted re-run at `73a75cbf`, the full re-run at `b562d1e4`, and the first run, which found D1-D3.

## Final full run (573841ed)

- Source: `super-auto/claude-mod-inbound-delivery` at `573841ed` (includes main `88f5c69f` and the ht-j16.34
  attention narrowing). Claude Code 2.1.295. Command: `python3 tests/native/claude_mod/stress.py --iterations 3
  --settle 60`. Wall clock 20.8 min.
- [`summary.json`](summary.json) and [`live/`](live/) hold this run. Leak check: "no leaked test processes".

## Final targeted re-run (73a75cbf)

- Command: `python3 tests/native/claude_mod/stress.py --iterations 3 --settle 60 --scenarios
  reload_mid_turn,reload,clear_rebind,idle_submit`, Claude Code 2.1.295, 4.8 min. Results and files:
  [`final-subset/`](final-subset/) (`summary.json` and `live/`). Every message acked, 1 total native prompt (per `native_prompts_total` in the summary), no
  duplicate ledger entries. Leak check: "no leaked test processes".
- `73a75cbf` changes only the mod: a successor no longer records ids its session record already holds as
  delivered. The other 10 scenarios were not re-run; their code paths are unchanged since `b562d1e4` except for
  that function.

## Full re-run (b562d1e4, after ht-j16.28-.33 and dc5b2f6c)

- Source SHA: `b562d1e4757534323ee500b40e864e2274e145b1` (`super-auto/claude-mod-inbound-delivery`), Claude Code
  2.1.295, same driver and profile setup as below. Command:
  `python3 tests/native/claude_mod/stress.py --iterations 3 --settle 60` (no fixed seed). Wall clock 16.0 min.
  51 require-ACK messages and 3 lazy messages were sent; every one ended acked or `displayed`.
- [`summary.json`](summary.json) and [`live/`](live/) now hold this re-run. The first run's `live/` files that
  the D1-D3 sections cite are in git history at `2c3120b1`; [`run1-excerpts/`](run1-excerpts/) is unchanged.
- Leak check: `scripts/check-no-leaked-processes --run-id $HT_LEAK_RUN_ID` printed "no leaked test processes".

| Scenario | First run (c6caf381) | Re-run (b562d1e4) |
|---|---|---|
| clear_rebind (D1) | 0/3 | 3/3 |
| reload_mid_turn (D2) | 1/3 | 2/3 |
| denied_tool_with_pending_context (D3) | 0/3 | 3/3 |
| the other 11 scenarios | 3/3 | 3/3 |

Native wake prompts across the whole re-run: 1.

**The remaining `reload_mid_turn` failure is a ledger entry, not a second delivery.** In iteration 1,
`mKqWl1pGt` was submitted once (`submit` at 1791559138267). The pre-reload mod recorded it `delivered` and acked
it (`already_settled`). After the reload, the new mod instance found the predecessor's persisted `submitting`
record, matched the submitted turn and recorded the id `delivered` again (`predecessor_submit`, 1791559140683);
its ack came back `already_settled`. The pane (`live/pane-s-reload-mid-turn-reload_mid_turn-1.txt`) shows the
message once. The driver's "duplicate delivered" check counts ledger entries, so it fails the iteration. The
cause is that a submit that resolved just before dispose leaves its `submitting` record behind; the successor
could skip ids its session record already holds as delivered.

## First run (found D1-D3)

Status at that SHA: 11 scenarios passed every iteration. 3 scenarios failed for product reasons, recorded below
as defects D1-D3. The driver was not changed to hide any of them.

## Run

- Source SHA: `c6caf38145435419d190fb945511cf2bfbaf89f5` (branch `live-stress-driver`, cut from
  `super-auto/claude-mod-inbound-delivery` at `504d2515`). The run used the herdr-threads binary built from that
  checkout (`nice cargo build --locked --all-features`) and the mod that binary's `setup claude` installed.
- Claude Code: 2.1.295, Opus 5.5 at medium effort (the profile's default), macOS (Darwin 25.6.0), tmux 3.7c.
- Command: `python3 tests/native/claude_mod/stress.py --iterations 3 --settle 60 --seed 2`. Wall clock 23.1 min.
  51 require-ACK messages and 3 lazy messages were sent.
- Raw result: [`summary.json`](summary.json) holds per iteration the ids, the delivery path from the ledger, the
  daemon receipt states, the violations and a pane tail. [`live/`](live/) holds the merged mod ledgers (one per
  Claude process), the scenario hook log (`hooks.jsonl`), every pane capture, the stand-in Herdr's
  `agent.prompt` calls and the final receipts.
- A first run at `81dc24d2` (the same driver without the "acked outside the mod" check) gave the same picture:
  [`run1-excerpts/`](run1-excerpts/) keeps its summary and the files for D1-D3.
- Leak check: `scripts/check-no-leaked-processes --run-id $HT_LEAK_RUN_ID` printed "No leaked test processes".

### Setup the driver uses

- **Profile.** The signed-in profile is used in place as `CLAUDE_CONFIG_DIR`, with the real `HOME` (its login is
  bound to that path in the Keychain, so a copy is not signed in) and `DISABLE_AUTOUPDATER=1`. The profile's
  `settings.json` is never written.
- **Hooks and mod.** `setup claude` runs into a scratch config dir. The driver passes the hooks it generates, plus
  its own scenario hooks (Stop, UserPromptSubmit and SessionStart, which log and block on marker files), with
  `claude --settings <file>`. It passes the mod with `CLAUDE_CODE_PLUGIN_DIRS`.
- **herdr-threads.** A private state dir and daemon. A stand-in Herdr endpoint, with the protocol of
  `tests/integration/sweep.rs` `host_reply`, serves two fake panes: `w1:p1` for the sender and `w1:p2` for Claude.
  It records `agent.prompt` calls (native wakes) and never types them. The built binary comes first on the
  session's `PATH`, so any `herdr-threads` the model runs reaches the isolated instance.
- **Bootstrap.** The seats, thread, invite and accept are set up like the `mod_delivery` Rig. The accept waits
  for the first session's SessionStart check-in.
- **tmux.** A private server, `tmux -L ht-mod-<uuid>`. Each scenario gets one Claude process, or two for
  `resume_keeps_set`, all with the same pane id.

## Result (3 iterations)

| Scenario | Pass | Delivery path seen | What was exercised |
|---|---|---|---|
| `busy_context` | 3/3 | context | Message sent while `sleep 8` ran; it rode the Bash result. |
| `idle_submit` | 3/3 | submit | Idle session; one framed `$.prompt.submit` started a turn. |
| `lazy_append` | 3/3 | append | Lazy message; `$.session.append`, no turn, `lazy_recipients.state = displayed`. |
| `queued_user_prompt` | 3/3 | context | User prompt queued mid-turn (`BRAVO`), message sent too. |
| `stop_hook_continuation` | 3/3 | submit | The Stop hook blocked once (3 blocked Stops in the hook log); the submit came only after the continued turn completed. |
| `esc_interrupt_hold` | 3/3 | submit | Esc during `sleep 20`. The ledger shows `held post_abort`, and the submit came only after the next user turn completed. |
| `permission_dialog` | 3/3 | context | Message sent while the Write permission dialog was open, then approved; it rode the answered result. |
| `clear_rebind` | **0/3** | submit (pre-clear only) | **D1** |
| `resume_keeps_set` | 3/3 | submit | Pre-message, process exit, `claude --continue`, post-message. |
| `reload` | 3/3 | submit | `/reload-plugins` between two idle messages. |
| `reload_mid_turn` | **1/3** | context or submit | **D2** |
| `kill_watch_fallback_and_restream` | 3/3 | context | The `watch` child was SIGKILLed after the mod had received a message. The daemon re-streamed it on reconnect, and the mod received it twice but delivered and acked it once. |
| `user_prompt_submit_block_drop` | 3/3 | submit | The UserPromptSubmit hook blocked the submit (`refused drop:UserPromptSubmit operation blocked by hook`). After the 30 s backoff the submit went through. |
| `denied_tool_with_pending_context` | **0/3** | submit | **D3** |

Every require-ACK receipt ended `acked` and every lazy row `displayed`. No message was lost, and the ledger
never showed a duplicate `delivered` or a duplicate settling ack. The stand-in recorded 7 native `agent.prompt`
wakes: at session starts before the mod connected, and while the mod was locked out in D1.

## Product defects

### D1. After `/clear` the mod's `watch` uses the pre-clear session id and is refused until the next session end

- **Scenario:** `clear_rebind`, 3/3 iterations in both runs. The mod never reconnects after `/clear`.
  - Iteration 0: the post-clear message stays `pending` (not settled in 60 s).
  - Iterations 1 and 2: the messages were acked outside the mod. With the channel down, the daemon fell back to
    native attention, and the model ran `herdr-threads inbox`.
- **Ledger** (`live/ledger-s-clear-rebind.jsonl`): the `/clear` came at 1791552581155 (SessionStart `clear`, new
  session `3718ea1e`).
  ```
  1791552576198 acked ['mWGjgx2op'] submit settled         # pre-clear message, fine
  1791552581822 restart exit:0                              # stream closed (binding changed)
  1791552583540 restart exit:2                              # refused, then every 3, 6, 10, 30 s ...
  ... exit:2 every 30 s until the session ended
  ```
- **Root cause, confirmed by a probe** (dry run, same build). After `/clear`:
  - `watch --session <pre-clear id>` printed `{"kind":"status","state":"refused","reason":"session_mismatch","exit":2}`.
  - `watch --session <new id>` connected.

  `register.js` `refresh()` re-reads `$.session.id()` once, on the tick right after the `session.end` handler
  returns. On 2.1.295 that still returns the old id. Restarts after an exit-2 refusal reuse the stale `S.sid` and
  never re-read it.
- **Knock-on in later iterations.** Each further `/clear` triggers one more `refresh()`, which picks up the
  previous clear's id. That id is still bound for a moment, so the mod connects briefly. It submits the queued
  messages, then gets `ack:stale_generation` and is locked out again. In `run1-excerpts/ledger-s-clear-rebind.jsonl`
  at 1791551294235, two messages are delivered by submit and refused `ack:stale_generation`. The model then
  received them again through `herdr-threads inbox` (pane `run1-excerpts/pane-s-clear-rebind-clear_rebind-1.txt`).
  The same messages were presented twice.
- **Process exit.** On exit, `refresh()` finally reads a bound id. The mod connects and submits into a dying
  session (`refused drop:error:HooksError: ... no session is bound in this process`).

### D2. A reload at the turn boundary while a submit is in flight submits the same message twice

- **Scenario:** `reload_mid_turn`. Failed 2/3 in this run and 1/3 in run 1.
- **Engine behaviour.** Claude Code applies both `/reload-plugins` and a module save only at the turn boundary,
  so the reload lands right as the turn ends. The scenario saves the installed `register.js` mid-turn.
- **What happens.** At turn end the old core submits the message it received mid-turn. The reload disposes that
  core before the submit resolves, so no `delivered` is recorded. The daemon re-streams the unacked id. The new
  core submits it again while the first submit's turn is still open.
- **Ledger and hook log** (`live/ledger-s-reload-mid-turn.jsonl`, `live/hooks.jsonl`, iteration 1):
  ```
  1791553303489 received mEO6N2s9R
  1791553304634 Stop                                   (turn ends)
  1791553304653 submit   mEO6N2s9R                     (old core)
  1791553304700 UserPromptSubmit [herdr-threads] ...   (first submitted turn opens)
  1791553304767 received mEO6N2s9R                     (new core, re-streamed)
  1791553304767 submit   mEO6N2s9R                     (second submit, inside the open turn)
  1791553307039 Stop
  1791553307089 UserPromptSubmit [herdr-threads] ...   (the same message again)
  1791553307115 delivered mEO6N2s9R submit / 1791553307139 acked settled
  ```
- **In the TUI** (`run1-excerpts/pane-s-reload-mid-turn-reload_mid_turn-1.txt`): the framed message appears twice.
  The model answers the second time with "That's the same message (mUCRIWheE) I already answered".
- **Why the ledger alone misses it.** The ledger records one `delivered`, because the disposed core never logged
  its own. The driver's hook-log turn check (`check_turn_overlap`) is what catches it.

### D3. Esc at a permission dialog interrupts the turn but sets no post-abort hold

- **Scenario:** `denied_tool_with_pending_context`, 3/3 iterations in both runs.
- **What happens.** A message arrives while a Bash permission dialog is open, and the user presses Esc. The TUI
  shows `Interrupted · What should Claude do instead?`, the same interrupt as Esc during a tool. The mod
  submits about 100-150 ms later and takes over the turn the user was being asked to direct.
- **What should happen.** The post-abort hold exists to prevent exactly this: spike row 10, spec D5 rule 1.
- **Evidence.** The ledger has no `held post_abort` line. Compare `esc_interrupt_hold`, where the ledger holds
  every time.
  ```
  1791553583461 received mckEjrcSw                    (dialog open)
  Esc pressed at 1791553585459
  1791553585607 submit   mckEjrcSw
  1791553585666 UserPromptSubmit [herdr-threads] ...
  ```
  Files: `live/ledger-s-denied-tool-with-pending.jsonl`, and
  `live/pane-s-denied-tool-with-pending-denied_tool_with_pending_context-0.txt` (line 10: `Interrupted`, then
  `Prompt from the herdr-threads plugin`).
- **Probable cause.** This reject reaches the mod as a `turn.complete` with `isAborted` false. The mod's hold keys
  only on `isAborted` (`register.js` `onTurnComplete`).
- **Not affected.** The denied result itself correctly did not carry the message as context.

## Observations (not counted as failures)

- **Stale attention at each session start.** At every session start the mod submits an attention item
  (`herdr-threads: attention pending; run herdr-threads inbox`) when one is current, even after the invitation
  was accepted. The model then runs `herdr-threads inbox` and finds it empty. This costs one model turn per start.
  The spec says attention is re-sent once per watch run.
- **Ledger rewrites.** The mod's ledger is rewritten from its in-memory tail, so a reload starts the file over.
  The driver polls every 100 ms and merges the lines, so a line written and overwritten inside one poll interval
  could be missed.

## Not exercised

- **Mid-turn reload.** Claude Code defers the reload to the turn boundary (see D2).
- **In-session session switches.** `/resume <id>` and `/branch` were not run. `resume_keeps_set` restarts the
  process with `--continue`.
- **Timers.** Truncated (>8 KiB) bodies, the 120 s idle expiry of the post-abort hold, the draft hold, the 10 min
  stall cooldown and a daemon restart. The driver ends every hold with a user turn.
- **Turn variants.** Subagent tool calls, and permission modes other than `default`.
- **Real Herdr.** The endpoint is a stand-in, so native wakes are recorded, not typed. Nothing was run in the
  Desktop app or VS Code.

## Re-running live

```
nice cargo build --locked --all-features
export HT_LEAK_RUN_ID=$(uuidgen)
python3 tests/native/claude_mod/stress.py --profile <signed-in CLAUDE_CONFIG_DIR> --iterations 3 --settle 60
scripts/check-no-leaked-processes --run-id "$HT_LEAK_RUN_ID"
```

- **Defaults.** `--profile` defaults to the spike profile, `--iterations` to 20 (about 2.5 hours; 3 iterations
  take about 23 minutes). `--scenarios a,b` limits the run.
- **Debugging.** `--debug-file` writes `claude --debug-file` logs, and `--keep-root` keeps the private run dir.
- **Concurrency.** Run one live driver at a time per profile.
- **Driver checks.** `python3 -m unittest tests/native/claude_mod/test_stress.py`.

## Invariants checked per iteration

- **Delivery and acks** (ledger):
  - at most one `delivered` and one settling `acked` (`reason: settled`) per message id per Claude session;
  - a `/clear` or an `ack:stale_generation` refusal legitimately resets the id;
  - no ack for a truncated id.
- **Submit timing:**
  - no ledger `submit` while the mod's own `turn` is set;
  - no `submit` inside a post-abort hold (Esc until the next user turn is typed);
  - no `submit` inside a main turn as the scenario hook log sees it: a UserPromptSubmit to the next unblocked
    Stop, Esc, session exit or SessionStart.
- **Receipts:**
  - every sent id ends `acked` or `pending` in the daemon (never missing);
  - a settling ledger ack never disagrees with a pending daemon receipt;
  - every daemon-acked id was acked by the mod.
- **Settled in time:** every require-ACK id is `acked`, and every lazy row `displayed`, within `--settle` seconds
  (plus 60 s for the drop scenario).
