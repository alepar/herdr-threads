# herdr-threads Claude Code mod

Development contract for the bundled mod (epic ht-j16). The spec is
`docs/superpowers/runs/2026-10-09-claude-mod-inbound-delivery/2026-10-09-claude-mod-inbound-delivery-design.md`
(D1-D8); the wire and JSON-line types are `src/protocol/watch.rs`. This tree is a
stub until ht-j16.6 implements delivery.

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
