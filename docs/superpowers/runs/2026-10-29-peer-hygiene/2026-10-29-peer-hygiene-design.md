# Peer hygiene: dead identities, identity stability, link errors (2.8.0)

Branch `feat/peer-hygiene` from `ea19734ef`. Evidence: blacky's store, read only, and the laptop seat's
read-only outputs (2.6.3).

## 2. Identity stability: finding first, because it reframes item 1

The 54 `host=Mac.dayton` nodes are not earlier identities of the laptop's real store. They are test daemons
from a single test-suite run on the Mac:

- Blacky first saw all 54 between 10-17 07:49:30Z and 07:55:38Z, which is 13 to 19 minutes after the laptop's
  current node Q0V (created 07:15Z) first linked. Nothing has appeared since.
- Each one has a single log of 1 to 22 entries, except KxC with 264. Their content is a test fixture:
  - 54 `node.genesis`, 114 `seat.created`, 57 `thread.created`, 63 invitations, 308 `message.sent`;
  - 6 failed entries ("origin is not the authorized author of seat.created");
  - 24 entries `parked_dep` behind those failures.
- They reached blacky only through Q0V: every last contact is `relayed`, and no dial target points at any of
  them.
- Commit 681ea5e46 (ht-978d, "fix: isolate test daemon peer discovery") landed at 10-17 08:14Z, about 20
  minutes after the last of them appeared. Before it, test daemons used the real `herdr machine list`
  discovery. Its validation line reads "Blacky log count stays 56", and 56 is these 54 plus nSG and Q0V.
- ht-hvyf (2.4.3) closed the other leak, process-scan dial targets of test `herdr --remote` clients.

The rest is by design, so there is no defect to fix:

- Per N1, a node is its key. A fresh state dir mints a new key and so a new node: a test instance, a scratch
  `--state-dir`, a wiped store.
- A reboot, a migration or `sync resync` keeps the key. Resync draws only a new incarnation of the same node.
  A wiped store that still has its key also starts a new incarnation of the same node.
- So the only way a new node becomes a lingering peer is that it reaches a real node. That path is closed for
  test builds (ht-978d) and for the scripts, which seed `discover:false` (wake-e2e, isolated-herdr,
  release/native/linux fixtures).
- A release-binary daemon that someone runs by hand on a scratch state dir is a real node by definition, so it
  stays outside this guard.

Fix scope for item 2:

- No code change.
- One regression test: an isolated test daemon makes no discovery call against a real `herdr` (a guard test
  for ht-978d, if one does not already exist; I will check before adding).
- The finding goes into the report.

## 1. `peer prune`: forget dead node islands (operator)

### "Provably this host": what can and cannot be proven

The model has no lineage across keys. `GenesisPayload.predecessor` chains incarnations of one key only. The
host label is the `gethostname()` captured at genesis, which the user already ruled out as proof. That leaves
no replicated evidence that two keys ran on one machine, and inventing some would be a heuristic (N5).

So the command keys safety to properties it can prove from replicated data:

- the node is dead;
- nothing live depends on it.

The host label serves only as a required scope filter: it is never proof, it only limits what one run can
touch.

### Candidate rule

A node X qualifies only when every condition below holds. The daemon checks them in one transaction against
its canonical view (A2).

1. X is not the local node.
2. X has no live link, and no enabled dial target has learned X.
3. X's last contact (direct or relayed) is older than `--older-than` (default 7d). If there is no contact
   row, X's `first_seen_at` counts instead.
4. X's `host_label` equals `--host`. The default is the local node's own label. Blacky would pass
   `--host Mac.dayton`.
5. X has no open fork or clone evidence (`sync_forks`, open `sync_clone_evidence`). That evidence stays
   visible.
6. Island closure over the candidate set S, which is computed as a fixpoint:
   - every seat that is a member of, was invited to, or holds a receipt in a thread created by X is homed in
     S;
   - every thread that has a member or a message from one of X's seats was created in S;
   - no journal entry of a log outside S is parked or pending on something X creates (`needs`/`deps`);
   - no `peer_requests` row names X.

   A node that fails closure is reported with its reason and is not pruned.

On blacky today, all 54 pass: 0 threads mix dead and live seats, and no live log references their facts.

### Effect

The command deletes no data. It adds a tombstone row and changes the views, so it is reversible and does not
lose facts:

- New table `sync_forgotten_nodes(node_uuid PK, tag, host_label, pubkey, forgotten_at, provenance)`, with
  provenance `operator:local-user:<uid>` (A3). It is node-local and never replicated.
- Coordinator: for forgotten logs it sends no `Want`, omits them from its own `Have`, and ignores any
  `Entries` for them. This is a receiver-side pull policy only: no frame or wire change, and old peers are
  unaffected. A peer that still holds the logs simply goes on holding them.
- Views:
  - `peer list` and `sync status` omit forgotten nodes and print one `forgotten N` line instead (11 to 14
    bytes);
  - `sync unapplied` excludes them, which takes the laptop's parked/failed counts to zero;
  - `peer prune --list` shows the tombstones.
- Materialized seats and threads stay as they are. N5 forbids retiring remote seats by observation, and they
  are already invisible: the islands have no live members.

### Purge alternative, rejected

Physically deleting journal entries, logs, seats, threads, messages, invitations and receipts touches about
25 tables with FK chains. It would also need a tombstone anyway, because otherwise any peer that still holds
the logs re-sends them on the next `Have`. That makes it more risk for no extra visible effect, so it can be a
later `--purge` if store size ever matters. Today it is about 720 entries.

### What blacky must forget

Blacky must forget the same 54 nodes. Every node runs the command itself. The criteria depend on replicated
data plus that node's own contact times, so both sides compute the same set.

A replicated "forget" fact is rejected, for two reasons:

- it would let one node hide nodes from another, which conflicts with N3/N9 (views are local);
- it would need a new fact kind, and older nodes would park it.

### CLI

```
herdr-threads human peer prune [--host LABEL] [--older-than DUR] [--yes] [--list]
```

- Dry run by default. `--yes` applies. It is an operator command, under `human` like `peer forget` and gated
  by `is_mutating`.
- Dry-run output:
  - `peer_prune` (10 B);
  - one row per candidate, `node <tag> prune last=<MM-DD HH:MMZ> logs=N entries=N` (about 50 B);
  - one row per refusal, `node <tag> keep: <reason>` (about 30 to 60 B, reason enum: linked, dial_target,
    recent, other_host, fork_evidence, referenced_by <tag>);
  - capped at 64 rows plus a `next:` line;
  - a trailer `would_prune N` or `pruned N`.
- Each applied prune writes a `daemon.log` line with its provenance.
- Client-daemon capability `peer.prune.v1`. An older daemon gets a clear refusal, never a garbled op.

### TRUST-POLICY amendment

- New rule N16, "Forgetting a dead node is a local view repair": the operator action, the six conditions, no
  deletion, no replication, node-local pull policy, provenance.
- Accepted limit: a live node that later authors a fact depending on a forgotten island (for example inviting
  one of its seats) parks it here until the tombstone is removed. With closure checked at prune time, that
  only happens through a deliberate act on a dead seat.

## 3. Link errors out of the main views, plus `peer events`

### Cause

Both views print the persisted `sync_dial_targets.last_error`. It is set on `Backoff` and never cleared on
`Linked`. `sync status` also falls back to it for an inbound or linked link, because the coordinator always
reports `last_error: None`.

### Display rule

`err=` shows only while it is current. The rule: the target is not `linked`, and `last_error_at >=
last_linked_at` (or the target has never linked). The column stays in the store; `last_error` in the `peer list` and `sync status` replies (text and `--json`) carries the current error only, and the history is `peer events`.
`sync status` link rows drop the dial-target fallback when the link is active.

### `peer events`

- New table `sync_link_events(seq PK, at, node_uuid NULL, target_id NULL, kind, detail ≤160 B, n, last_at)`.
- `kind` is one of `linked`, `ended`, `dial_failed`, `not_installed`, `stopped`, `inbound_linked`,
  `inbound_ended`.
- Writers: the dialer's `DialTransition` record path (outbound) and the coordinator's link up/down (inbound,
  `peer --stdio`). These are the points that already write daemon.log.
- Flood bound: a repeat of the newest row's (kind, target/node, detail) only bumps `n` and `last_at`, so a
  backoff loop stays one row.
- Retention: the newest 1000 rows and at most 30 days, pruned on insert.
- CLI: `herdr-threads peer events [--node N] [--limit N (default 20, max 64)] [--before SEQ]`, read-only.
- Output:
  - `peer_events` (11 B);
  - one row per event, newest first: `event <seq> <MM-DD HH:MM:SSZ> <tag|-> <kind>[ x<n>][ to=<last
    HH:MM:SSZ>]: <dest> <detail>`, about 50 to 90 B plus a detail of at most 160 B;
  - a `next: herdr-threads peer events --before <seq> ...` line when more rows exist.
- `--json` returns the serde rows. Capability `peer.events.v1`.

## Acceptance

- Laptop and blacky dry runs list the 54 Mac test nodes as `prune` and nSG/Q0V as excluded. After `--yes`:
  - `peer list` shows only live nodes plus `forgotten 54`;
  - `sync status` shows no parked or failed rows from them;
  - the link to the live node is unchanged (rx/tx counts keep advancing).
- Multinode tests:
  - an island is pruned and no longer pulled after a relink;
  - a node referenced from a live thread is kept with `referenced_by`;
  - a linked node, one with a recent contact and one with a dial target are each kept;
  - dry run writes nothing.
- `err=` disappears after a relink; a current backoff still shows it.
- `peer events` records link up, end and dial failure, collapses repeats, and pages with `next:`.
- Checks: clippy all-features `-D warnings`, fmt, harness-neutral, wire-compat (new protocol types only add
  baseline lines and new fields carry `serde(default)`), default-features, focused tests, and one full suite
  with a leak check.

## As built (2026-10-29)

Approved by the coordinator with tombstone and hide, no purge. Where the implementation differs from the text
above:

- **Island rule.** The closure is computed over the journal's own dependency graph. Two nodes share an island
  when an entry of one `needs` an entity an entry of the other `creates`. This covers seats, threads,
  invitations, messages and receipts in one rule, because facts are the only replicated references and the
  journal is never compacted. The local node, nodes off the host and log owners unknown to the directory block
  their island. A `parked_layout` entry in a non-prunable log blocks everything (`opaque_entries`).
  Blacky today: 6 cross-node edges, of which only nSG and Q0V touch the live nodes.
- **CLI.**
  - `peer prune [--host L] [--older-than-days N] [--yes | --list]`. The dry run and `--list` are reads;
    `--yes` is operator-gated (`human`).
  - The `next:` line of a dry run is the `--yes` command.
  - `@MACHINE` matching (N14) also leaves forgotten nodes out. Today `@Mac.dayton` matches 55 nodes and is
    refused as ambiguous.
- **Link events.**
  - A `kind` (`up`, `down`, `dial_failed`, `not_installed`, `stopped`) plus an `origin` (`dialed`,
    `inbound`).
  - The coordinator records up and down for every link, inbound and dialed. `record_transition` records the
    dial outcomes before a link is up.
  - Repeats collapse into `xN to=TIME`.
- **No client capability names.** Earlier `SyncCommand` additions (address resolve, remote launch) added none.
  An older daemon refuses the unknown `op` with its decode error.
- **Item 2 guard.** `isolated_machine_list_never_names_the_real_herdr` covers the machine-list path that let
  the 54 test daemons in. The process-scan and lane-probe paths already had guard tests from ht-978d.
