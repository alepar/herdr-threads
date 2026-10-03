# Live service-authored ACK validation

Both pinned live harnesses passed the manual initial `servicesend` cell on
`df85b04fd5cca2a7650aa63bbfc9a8e651b190e0`, based on main `1b35596d`.
This proves acceptance item 9 of the herdr-graph service-send amendment for
these cells. Results and earlier attempts are recorded in
[`service-send-native.json`](../history/graph-amendment-run/service-send-native.json).

| Harness | Version before / after / in session | Manifest | SS0 / SS1 / SS2 | Service message | Root ACK call |
|---|---|---|---|---|---|
| Claude | 2.1.287 / 2.1.287 / 2.1.287 | PASS | PASS / PASS / PASS | `msg-GMLSSLM8` | `toolu_01G6iFu4sfnpADXNdgqQa4qp` |
| Codex | 0.159.3 / 0.159.3 / 0.159.3 | PASS | PASS / PASS / PASS | `msg-A0wMCJDc` | `item_5` |

SS0 registers a v2 service session, ensures the thread, creates the required
invitation, and sends a receipt-bearing request. SS1 joins the model's exact-ID
root ACK call to the stored recipient receipt. Both stored ACK observations have
`cooperative_top_level` provenance. SS2 confirms the request is an ordinary
programmatic message with no seat actor and no receipt row for its service author.
Both cells also passed the separate handoff acceptance/ACK and child-absence checks.
These are cooperative claims joined to transcript calls, not kernel-verified
native proof; they cover these manual initial cells only.

The cells used the same rebuilt executable (SHA-256
`3287f07e1e46a10a97c0fe7fc6627052a8f29ad19573e517573ca0e0446f9a21`).
Their exact commands are:

```sh
nice cargo build --locked --all-features
scripts/validate-native-demo.sh --cell claude-manual --scenario servicesend
scripts/validate-native-demo.sh --cell codex-manual --scenario servicesend --codex-config features.shell_snapshot=false
```

Each cell used a private Herdr session, daemon, state directory and scratch
harness configuration. The prompt contained no message ID. The runner recorded
unchanged pinned harness versions before and after each cell; the session reported
the same version. Cleanup closed the owned tab and daemon, and subsequent daemon
health returned unavailable. The private Herdr server also exited.

Codex required the existing per-run shell-snapshot override. Without it, interactive
snapshot startup sourced the user's `.zshrc`, which put the installed protocol-2
executable ahead of the driver's protocol-3 shim. The same inherited PATH resolved
the shim under `zsh -lc` and the installed binary under `zsh -ilc`; the pinned
Codex source's `capture_snapshot` uses interactive startup. Disabling snapshots
resolved the live protocol mismatch without changing the user's installation or
configuration. Driver hardening remains tracked as **ht-l16**. The setting is
documented in the [official configuration reference](https://learn.chatgpt.com/docs/config-file/config-reference).

Earlier attempts remain archived with their original verdicts. One Codex run with
the override ACKed the service request but failed the separate handoff ACK after
the model omitted a directory component from its explicit state path (**ht-fgo**,
reported to the flakiness agent). Its next attempt passed on `f8f8247d`; both
harnesses then passed again after rebasing onto the compaction followups. The
earlier Claude preview failure and the unsupported dry-runs are also retained.

The retained artifacts contain extracted call records, receipt inspections,
service exchange records, version/binary stamps and step verdicts. Raw sessions,
launch environments, credentials and SQLite databases remain outside the repository.
The service-author inspection was extracted read-only from each runner's database
snapshot. The final cell manifests describe the ordinary handoff; the supplemental
`servicesend-ack.json` records the service ACK join.

Validation on the evidence SHA: validator corpus **35 tests passed**,
all-target/all-feature clippy with warnings denied **PASS**, default-feature check
**PASS**, and formatting check **PASS**. This evidence commit changes no product
source; the rebased driver change only makes both required instructions visible
within the handoff preview.
