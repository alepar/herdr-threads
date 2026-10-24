# agy on Linux: live wake through the relay, and the turn-end hook probe (2026-10-24)

Host blacky (Fedora 44, Linux 7.0.12 x86_64), Herdr 0.9.5, genuine agy **1.4.6** `linux_amd64` (downloaded and
sha512-verified by `scripts/agy-sandbox/fetch-agy`, pinned in `agy-versions.json`), herdr-threads 2.3.1 from
`integration/ht-agy-linux`. Model traffic only through `scripts/agy-relay` (OpenRouter key read by the relay from
`~/.hermes/.env`, never copied, printed or given to agy; agy holds only the relay's per-run local token).

## Summary

| Run | Result | Spend | Evidence |
|---|---|---|---|
| `agy-live-smoke --dry-run` (scripted model), at `ff56289d5` | **PASS**: wake, injected `<SYSTEM_MESSAGE>` with the frame header, ACK, reply, ack_phase `wake`, leak check clean | $0 | [live-smoke-dry-run/](live-smoke-dry-run/) |
| `agy-live-smoke --live` (`google/gemini-2.5-flash`), at `ff56289d5`, clean tree | **PASS**: wake, injection, explicit `herdr-threads ack`, reply SMOKE-OK, ack_phase `wake`, leak check clean | $0.0142, 8 requests | [`../agy-live-smoke/2026-10-24-ff56289d5195/`](../agy-live-smoke/2026-10-24-ff56289d5195/) |
| the same, earlier, at `096e18a36` | wake, injection, reply, receipt acked; **FAIL** on one check: the model ran `inbox` instead of `ack` (see Defects 1) | $0.0143, 6 requests | [`../agy-live-smoke/2026-10-24-096e18a36f69/`](../agy-live-smoke/2026-10-24-096e18a36f69/) |
| `wake-e2e.py --harness agy --cases send-p0/group,send-p1/group`, run 2 | 4/6 PASS; `ag-t0` blocked at an agy permission prompt for `echo` (driver fixed: `command(echo)` pre-allowed) | $0.0301, 20 requests | [wake-e2e-run2/](wake-e2e-run2/) |
| the same, run 3 | 4/6 PASS; **every wake decision correct**; `ag-t0` rows not `delivered` because the model did not print the line it said it would (model behaviour) | $0.0375, 24 requests | [wake-e2e-run3/](wake-e2e-run3/) |
| turn-end probe (ht-kyfw), at `ff56289d5` | Stop `continue` capped at 10 per turn; PostInvocation `injectSteps` makes no extra call; `force_continue` has no cap below the relay's 40-request ceiling | $0 | [probe/](probe/) |

Run 1 of the wake e2e (not kept) exposed two driver bugs, both fixed on this branch: the launch line ran `agy` by
name (the pane has the Herdr server's PATH), and the first-run walker kept re-answering the colour page, because
with `altScreenMode: never` earlier pages stay on screen.

## How agy runs on Linux

The agy gate's macOS `sandbox-exec` profile has a bubblewrap equivalent (`scripts/agy-sandbox/agyenv.py`):
`bwrap --die-with-parent --unshare-net --ro-bind / / --dev-bind /dev /dev --proc /proc --bind <root> <root>
--tmpfs <each of ~/.gemini ~/.claude ~/.codex ~/.aisw ~/.hermes, /dev/shm, /dev/mqueue that exists>`. agy gets its
own network namespace (loopback only) and a read-only system; the host /dev stays bound so its TUI keeps the
pane's pty, and the user-writable /dev/shm and /dev/mqueue are private tmpfs, so no write outside the world root
reaches the host. A socat inside the namespace
listens on the relay's loopback port and forwards to `<root>/relay.sock`, which a host-side socat forwards to the
relay. Measured with a stand-in server: the relay port answers, any outside host fails, the hidden directories
are empty, a write outside the root fails. `test_agyenv.LinuxSandboxBehaviour` checks the same with the rendered
wrapper (outbound connect, hidden dirs, writes outside the root, `/dev/shm`); the first revision bound /dev with
no tmpfs, and its /dev/shm writes reached the host (super-roast finding, fixed in `ff56289d5`).

There is no pid namespace: agy's hooks report agy's pid as the binding's liveness evidence, so it must be a host
pid. With `--unshare-pid` the binding recorded `liveness_pid: 2`, the daemon disconnected it
(`liveness_process_exit`) and no wake fired.

The wake e2e (`--harness agy`) runs agy without that sandbox, with HOME at its scratch home, the relay's token and
URL from a 0600 env file, and on the Terms page it unticks the pre-ticked consent to Google collecting
interaction data (agy 1.4.6 shows it ticked).

## Wake e2e on agy (runs 2 and 3)

| case | T | run 2 | run 3 |
|---|---|---|---|
| send-p0/group | 0 | blocked at `echo` permission | woke 1.04 s; not `delivered` |
| send-p0/group | -1 | PASS, woke 0.76 s | PASS, woke 0.76 s |
| send-p0/group | 3 | PASS, woke 1.02 s | PASS, woke 0.51 s |
| send-p1/group | 0 | blocked at `echo` permission | stayed idle (correct); not `delivered` |
| send-p1/group | -1 | PASS, woke 0.76 s | PASS, woke 1.01 s |
| send-p1/group | 3 | PASS, woke 0.76 s | PASS, woke 0.76 s |

Every passing row also met `no_extra_inbox` (0 `inbox` calls: the PreInvocation hook delivered the item). In run
3, `ag-t0`'s wake turn shows the model's own thought "I have received a message from sloNOTdXRzOg … I will print the
received message as instructed", then the turn ended with no output; on the next (`hello`) turn it printed the p0
`RECEIVED` line. The relay mapped agy's requests to `gemini-3.1-pro-preview` and `gemini-3.1-flash-lite-preview`
(`relay-summary.txt`).

## Turn-end probe (bead ht-kyfw)

`scripts/agy-gate/turn_end_probe.py`: one isolated world per mode, the fake upstream answering `done` to every
request, probe hooks in the world's own `~/.gemini/config/hooks.json`, two user turns each (`ONE`, `TWO`). Raw
records: `probe/<mode>.json` (hook log with payloads and outputs, the probe markers each model request carried).

| Mode | Hook output | Observed on agy 1.4.6 |
|---|---|---|
| `stop-continue` | Stop always `{"decision":"continue","reason":"PROBE-STOP-REASON-<n>"}` | 11 model requests per turn: the first plus **10 continues**; the 11th Stop also said continue and agy stopped. The count restarts each user turn (`executionNum` 0..10, then 0). Each `reason` reaches the next request and stays in the history (request k carries reasons 1..k-1). Stop payloads: `terminationReason: "NO_TOOL_CALL"`, `fullyIdle: true` |
| `post-inject` | PostInvocation `injectSteps` (one `systemMessage`) on its first call only | **No extra model call** (1 request per turn). The injected message first appears in the next user turn's request |
| `post-force` | PostInvocation `injectSteps` + `terminationBehavior: "force_continue"` on every call | **No cap below the relay's 40-request ceiling**: 40 model requests in turn 1, each carrying the previous injections, until the relay answered 429; agy then ended the turn with Stop `terminationReason: "ERROR"` and showed `agent executor error … RESOURCE_EXHAUSTED`. Turn 2 was not measured (the budget was spent) |

Implications for end-of-turn delivery (idea 11): a Stop `continue` is a bounded (10 per turn) way to keep a turn
open for an item that arrived after the last tool boundary, and its `reason` is a persisted message. A
PostInvocation injection alone is free but only lands on the next user turn. `force_continue` must be limited by
the hook itself: agy imposed no limit within 40 calls.

## Provenance

The live smoke, the dry run and the probe above ran at `ff56289d5` (`dirty: false` for the live smoke), the branch
head's code apart from this evidence. The two wake e2e runs and the first live smoke ran before a rebase, at
`096e18a36` (a pre-rebase copy of `a490eace7`), the e2e runs from a dirty tree that already held the e2e fixes of
`fb9f23f34` (absolute agy path, onboarding walker; run 3 also the `command(echo)` allow); the consent walker became
fail-closed later (`ff56289d5`), which these runs predate. The wake e2e was not re-run at the head.

## Defects and notes for routing

1. **live smoke check is stricter than the receipt contract** (low, test infra; the head run happened to ack
   explicitly and passed). `the agy seat ACKed through
   run_command` requires a `herdr-threads ack` call, but the receipt was settled by `cooperative_hook_delivery`
   (confirmed from agy's transcript), which by design ACKs a completely delivered item; the model read the skill and
   ran `inbox`, then replied. The check should accept a receipt the seat's delivery settled.
2. **agy 1.4.6 changed `run_command`'s schema**: `Cwd` and `toolAction` are rejected (1.3.2 required them). Only
   the dry run's scripted model was affected; it now offers both shapes. Real models read the schema.
3. **agy 1.4.6 Terms page pre-ticks consent to Google collecting interaction data.** Our walkers press Done with it
   ticked in the agy gate (inside the no-network sandbox, so moot there); the wake e2e unticks it.
4. **The relay maps agy's Gemini model ids to OpenRouter models it chooses** (`gemini-3.1-pro-preview`,
   `gemini-3.1-flash-lite-preview` here, `google/gemini-2.5-flash` in the smoke): wake-e2e content checks depend on
   that model's instruction-following, as run 3's `ag-t0` shows.

5. **The dry run's ACK check passes by construction** (FYI, pre-existing): it matches the tool calls the scripted
   model emitted, not commands agy executed or the receipt's ack provenance.

## Reproduce

```sh
python3 scripts/agy-sandbox/fetch-agy --dest DIR          # verified download for this platform
export HT_AGY_BIN=DIR/<version>/agy
scripts/agy-live-smoke --dry-run                          # zero spend
scripts/agy-live-smoke --live                             # key from ~/.hermes/.env, relay only
python3 scripts/agy-gate/turn_end_probe.py --evidence-dir OUT
# wake e2e through a standalone relay: start scripts/agy-relay/relay.py --live --run-dir R, then
GEMINI_API_KEY=$(cat R/local-token) GOOGLE_GEMINI_BASE_URL=http://127.0.0.1:$(cat R/port) \
  PATH=DIR/<version>:$PATH python3 scripts/wake-e2e.py --harness agy --cases send-p0/group,send-p1/group
```
