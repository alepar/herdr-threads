# agy live smoke

One real agy turn against a real model, proving the adapter's delivery path end to end: enroll, wake,
`systemMessage` injection, ACK and reply. The runner is `scripts/agy-live-smoke`
(`scripts/agy-gate/live_smoke.py`, with `RelayBackend` in `scripts/agy-gate/relay_backend.py`). It is
test infrastructure (R12), not production code.

## Who runs what

- **Only the orchestrator runs `--live`, once per final SHA**, and commits the record it produces
  under this folder as `<UTC date>-<sha12>/`. Implementation leaves, reviewers and CI never run it.
- `--dry-run` runs the identical flow against a scripted fake upstream: genuine agy, zero spend. Anyone
  with a genuine agy may run it.
- Both refuse (exit 2) under `CI` / `GITHUB_ACTIONS`, on a host with no agy sandbox (macOS `sandbox-exec`, or Linux with `/usr/bin/bwrap` and `/usr/bin/socat`), without `HT_AGY_BIN`, and with
  a Herdr older than 0.9.1. `--live` also refuses a dirty worktree unless `--allow-dirty` is given; the
  record names the SHA.

## Rerun

```sh
export HT_AGY_BIN=/path/to/genuine/agy            # scripts/agy-sandbox/fetch-agy downloads one
scripts/agy-live-smoke --dry-run                  # evidence goes to the scratch root
scripts/agy-live-smoke --live                     # orchestrator only; evidence goes to this folder
scripts/agy-live-smoke (--live | --dry-run) [--evidence-dir DIR] [--allow-dirty] [--herdr-threads-bin PATH]
```

`--live` reads the OpenRouter key only from `<pw_dir>/.hermes/.env` (`OPENROUTER_API_KEY`, `sk-or-`
prefix); an environment variable is ignored by design. The key lives only in the relay process and is
never written. agy runs inside the loopback sandbox of the native gate and reaches only the relay (on Linux a
bubblewrap network namespace whose loopback port socat forwards to the relay; `scripts/agy-sandbox/agyenv.py`).

## Flow

One `World` with `RelayBackend` (the relay runs outside the sandbox, agy inside it):

1. `setup agy --with-permissions`, start agy, walk its first-run screens, prompt `This is an automated
   herdr-threads smoke test. Reply with the single word READY.` Check: the agy seat is enrolled.
2. Wait for `idle`. A human sender sends an ACK-required message on the sender-agy automatic pair channel
   (it joins both seats, so no invitation or runner-side accept), asking the model to acknowledge it and reply
   `SMOKE-OK`. Records before 2026-10-24 used an invited thread and a runner accept in the agy pane's identity.
3. **Wake**: within 90 s a new main model request carries the hook-injected `<SYSTEM_MESSAGE>` holding
   the frame header with the message id, and no `<USER_REQUEST>` is a prompt the runner typed.
4. **Act**: within 180 s the model ACKs through `run_command` (`herdr-threads ack <id>`), without the agent
   ever reaching `blocked`, and replies `SMOKE-OK` (case-insensitive) in the thread. The delivery receipt
   is settled, under `cooperative_hook_delivery` or superseded by the ACK.
5. **Fallback**: with no ACK after the woken turn, one explicit prompt (`Check your herdr-threads
   messages, acknowledge any that require it, and reply SMOKE-OK in that thread.`). `ack_phase` is then
   `"prompt"` instead of `"wake"`.
6. **Pass** needs wake, injection, ACK, reply, no permission prompt, no relay refusal and zero leaked
   processes. `ack_phase: "prompt"` passes and is flagged as model behaviour.
7. A 10-minute wall-clock cap; a relay `exhausted` aborts with `aborted: "budget"`.

## Budget

The relay enforces hard ceilings of 40 requests and $0.50 per run (test-infra D10). `--live` lowers them
to 24 requests and $0.25. Expected spend is 6-12 main requests at about $0.003 each; `--dry-run` spends
nothing and records `cost_usd: 0.0`.

## Evidence record format

| File | Contents |
|---|---|
| `summary.json` | `sha`, `dirty`, `agy_version`, `agy_sha512`, `herdr_version`, `model`, `requests`, `cost_usd`, `phases`, `ack_phase`, `checks` (`name`, `ok`, `detail` each), `pass`; also `mode`, `aborted`, `refusals`, `cost_estimated`, `leak_check`, `duration_s` |
| `report.md` | A short narrative: versions, request and cost totals, phases and checks |
| `relay.jsonl` | The relay's per-request log, redacted (D10): no bodies, text trimmed to 2 KiB |
| `daemon-excerpt.json` | The agy seat, its bindings, the thread's invitation, membership and messages (bodies trimmed) and the receipt rows |
| `pane-final.txt` | The agy pane at the end of the run |
| `requests-excerpt.json` | The injected `<SYSTEM_MESSAGE>` text, the model's tool calls and (dry-run only) their results |

Every file passes through one redactor: `sk-or-…` becomes `sk-or-REDACTED`, the world root becomes
`<root>` and the real home becomes `<home>`. There are no full request bodies. The runner refuses to
finish (exit 1) if any `sk-or-` that is not that marker is left in a file it wrote.

Exit codes: 0 pass; 1 the smoke failed, aborted, or the key scan fired; 2 environment refusal; 3 the
leak check failed.

## Records

- `2026-10-10-4d2ec663dbfa/`: live, genuine agy 1.3.2, `google/gemini-2.5-flash` through `scripts/agy-relay`.
  PASS, ack_phase `wake`, 7 requests (cap 40), cost 0.02208829 USD (cap 0.50), leak check clean, no
  relay refusal. R17 compact framing and R18 wake ran together on genuine agy; every check passed.
- `2026-10-24-096e18a36f69/`: live on Linux (blacky, x86_64), genuine agy 1.4.6, `google/gemini-2.5-flash`.
  Wake, injection, SMOKE-OK reply and an acked receipt; FAIL on the explicit-`ack` check only (the receipt was
  settled by `cooperative_hook_delivery`, the model ran `inbox`). 6 requests, 0.01434975 USD, leak check clean.
  Details: `docs/evidence/agy-linux-2026-10-24/`.
- `2026-10-24-ff56289d5195/`: live on Linux at a clean tree, genuine agy 1.4.6, `google/gemini-2.5-flash`. PASS,
  ack_phase `wake`, 8 requests, 0.01423731 USD, leak check clean.
