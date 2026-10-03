# Private warning lifecycle capture

Run on 2026-10-03 with `herdr 0.9.1` and all-feature `herdr-threads` binary SHA-256
`5e44095576344a904a27a37b89961f6c1af42ac1c31ae47f0b1692131c9eeabb`.
The runner created a private Herdr server, two private panes, isolated HOME and
Claude/Codex config, and a stand-in Claude command. It stopped its daemon and
server, removed its temporary root, and passed the run-scoped leak check.

Reproduce from the worktree root after building the current binary:

```sh
nice cargo build --locked --all-features
python3 docs/evidence/thread-overhead/run_warnings.py \
  --binary target/debug/herdr-threads \
  --output docs/evidence/thread-overhead/warning-capture
```

The script uses `scripts/lib/isolated-herdr.sh` for teardown and
`scripts/check-no-leaked-processes --run-id <run-id> --root <private-root>`.
It needs native process inspection (`ps`); the workspace sandbox denied that
inspection, so this capture ran with permission for the isolated script.

| Stage | Time from first send | Condition rows | Warning jobs | Active warnings | Delivered prompts |
| --- | ---: | ---: | ---: | ---: | ---: |
| First overdue open | 5.22 s | 1 | 1 | 1 | 1 |
| Second overdue source, quiet | 10.48 s | 1 | 1 | 1 | 1 |
| Persistent warning phase | 31.26 s | 1 | 1 | 1 | 2 |
| Both receipts ACKed, clear | 31.39 s | 1 | 2 | 0 | 2 |
| Daemon restart | 31.46 s | 1 | 2 | 0 | 2 |
| New overdue source, reopen | 36.68 s | 2 | 3 | 1 | 2 |
| Reopened phase after wake throttle | 61.92 s | 2 | 3 | 1 | 3 |

The first and second receipts shared one open warning ID. ACKing only the first
left it active; ACKing the second recorded one clear warning. The cleared row
and zero active warnings survived a daemon boot ID change. The later receipt
opened a new row and warning ID for the same thread and recipient.

The stand-in received three **nonempty** generic attention prompts. Their
times were approximately 0.50, 31.2, and 61.8 seconds after the first send.
There were five raw transport lines, including two empty lines from shell
submission verification. The 30-second minimum wake delay was configured;
the generic prompts cannot be attributed to a specific warning event because
ordinary sends and warning jobs share the seat's wake channel. This capture
does establish that the second overdue source produced no additional durable
warning job or prompt before the throttle elapsed.

`result.json` records all stage snapshots, IDs, event sequences, exact timings,
binary hash, boot IDs, and leak result. `commands.jsonl` records the private
CLI and Herdr calls; `received.jsonl` preserves the raw stand-in transport.

## Deferred recovery and waiver closures

The native capture above uses receipt ACKs, which publish a single-condition
clear in the ACK transaction. The multi-thread recovery and human-waiver paths
were checked with focused store tests. A 256-condition recovery decision wrote
one close sweep and one work job, touched at most five database rows including
the seat update, and published no clear in the foreground. A one-unit worker
admission cleared exactly one condition; subsequent admissions completed after
the writer was reopened. Other tests checked disjoint repeated-waiver sweeps,
the first close timestamp, a late member excluded from both pending and
projected clear recipients, and clear-before-reopen event order.

A new unavailable episode's publication waits with `StoreBusy` while its prior
episode's clear is queued for the same seat and thread. After the fair work
lane publishes that clear, a fresh send operation can publish the new open.
The blocked operation's frozen preparation may require the existing bounded
cleanup path before reuse because the clear advances the timeline revision.
