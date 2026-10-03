# herdr-threads design index

One row per document in this directory. Status values: `implemented` (the mechanism exists in `src/`),
`adopted` (an amendment, direction or evidence document that later work builds on), `draft` (genuinely
unimplemented; none today). As of 2026-10 the trust model is cooperative: every design below carries or
points to a "Cooperative reality (2026-10)" account, and text describing the removed adversarial
verification layer is marked superseded rather than deleted. Decision record for that change:
[root spec §B4](remaining-findings/2026-10-01-remaining-herdr-threads-findings-design.md#b4-remove-the-pre-cooperative-verification-layer).

| Design | Status | Decision record |
|---|---|---|
| [Root design](2026-09-27-herdr-threads-design.md) | implemented | root spec §B4; "Cooperative reality (2026-10)" |
| [Caller attribution](2026-09-27-herdr-threads--caller-attribution-design.md) | implemented (probe and permit text superseded; cooperative attribution ships) | ht-4is.2; §B4 |
| [Store](2026-09-27-herdr-threads--store-design.md) | implemented | ht-4is.3; §B4 |
| [Seat identity](2026-09-27-herdr-threads--seat-identity-design.md) | implemented (native-proof text superseded; structural continuity ships) | ht-4is.4; §B4 |
| [Scheduler](2026-09-27-herdr-threads--scheduler-design.md) | implemented | ht-4is.5 |
| [Daemon](2026-09-27-herdr-threads--daemon-design.md) | implemented (native permit sentence superseded) | ht-4is.6; §B4 |
| [CLI](2026-09-27-herdr-threads--cli-design.md) | implemented | ht-4is.7 |
| [Harness](2026-09-27-herdr-threads--harness-design.md) | implemented (empty-shell proof text superseded) | ht-4is.8; §B4 |
| [Package](2026-09-27-herdr-threads--package-design.md) | implemented | ht-4is.10 |
| [Validation](2026-09-27-herdr-threads--validation-design.md) | implemented | ht-4is.11 |
| [Cooperative caller contract adoption](cooperative-caller-contract-adoption.md) | adopted | cumulative review 159 |
| [Cooperative receipt user direction](cooperative-receipt-user-direction.md) | adopted | user direction 2026-09-28 |
| [Shared contract amendment, revision 4](shared-contract-amendment-adopted.md) | adopted (permit, lifecycle-subscription and empty-shell parts superseded) | §B4 |
| [Guarded native start disposition](guarded-native-start-disposition.md) | adopted (empty-shell path superseded) | §B4 |
| [Seat resolution recovery clarification](seat-resolution-recovery-clarification.md) | adopted (allocation-guard text superseded) | §B4 |
| [Herdr direct transport contract](herdr-direct-transport-contract.md) | adopted | source evidence for `src/host/native.rs` |

Mechanisms added by the 2026-10 remaining-findings run, each a section in the design named below with the nested spec as its decision record:

| Mechanism | Section | Status | Decision record |
|---|---|---|---|
| Pacer, commit-change lane kicks, idle bounds | [Root design](2026-09-27-herdr-threads-design.md) "Pacer, lane kicks and diagnostics" | implemented | [Pacer adoption spec](remaining-findings/2026-10-01-remaining-herdr-threads-findings--pacer-adoption-design.md); root spec §B2 |
| Error taxonomy, handshake skew, `full_bodies` capability | [Root design](2026-09-27-herdr-threads-design.md) "Pacer, lane kicks and diagnostics" | implemented | root spec §B3; [operator guide](../../operations.md#logs-degraded-lanes-and-failure-remedies) |
| Live indexes, retention lane, page-fit helper | [Store](2026-09-27-herdr-threads--store-design.md) "Live indexes and retention" | implemented | [Retention and live indexes spec](remaining-findings/2026-10-01-remaining-herdr-threads-findings--retention-and-live-indexes-design.md); root spec §B1 |
| Optimistic admission ladder, `harness-versions.json`, canary | [Harness](2026-09-27-herdr-threads--harness-design.md) "Optimistic admission and the versions document" | implemented | [Harness canary spec](remaining-findings/2026-10-01-remaining-herdr-threads-findings--harness-canary-design.md); root spec §B6 |
