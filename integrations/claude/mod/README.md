# herdr-threads Claude Code mod

Development contract for the bundled mod (epic ht-j16). The spec is
`docs/superpowers/runs/2026-10-09-claude-mod-inbound-delivery/2026-10-09-claude-mod-inbound-delivery-design.md`
(D1-D8); the wire and JSON-line types are `src/protocol/watch.rs`.

## Layout

Embedded into the binary and installed (ht-j16.7, `include_str!`):

- `.claude-plugin/plugin.json` (`version` is `0.0.0` in the source tree)
- `hooks/hooks.json` (`{ "modules": ["./register.js"] }`)
- `hooks/register.js`
- `types/index.d.ts`

Development only, never embedded: `README.md`, `tsconfig.json`, `tests/**`.

## `watch` child

```
herdr-threads watch --harness claude --session <id>
herdr-threads watch ack --session <id> --via context|submit|append <ids...>
```

Environment: `HERDR_PANE_ID` and the state dir are inherited. `HERDR_THREADS_MOD_DELIVERY=off`
makes `watch` exit 3 before connecting. `HERDR_THREADS_MOD_LEDGER=<file>` makes the mod append one
JSON line per decision: `{at, kind, ids, via, turn, reason}` with kind one of `received`,
`delivered`, `acked`, `held`, `submit`, `refused`, `restart`.

The `watch` child polls `getppid()` each second and exits when its parent is gone.

### Exit codes and mod reaction

| Exit | Meaning | Mod |
|---|---|---|
| 0 | stream ended (any Close except `disabled`/`replaced`) | restart with backoff |
| 1 | other error (`daemon_unavailable`, `error`) | restart with backoff |
| 2 | refused: `no_binding`, `session_mismatch`, `held`, `unresolved`, `cooldown`, `busy`, `stopping` | retry after 1, 2, 5, 10 s, then every 30 s |
| 3 | permanent: `not_claude`, `disabled`, `replaced`, `no_pane`, `env_disabled`, `unsupported` | stop until reload |

One `status` JSON line is printed before a non-zero exit.

## stdout JSON lines

One object per line, `schema: 1`; consumers ignore unknown keys.

| `kind` | `id` | notes |
|---|---|---|
| `message` | message id | ordinary receipt, `ack_required: true` |
| `lazy` | message id | `ack_required: false` |
| `attention` | `attention:<version>` | never acked; re-sent once per version and after each (re)start |
| `status` | `status:<n>` | `state` connected, refused or closing; optional `reason`, `exit` |

Bodies over 8 KiB are cut at a char boundary, followed by the truncation marker, with
`truncated: true`; the mod never acks a truncated item. Pages hold at most 32 items and 64 KiB.
All text is untrusted data; a submit never starts with `/`.

**Framing.** The fixed header, then one block per item: `[herdr-threads] <kind> <id> in <thread id> "<thread name>" from <sender id> "<sender name>"[ markers]:` and the body. The names are what `watch` supplies (the thread's topic; the sender seat id, or `service`), flattened to one line, quoted, and left out when absent or equal to the id. Markers are `[human]`, `[relays user]`, then `[query]`/`[request]`/`[rule]`, the same fixed text as every other read path. Attention blocks are `[herdr-threads] attention <id>:` and the marker.

## `watch ack`

One stdout line per id: `{"id","result","reason"?}` with result `settled`, `already_settled`,
`refused_terminal`, `stale_generation` or `retryable`. Exit 0 iff every id has a line. Retryable
ids are retried on the next Attention, after the next registration and every 30 s.

## Generations, grace, stall

`generation` is the binding generation; check-ins (`/clear`, resume) rotate it, a plugin reload
does not. On resume the mod may re-ack items delivered under the immediately previous generation
of the same native session; older ones are `stale_generation` and are re-streamed. A drop starts a
30 s reconnect grace (same-generation re-registration cancels it); `Close{binding_changed}` starts
a 30 s seat-level rebind grace. A stalled channel (10 min without ack while an ordinary receipt
waits) is closed with `stalled` and refused for 10 minutes. See spec D5-D7.

## Behaviour

`hooks/register.js` is one self-contained module. `createCore(io)` is the delivery state
machine (events in, side effects only through `io`); `register(on)` wires it to the engine and
builds `io` from `$`. Rules (spec D5, D6):

- **Startup.** At `session.start` (also after a reload) the mod reads the recorded turn state
  from `$.state['herdr-threads'].turns`; with none it starts *assumed busy* until the first main
  `turn.complete`, or 5 s with no `turn.start` and a readable prompt box. It then spawns
  `watch` for the session id (and later `watch ack`) with the launch line `setup claude` wrote into the
  installed `hooks/register.js`: the hooks' own invocation (absolute executable, `--state-dir`,
  `--host-endpoint`) as `const LAUNCH = {"argv":[...]}`. Only when that line is absent (the repo copy
  `claude plugin test` loads, or a hand-loaded checkout) does the mod launch by name:
  `HERDR_THREADS_BIN`, else `herdr-threads` from `PATH`. A missing `$.store`,
  `$.prompt.submit`, `$.session.append` or `$.process.spawn` leaves the mod inert (ledger
  `refused`, reason `api_missing`). The engine's validator forbids reading `$` members as
  values, so the check is by calling: the store is probed at load, the others on first use.
- **Busy.** The open main turn ids live in `$.state` (events with `agentId` are ignored). A
  `turn.start` replaces the open set; a `turn.complete` removes its id and any older one.
- **Routing.** `message` and `attention` while busy ride the next main `tool.call` result as
  `context`, only when that result is the answered variant (a deny, an error or a subagent call
  leaves them queued). While idle they go out as one batched `$.prompt.submit`, never awaited
  in a hook, never while a main turn is open (re-checked right before the call). `lazy` rows
  go to `$.session.append` at once. Each context or append delivery logs one dim `$.ui.log` line.
- **Idle-submit gates.** (1) After a main turn that was interrupted (an aborted `turn.complete`, `reason` `aborted`, or a
  turn whose last main `tool.call` ended in a deny or an error, which is how Esc at a permission
  dialog shows) submits are held until a later
  non-aborted main turn completes, or 120 s pass with no open turn and an empty prompt box
  (this overrides the draft rule). (2) A non-empty prompt box holds a submit up to 120 s. (3) One
  submit in flight. A `drop` keeps the items while their run is connected, ledgers `refused` and backs off 30 s.
- **Channel loss.** Items are delivered only while the `watch` run that streamed them is
  connected, from its `status connected` line until its `closing`/`refused` line, its exit or a
  `session.end`. Then every queued item not in flight is dropped (ledger `refused`, reason
  `channel_lost:<why>`), and the next run re-streams whatever is still pending. A submit or
  append in flight at the loss that succeeds counts as delivered and is acked. One that is
  dropped or denied is discarded, unless the new run re-streamed it. The delivered-but-unacked
  set is kept and re-acked after the next `connected`.
- **Acks.** After a resolved delivery the mod runs `watch ack --via <path> <ids>` for messages
  and lazy rows only; never attention items, never truncated items. Per-id results: `settled`,
  `already_settled`, `refused_terminal` and `stale_generation` leave the set (the last also
  forgets the delivery so a re-stream is delivered); `retryable`, a non-zero exit or a missing
  line stay and are retried on every new stream line, after `status connected` and every 30 s
  while connected.
- **Reload.** Before `$.prompt.submit` the mod writes the batch (`sid`, ids, ackable ids) to `$.state`
  `turns.submitting` and clears it when the submit resolves. A core that loads such a record for its
  session is busy and never re-submits or attaches those ids. A main `turn.start` whose prompt frames
  them, or the `turn.complete` of a turn it never saw start, marks them delivered via `submit` (ledger
  reason `predecessor_submit`) and acks them. 120 s with no open turn, no turn start and an empty
  prompt box releases them for normal delivery (`predecessor_no_turn`).
- **Persistence.** `$.store` key `delivered:<session id>` holds `{delivered, unacked,
  attentionVersions}`. `session.end` `clear` discards the key (and the queue); `resume` keeps it
  (a changed session id, as with `/branch`, starts an empty key: an accepted duplicate limit).
  Every `watch` restart re-reads `$.session.id()` first. The engine can still report the old id
  right after a `session.end` handler returns (2.1.295 after `/clear`). A changed id is adopted
  with the reason of the last `session.end`: `clear` starts an empty record, `resume` loads the
  new key. Attention items are delivered once per watch run (re-sent after each restart).
- **Child.** Exit 0/1 restart after 1, 2, 5, 10, 30 s (reset after 60 s connected); exit 2
  retries on the same ladder; exit 3 stops until reload. A ledger `restart` line records each.
  A `refused` `session_mismatch` or a `closing` `binding_changed` makes each tick re-read the
  session id while no child runs. A changed id restarts at once with the ladder reset; an
  unchanged one waits for the ladder.
- **Ledger.** `HERDR_THREADS_MOD_LEDGER=<file>`: the in-memory tail (2000 lines) is rewritten
  through one serialized promise chain, since `$.fs` has no append.

## Tests

`scripts/test-claude-mod` runs `claude plugin validate --strict` and `claude plugin test` with
an isolated `CLAUDE_CONFIG_DIR`, or prints `skipped: <reason>` when `claude` is absent or older
than 2.1.287. `tests/delivery.test.ts` has one test per rule; `tests/stress.test.ts` runs 600
seeded schedules (base seed constant `BASE_SEED`; a failure names the seed and step, and
`TRACE_SEED` replays one). The runner gives tests no environment, so the seed is edited in the
file.
