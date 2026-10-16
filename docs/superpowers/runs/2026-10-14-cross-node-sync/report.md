status: completed with 0 unresolved Blocking, 3 escalations [degraded: code findings parked, final review: not ready (main v1.1.0 not merged; landing order vs ht-794l undecided; release tooling breaks on main — about.toml lacks BSD-3-Clause, 8-char ID regexes in wake-e2e/validate-package/real-ssh-e2e; punch-list robustness items open)]
qualifier: code findings parked
qualifier: final review: not ready (main v1.1.0 not merged; landing order vs ht-794l undecided; release tooling breaks on main — about.toml lacks BSD-3-Clause, 8-char ID regexes in wake-e2e/validate-package/real-ssh-e2e; punch-list robustness items open)
metrics: pending (upstream-feedback not yet run)

Run: epic ht-5fr, branch `super-auto/cross-node-sync` (tip d742211d8) vs `main`. Spec: `docs/superpowers/runs/2026-10-14-cross-node-sync/2026-10-14-cross-node-sync-design.md` plus six nested specs and `2026-10-15-cross-node-sync--main-v1-reconcile-design.md`. Ledger: `.superpowers/sdd/ht-5fr-plan/progress.md`.

## Implemented

All 261 ht-5fr beads are closed, with 0 open [bd list --parent ht-5fr]. 148 landed across 8 coordinator invocations; the last was wf_933c0649-482 [run.md codeBuckets.completed]. Final sweep at 7e558f43a: 7438 passed, 0 failed, 47 skipped, wall time 188 s, no leaks, check-default-features passed [run.md codeBuckets.sweep, sweepFix]. Each line below gives the task, its commit range and whether it needed a fix pass. Source for every line: ledger completion line plus closed bead. Epic beads and `review:` bookkeeping beads are left out.

**ht-5fr.1 Node identity, layout and schema baseline**
- ht-5fr.1.1 Node identity module (NodeUuid/NodeTag, key custody, node.json): 56795f1..4496efa
- ht-5fr.1.2 NodePaths resolve the node layout by state dir only: ff1b452..f74f0be
- ht-5fr.1.3 Optional claim host (RuntimeContext.require_host): 5900551..0b2c3f3
- ht-5fr.1.4 Squash schema into node baseline migrations/node/0001: 4496efa..21bf88d
- ht-5fr.1.5 Node row delta (local_node, herdr_hosts, NodeSeed): b1b82fb..b50e506
- ht-5fr.1.6 Tagged public IDs and pure direct-thread ID function: 5c55f79..2dc7f63
- ht-5fr.1.7 Store-only daemon and primary-host attach via HostChoice: 43deeb6..516a754
- ht-5fr.1.8 Delete legacy migration chain code: dfc1f7f..0cfd35f
- ht-5fr.1.9 Doctor legacy report and legacy purge: 88d6f4e..41fc2fe
- ht-5fr.1.10 Seat home-node delta, node_directory, identity start check: 516a754..f462692 (fix pass)
- ht-5fr.1.11 Node layout integration suite, health hosts, doctor store-only label: fea6be5..6d12a1e
- ht-5fr.1.12 ID consumers (CLI parsing, framing, Claude mod, adapters): f462692..ab0880d
- ht-5fr.1.13 Forward node-schema migrations after the baseline: 75e3e96..ce8bb1e

**ht-5fr.2 Multi-host daemon: HostSessions**
- ht-5fr.2.1 Host addressing seam (HostLocator/HostId/HostIndex): 2dc7f63..43deeb6
- ht-5fr.2.2 CommitKicks fan-out (per-lane pacer lists): 7f77ee4..9a6c5c7
- ht-5fr.2.3 Claim host on the wire (CallerClaim.host, fences): 13caee8..d2ce2a2
- ht-5fr.2.4 Baseline re-key to host_id, per-host observation queries: d8b8c5f..7e51c74
- ht-5fr.2.5 Wake scope per host: 3660268..8df18c9
- ht-5fr.2.6 cooperative_claim by claim host; remove StoreContext.primary_host: 34a66be..e103914
- ht-5fr.2.7 HostSessions registry (attach, sweep, per-host workers): b3c74fa..8e50e1b
- ht-5fr.2.8 Compose HostSessions into the daemon: 975e560..b8e970f (fix pass)
- ht-5fr.2.9 Archival per-host evidence: 6d5c89a..557562e
- ht-5fr.2.10 Health and doctor per attached host: 5848e62..74da622
- ht-5fr.2.11 Seat-row host joins by seats.host_id: 9ce92ac..67152fb
- ht-5fr.2.12 Dispatch by claim host and multi-host integration suite: a77a8f1..5848e62
- ht-5fr.2.25 Fix: start host-scoped lanes when the first host attaches: 44363cb..6a33289

**ht-5fr.3 Journal core**
- ht-5fr.3.1 Hybrid logical clock: c07f75f..f54c8ac
- ht-5fr.3.2 Envelope v1 and sync value types: f54c8ac..b69c5ec
- ht-5fr.3.3 ChainRules and the JournalPort trait: 4da5e82..49b0000
- ht-5fr.3.4 Journal-core baseline DDL and guard triggers: fad81c5..47cbb11
- ht-5fr.3.5 FactKind/FactRegistry and ApplyContext seam: b69c5ec..4da5e82
- ht-5fr.3.6 Journal open, local authoring with inline apply, node.genesis: 4afa9e4..fea6be5
- ht-5fr.3.7 Journal ingest, serve and vector: 5af241c..b5b37eb
- ht-5fr.3.8 ApplyEngine passes and replay; SqliteStore implements JournalPort: 04a1c8c..923feae
- ht-5fr.3.9 MemoryJournal and JournalPort contract suite: d2ce2a2..68ba112
- ht-5fr.3.10 node.retag and tag collision handling: d66f6fe..9df80c0
- ht-5fr.3.11 ThreadRegisters LWW module: a70462a..010c7ea
- ht-5fr.3.12 Lane::Apply, LaneSet u16, kicks_for_commit: f7eca30..18ae275
- ht-5fr.3.13 Apply lane worker and daemon wiring: 32762d0..7f77ee4
- ht-5fr.3.14 Journal replay oracle: ce8bb1e..d8b8c5f
- ht-5fr.3.15 Journal open in daemon start: 18ae275..dfc1f7f
- ht-5fr.3.16 ApplyContext services and OwnHistoryHold: 3a6f084..b3c74fa
- ht-5fr.3.21 Fix: verify and dedupe self-log entries before AheadOfSelf (security): ccfc5d8..2dbc6c6
- ht-5fr.3.32 Fix: commit_kicks fixture vs the seeded self sync_logs row: 8df18c9..ccfc5d8
- ht-5fr.3.37 Fix: own-history hold cap without ingest kicks: be60272..eb8cab5

**ht-5fr.4 Fact kinds and write-path conversion**
- ht-5fr.4.1 Split src/store/control.rs (move-only): 4a7f761..8457c0f
- ht-5fr.4.2 Fact-kind schema additions: e7b72c2..7d79ba3
- ht-5fr.4.3 Fact seams and kind module layout: 8e50e1b..08f25be
- ht-5fr.4.4 WriteGuard and the replicated projection: 7d79ba3..ba6551b
- ht-5fr.4.5 FactRoute and seat.created: 1de1f66..44363cb
- ht-5fr.4.6 DeliveryTail extraction: 4913900..bc2bc01
- ht-5fr.4.7 seat.retired and remote eligibility: f6fc8da..03b457c
- ht-5fr.4.8 thread.created/register/direct: 03b457c..8f7094e
- ht-5fr.4.9 Membership facts: 809da61..d10cd4e
- ht-5fr.4.10 Invitation facts and direct pairs (with inviter_kind): 89af24c..4e8d4f8
- ht-5fr.4.11 message.sent kind and local-adopt materializer: a0fd9bc..131a689
- ht-5fr.4.12 receipt.settled and the batch ACK event: 4e8d4f8..34910c8
- ht-5fr.4.13 Deadline lane scoped to the authoring node: 34910c8..787be9a
- ht-5fr.4.14 Remote seats in node-local workers and reads: 8f7094e..8b609f9
- ht-5fr.4.15 Attribution staging and local_journal provenance: 4227634..0335093
- ht-5fr.4.16 message.sent remote-recipient staging: 131a689..89af24c
- ht-5fr.4.17 Send-fence narrowing with reader audit: bc2bc01..1de1f66
- ht-5fr.4.18 Fact kinds integration and oracle workload: 6d63d72..3e6096d
- ht-5fr.4.37 Fix: store-only daemon restores its own seats after wipe: 0041209..c6b56a2
- ht-5fr.4.38 Fix: restore the actor on topic, name and archive events: 0e87f07..be60272
- ht-5fr.4.40 Fix: bind thread.register actor to the signing node (security): d07d7b4..eb877e6

**ht-5fr.5 Peer link and sync coordinator**
- ht-5fr.5.1 Peer wire frames: 273859f..5900551
- ht-5fr.5.2 PeerLink handshake, credit window, liveness: 5862d83..a640f94
- ht-5fr.5.3 Lane::Sync kick map: 0cfd35f..04a1c8c
- ht-5fr.5.4 Sync store (dial targets, node contacts): 0d12485..75e3e96
- ht-5fr.5.5 SyncCoordinator actor skeleton: 1160c5a..32762d0
- ht-5fr.5.6 Reachability gossip and round-complete signal: 2057b56..34a66be
- ht-5fr.5.7 Dialer with backoff and child ownership: e20d12c..3660268
- ht-5fr.5.8 Daemon transport peer sniffing: 9a6c5c7..a865260
- ht-5fr.5.9 peer --stdio stateless pump: 44d088c..ee90e79
- ht-5fr.5.10 Compose sync runtime into the daemon: d54ba50..7f41f7d
- ht-5fr.5.11 Store-only daemon idle exit: 35ab4bf..f6adf75
- ht-5fr.5.12 Coordinator routing and failover: 0214a30..4227634
- ht-5fr.5.13 Coordinator serve, live tail, ingest outcomes: f4ca99d..28e0655
- ht-5fr.5.27 Fix: dialer ChildGuard signals the process group: 2dbf436..0890d58
- ht-5fr.5.29 Fix: dial-each-other stall: cdfabed..36bedfc

**ht-5fr.6 Forks, restore and incarnations**
- ht-5fr.6.1 ForkProof classification: d89e34f..3dd32f7
- ht-5fr.6.2 Fork wire seam: a865260..e20d12c
- ht-5fr.6.3 serve_branch and log_meta: 7e51c74..1160c5a
- ht-5fr.6.4 IncarnationStore fork tables: 923feae..13caee8
- ht-5fr.6.5 Journal reissue and ApplyMode::Reissue: d12b87c..2ca7035
- ht-5fr.6.6 Union re-issue pass: 5035549..d3c91d1
- ht-5fr.6.7 Owner/restore rotation, branch pulls, clone gate: e103914..8e11b30
- ht-5fr.6.8 Coordinator fork wiring: d3c91d1..2057b56
- ht-5fr.6.9 IncarnationsSink and request table: 96fd547..e1191a0
- ht-5fr.6.10 Resync marker, phases, backup/move: f6adf75..0c5ab80
- ht-5fr.6.11 Incarnations convergence scenarios: 3e6096d..3aaeb70
- ht-5fr.6.12 IncarnationStore status and declared-freeze adoption: 8e11b30..b94e48e
- ht-5fr.6.13 IncarnationsWorker thread: b8e970f..5bc4364
- ht-5fr.6.14 Resync rebuild wiring, TRUST-POLICY limit 3: 17380ff..732a8dd
- ht-5fr.6.15 Daemon composition of the incarnations runtime: 0c5ab80..6d5c89a
- ht-5fr.6.30 Fix: fork branch completion and queue backoff: e7fdaed..a3a5a4d
- ht-5fr.6.31 Fix: salvage keeps the resync marker when Headroom stopped a log: d01f73d..30b0a11

**Top-level leaves**
- ht-5fr.7 Peer discovery worker: 557562e..17380ff
- ht-5fr.8 Operator and status CLI for sync: 732a8dd..7fab0ef
- ht-5fr.9 TRUST-POLICY amendment: 6df3a62..56795f1
- ht-5fr.10 Multi-node test harness: 7f41f7d..8922549
- ht-5fr.11 Configuration smoke: 3aaeb70..40e34b8
- ht-5fr.12 Agent and user docs: 26740b3..6d63d72
- ht-5fr.13 Sync-aware deadline hold and unreachable-node marker: 787be9a..26740b3
- ht-5fr.14 Real-ssh two-node check in the Linux Docker runner (self-test only; live run pre-release): 7fab0ef..a77a8f1
- ht-5fr.15 Integration sweep: 4729922..8a4ad78 (landed; tracker closure unconfirmed, so the session closed the bead)
- ht-5fr.18 Fix: Linux process-table discovery reads only this user's processes (security): 74da622..1c140c2
- ht-5fr.22 Fix: discovery "not installed" state; sync verify re-hashes: a3a5a4d..48f6255
- ht-5fr.23 Fix: nextest groups for the failpoints test and two-daemon tests: 48f6255..07b9ad8
- ht-5fr.24 Fix: sync timed waits on a Pacer: 0890d58..e7fdaed
- ht-5fr.31 Reconcile: cross-node 1:1 auto-join authoring: eb9fb5d..85f5ea9 (fix pass, 1 parked)
- ht-5fr.32 Reconcile: auto-joined pair read floor: bb2255d..afd2cfe
- ht-5fr.33 Reconcile: handoff pair route without invitation: 9ed8b86..bb2255d
- ht-5fr.34 Reconcile: nudge-threshold refuses remote seats: 5c7a880..a8e57d5
- ht-5fr.35 Reconcile: drop pre-v47 compatibility code: 9c85b90..4ffe53e
- ht-5fr.36 Reconcile: journal oracle priority fields: 0bca725..9c85b90
- ht-5fr.37 Reconcile: multinode priority/pair/invite tests: afd2cfe..e8b7885
- ht-5fr.38 Fix: two branch test failures: 0ab53a3..4c1baa2
- ht-5fr.39 Docs: stale instances/ paths, stale dead_code allows: 40d4ed5..0ab53a3
- ht-5fr.40 Security: envelope payload parser differential: 5eb1638..e4cabe0
- ht-5fr.41 Security: sync serve resource bounds: 00c70fd..469c7d6
- ht-5fr.42 Security: unsigned Have claims do not drive evidence, restore or hold: d83676c..c300f77 (fix pass)
- ht-5fr.43 Security: message.sent apply checks author membership and recipients: 3cee07c..a67ddb0 (fix pass)
- ht-5fr.44 Test: remote-seat filters are load-bearing: c300f77..bc4341e
- ht-5fr.45 Sync coordinator robustness minors: 64cf387..1a25c31
- ht-5fr.46 Apply and startup robustness minors: e4cabe0..64cf387
- ht-5fr.49 Test: two-node auto-ACK on public display paths: 8d1fec2..e3d9c80
- ht-5fr.51 Test: mod-channel accept-grace on an ordinary invitation: e279cb1..3d280b5
- ht-5fr.53 Fix: overdue hold waits for ingested settlements: 48a6f67..2cb09fd
- ht-5fr.54 Fix: overdue hold counts unlinked dial targets: 2cb09fd..cdef941
- ht-5fr.55 Fix: loop_inventory allowlist: 0800ece..395f05f
- ht-5fr.56 Fix: silent local-only fallbacks become errors: 9d176c4..48a6f67
- ht-5fr.57 Fix: skill wording; pre-accept body flush coverage: 2d71d39..1f5697a
- ht-5fr.58 Bug: lazy body via accept then inbox never completes: cdef941..da5bfd4
- ht-5fr.60 Fix (roast r1): node-scoped context resolution: 9d0a349..98356b6
- ht-5fr.61 Fix (roast r1): credit grants never dropped: bf643e1..9d0a349
- ht-5fr.62 Sweep fix: StoreCorrupt in handoff and summary_flow: 013d350..7e558f4 (fix pass, 1 parked)
- ht-5fr.63 Sweep fix: agy canonical_v2 page tests: 2ea0893..9fc3559
- ht-5fr.64 Sweep fix: peer --stdio exits 1 on stdin EOF: 3f45dde..2ea0893
- ht-5fr.65 Sweep fix: token_diet budget for node-tagged IDs: 8a0953a..3f45dde

## Remaining

**Escalations (unresolved; 3)**
- Design roast 1, material dissent on §10 union re-issue and same-key facts: does the incarnations §13 same-ordinal limit cover invitation.answered and receipt.settled, and must root §1/§10 "every node converges" name it? [run.md parked; roast-design-1.md]
- Design roast 1, material dissent on §9 duplicate links vs cloned VMs at a third node: does standby promotion surface the fork, and should a third node raise node_collision? [run.md parked; roast-design-1.md]
- Design roast 1, material dissent on §4 payload bytes in the JSON frame vs the §9 1 MiB frame cap (payload encoding not stated in root; root §10 vs incarnations line 170) [run.md parked; roast-design-1.md]

**Final review (last super-code review: not ready)** [run.md codeBuckets.review; fix-round-1 line]
- Main v1.1.0 (3ff902c77, 7 commits) is not merged. The permission layer refused the session merge. Expect a CHANGELOG conflict, and main's subagent-binding cap in src/store/queries.rs auto-merges untested on the node schema. This merge must come before the merge to main. (final review; run.md main-merge-3 and baseAbsorbed)
- Landing order vs super-auto/inbox-injection-auto-ack (ht-794l) is undecided. A trial merge hit 9 conflicts, including src/store/control.rs, the golden SessionStart.stdout fixtures and tests/protocol/lazy_delivery.rs, plus modify/delete on three test files this branch deleted. Whichever branch lands second takes the complete_hook_context seam. (final review; friction 2026-10-16 coordination)
- about.toml lacks BSD-3-Clause for ed25519-dalek, curve25519-dalek and subtle, so the release workflow's `cargo about` step fails. This is the same item as punch-listed r1 Cargo.toml:64. (final review)
- wake-e2e.py:874, validate-package.sh:501/512 and real-ssh-e2e.py:295 match 8-character IDs, but the new tagged IDs are 11 characters. Roast-pr-2 confirmed this at Should-fix and punch-listed it. (final review; roast-pr-2)
- docs/agent-usage.md still has the old ID shape and the protocol-8 text. (final review)
- The old instances/ layout is still used in validate-native-demo.py, native-hermes-probe.py and integrations/hermes/README.md. (final review; punch list r1)
- Punch-list robustness items are open: coordinator JoinError drops, the unbounded owns_tag walk, the blocking SQLite read in IncarnationsSink, and the shared WorkerStatus slot. (final review)
- author_answer still has a local-only fallback route. (final review; code-run-5 follow-ups)
- ht-5fr.62's 900-row scan-cap test keeps direct-SQL history and needs a code comment. (final review; ledger parked)
- Final metrics are UNAVAILABLE because a model safeguard refused the ledger read. (final review; ledger Metrics: UNAVAILABLE; friction 2026-10-16 infra)
- landed≠completed is off by one: 141 vs 140, because ht-5fr.15 is logged twice. (final review; ledger Metrics: ledger-check)

**Out of scope (filtered): punch list** [roast-pr-1/2 punch-list.txt; scopeFilter-round-1 reasons]
- Cargo.toml:64, the BSD-3-Clause license allowlist (release blocker). Reason: "release license-packaging config is not a goal-named behavior".
- src/store/materialize.rs:374, unbounded owns_tag `0..=k` walk from a signed tag_k. Reason: resource-exhaustion hardening; cooperative model.
- src/sync/coordinator.rs:933-935, JoinError dropped in handshakes, so link_count drifts. Reason: panic-containment robustness.
- src/sync/incarnations/sink.rs:443, synchronous SQLite read on a runtime thread. Reason: internal structural rule, not convergence.
- scripts/validate-native-demo.py:3807, old instances/ layout. Reason: validation tooling (clusterOverride).
- src/daemon/paths.rs:190, Hermes probe and README on the old layout. Reason: tooling and docs (clusterOverride).
- src/store/journal.rs:500 (Nit), HLC above i64::MAX is clamped and the stored form diverges from its hash. Reason: unreachable bound.
- src/sync/coordinator.rs:950 (Nit), a serve-job panic stops serving the link silently. Reason: robustness.
- integrations/skill/SKILL.md:144 (Nit), "seat ID tells which node" is false after a retag. Reason: doc nuance.
- CHANGELOG.md:5 (Nit), clean-start entry is stale against main v1.1.0. Reason: release notes.
- AGENTS.md:20 (Nit), TRUST-POLICY and harness-neutral lists omit src/sync and src/node. Reason: doc list.
- scripts/wake-e2e.py:874, 8-char ID regexes (roast-pr-2 new; punch list).
- src/sync/coordinator/ingest.rs:294 (Nit), the coordinator's record_success wipes the incarnations worker's failure (roast-pr-2 new; punch list).

**Cluster dropped (shared cause left unfixed)**
- join-error. Rule: every JoinSet/JoinHandle join in the coordinator actor handles `Err(JoinError)`, logs it and undoes its bookkeeping (link count, `serving`, `branch_busy`, serve_retry), with one test per arm. [run.md scopeFilter-round-1; roast-pr-1-step-back.md]

**Roast-unreviewed changes and gaps**
- Post-loop changes 15112f108..7e558f43a touched only tests (tests/cli/peer_stdio.rs, tests/cli/topology_handoff.rs, tests/harness/agy_delivery.rs, tests/integration/summary_flow.rs, tests/integration/token_diet.rs). They were not roast-reviewed and no post-cap audit was run. [run.md post-loop changes; git diff --stat 15112f108 7e558f43a]
- The sweep did not judge the tree merged with main v1.1.0. [run.md baseAbsorbed]
- Real ssh, the pre-release wake e2e and the Linux process_table owner path have never run on this branch. [run.md; roast-pr-2 rejected docs/release.md:97 as deferred operational work]
- Roast-pr-1 dropped 6 candidates at the remainder cap; they were never seen. [roast-pr-1.md]

**Graph changes**
- L33–L38 epic narrows were parked, then applied mid-run on user approval (the user ran narrow.py). [run.md graph-change-applied]
- L39, the split of src/store/receipts.rs, is still a parked proposal and was not applied. [run.md parked]

**Empty buckets**
- codeBuckets.escalated: none. pendingRetry: none. worktreesKept: none. processSweep: 0 stopped, 0 survived. [run.md codeBuckets]

## Gotchas & surprises

- Design roast 1 changed three design decisions [roast-design-1-step-back.md; roast-design-2.md resolved list]:
  - Clones are now detected and then operator-repaired behind a §10 clone gate, instead of being healed automatically.
  - Catch-up gained a §7 own-history hold.
  - thread.direct's creator became a min-register under a new §5 order-independent-apply invariant.
- Design roast 2's punch list (restore older than the rotation trips the clone gate; own-old-incarnation suppression drops unsettled ACK obligations; restore-window same-key authoring [fix-regression]; cold-start overdue hold; memory-clone boot id; root PeerHello lacks boot) was applied as spec patches without a re-roast. [run.md roastDesignExit]
- Spec gap, blocker ht-52dq on ht-5fr.4.2: inviter-less service and operator invites on journaled threads. The run chose `inviter_kind seat|service|operator` with origin-node authority. A fact-kinds §2/§6 amendment is owed; the user approved the decision. [run.md decision; friction 2026-10-15]
- Run-2 review decisions [run.md decision; approvals]:
  - The store-only daemon re-tries its own seat.created on the first host attach and at startup (ht-5fr.4.37).
  - The thread.register actor is restored and bound to the signing node (ht-5fr.4.38, ht-5fr.4.40).
- Main-v1 reconcile [run.md main-merge-2; reconcile spec; 23894daad]:
  - Payloads adopt priority 0..9 and require_ack; invitation.created drops lazy.
  - Each seat's home node authors cross-node pair auto-join. The protocol moves to 9.
  - 57 tests were ignored pending reconcile items, then resolved by ht-5fr.31–.37.
- Overdue hold: root spec §7 ("every linked peer and every enabled dial target") overrode the peer-link sub-spec's active-links narrowing (ht-5fr.54). [run.md decision]
- Pipeline defect: when an evidence-only review finding was fixed without a new commit, the coordinator classed it BLOCKED. This happened 4 times (ht-5fr.1.2, 1.6, 2.4, 3.37) and cancelled 5 stacked tasks that were then redone (1.3, 1.5, 1.12, 2.6, ...). A local coordinator patch was refused as self-modification. [friction; ledger fix pass BLOCKED lines; run.md pipeline-defect]
- Per-merge mergeCheck is build-only, so a cross-task test break (commit_kicks fixture) surfaced only in a later task's review and became ht-5fr.3.32. [friction 2026-10-15]
- Edges added mid-run were invisible to JS graph readiness, so ht-5fr.15 was dispatched before ht-5fr.4.40 existed. [friction 2026-10-15]
- Run 2 stalled because ht-5fr.3.37 was never re-dispatched after a triage wait edge. [run.md code-run-2]
- Background security reviews filed fix beads mid-run: 3.21 (forged self-log entries became fork evidence) and 18 (cross-user /proc read). They also prompted the pre-roast beads 40–43 (parser differential, serve bounds, unsigned Have claims, message.sent checks). [friction; run.md fix-beads]
- Code roast round 1 returned Blocking at src/main.rs:117: `peer --stdio` had no default state dir, so the default ssh transport could never link. Fixed by ht-5fr.60 and pinned by tests/cli/peer_stdio.rs. [roast-pr-1.md; roast-pr-2 prior-report tracking]
- ht-5fr.62's planned fixture fix conflicted with the real delivery model (blocker ht-sjmt). The session decided that tests author history through real sends. [ledger Task 138 BLOCKED; run.md decision]
- The integration sweep found 8 failing tests at cae40a906, which became ht-5fr.62–.65; the re-run was green. [run.md sweepFix]
- Slowness: merge queue peaks in run 1, and a run-4 round-1 merge queue peak of 4 [codeBuckets.slowness]. ht-5fr.1.2 was the only task in flight for about 2 h (a 163-file sweep) while ready beads sat idle until a landing [friction 2026-10-14]. No `Profile:` or `Edge cut:` lines were recorded [ledger].

## Entrypoints

Read in this order (dependency order of the task tree, then the diff vs main: 596 files, +135715/−37581):
1. `src/node/identity.rs`, `src/node/tag.rs`: node identity, keys and tags (ht-5fr.1.1)
2. `migrations/node/0001_node_baseline.sql`, `src/daemon/paths.rs`: the clean-start schema and node layout (ht-5fr.1.2, 1.4)
3. `src/service/host_sessions.rs`: multi-host daemon (ht-5fr.2.7)
4. `src/sync/envelope.rs`, `src/sync/hlc.rs`, `src/sync/journal_port.rs`, `src/store/journal.rs`, `src/store/apply.rs`: the journal core (ht-5fr.3.x)
5. `src/sync/facts/mod.rs` and its kinds, then `src/store/materialize.rs`: fact kinds and the write path (ht-5fr.4.x)
6. `src/sync/link.rs`, `src/sync/coordinator.rs` (+ `coordinator/{ingest,routing,serve}.rs`), `src/sync/dialer.rs`, `src/sync/discovery.rs`: peer link and coordinator (ht-5fr.5.x, 7)
7. `src/store/incarnations.rs`, `src/sync/incarnations/{sink,worker}.rs`, `src/sync/resync.rs`: forks, restore and resync (ht-5fr.6.x)
8. `src/app.rs` (`run_elected_impl`): daemon composition and primary caller
9. `TRUST-POLICY.md` (N1–N5 amendment, ht-5fr.9) and `tests/multinode/` (harness and smoke, ht-5fr.10/11)

## Smells

- **ht-5fr.31 (parked)**: 14 pair-accept tests were deleted rather than rewritten. The smell: mod-channel accept-grace coverage was lost, then partly restored by ht-5fr.51 on an ordinary invitation. [ledger Task 112]
- **ht-5fr.62 (parked)**: the 900-row scan-cap test keeps direct-SQL legacy history with no message.sent facts. The smell: the fixture cannot exist under clean start; the documented exception lives in the report, not in a code comment. [ledger Task 138]
- **Fix pass merged without re-review**: ht-5fr.1.10 (check_store_identity swallowed errors), 2.8 (startup attach wiring), 31, 42 (expired unproven own-log in update_own_history), 43 (TRUST-POLICY amendment list) and 62. The smell: each second-round fix went in without a second review. [ledger fix pass FIXED lines]
- **sweepFix pass**: ht-5fr.62–.65 fixed 8 sweep failures and merged with no roast; the re-run was green at 7e558f43a. The smell: these are test-only changes outside both code roasts. [run.md sweepFix]
- **src/cli/mod.rs:615 (roast-pr-2 unverified Nit)**: ht-5fr.60 replaced the single `require_host()` gate with about 10 scattered `claim_host()` calls. The smell: no test asserts that a seat-attributed command without a host still exits 2.
- **src/sync/coordinator/serve.rs:206 (roast-pr-1 Nit)**: a failing serve logs unthrottled every second and never records a Sync lane failure.
- **src/sync/coordinator/ingest.rs:334 (roast-pr-1 Nit)**: per-link diagnostics name the peer only by an internal link_id that is printed nowhere else.
- **tests/sync/incarnations_worker.rs:995 (roast-pr-1 Nit)**: the release-kick test relies on a fixed 500 ms sleep. The smell: it can pass falsely under load.
- **src/sync/coordinator.rs:2430 (roast-pr-1 Nit)**: the fail-closed JoinError arm of `admit()` has no test, so a fail-open mutation would survive the suite.
- **src/sync/coordinator.rs:314 (roast-pr-1 Nit)**: the per-key log throttle is reimplemented at least 6 times across the sync modules.
- **src/sync/coordinator.rs:797 (roast-pr-1 Nit)**: an AtomicBool latch sits in single-task actor state.
- **ht-5fr.15 deferred minors**: timing-based negative assertion (tests/multinode/sweep.rs, 1.5 s sleep); the 250 ms sync-hold recheck has no direct test; 6 leaked temp dirs were not investigated. The smell: the integration sweep's own test hygiene. [ledger; 403 `minor (deferred)` lines in total]
- No DONE_WITH_CONCERNS implementer reports, no degraded-verdict records and no regressionPass records. [ledger task-*-report.md; run.md]
