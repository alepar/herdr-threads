super-roast verdict: clean (2 nits) [low coverage]
mode: PR        iteration: 1 of 3
profile (assumed): Internal developer tooling (local daemon plus peer sync between one user's machines). The change is a display-only filter; no identity, routing or wake path is touched. Moderate blast radius: wrong output misleads a coordinator but decides nothing.
inputs: bead/ht-hldp@079a42b24 vs main@6e918785b
coverage: unavailable — inline run by the authoring subagent, no coverage object · lanes walked: correctness, trust-policy, wire-compat, concurrency/blocking, test-causality · raw 4 → deduped 4 → 0 confirmed defects
independence: none (inline) — the reviewer wrote the change; same blind spots apply · rung: inline

## Confirmed findings

None.

## Rejected (with reason)

- [Should-fix] src/service/dispatch.rs, `Command::Seats` label arm — a synchronous host read on the async runtime. Rejected: dispatch runs inside `spawn_blocking` (`src/daemon/transport.rs:635`), as the existing `hosts.resolve` host read in the same function does; `NativeCli::seat_labels` is the blocking `herdr api snapshot` call the seat publisher also makes from `spawn_blocking`.
- [Should-fix] TRUST-POLICY N13 / C1 — daemon-read labels feeding a decision. Rejected: the labels only narrow a `seat list` page (display); no seat, binding, wake, routing or resolution path reads them, and the match rule is the publisher's (`TargetLabels::fits`). The module header of `seat_directory` was updated from "never the daemon's" to the new fact.
- [Should-fix] Wire compatibility. Rejected: no wire type changed (`SeatsQuery`/`SeatFilter`/`SeatDirectory` untouched); `StorePort::seats_read` is daemon-internal with a default; `scripts/check-wire-compat` ok.

## Unverified nits (spot-checked)

- [Nit] A label-filtered listing's pages are each matched against that call's host read, so a tab renamed between pages can shift which local seats appear on later pages. Same as remote labels changing between pages; advisory by design, cursor digest unchanged.
- [Nit] With several attached hosts the reads share one 1 s budget sequentially; a slow first host can leave a later host's seats matching no label. Stated as an accepted limit in the seat-directory spec §6.6.

## Escalations (need human)

None.
