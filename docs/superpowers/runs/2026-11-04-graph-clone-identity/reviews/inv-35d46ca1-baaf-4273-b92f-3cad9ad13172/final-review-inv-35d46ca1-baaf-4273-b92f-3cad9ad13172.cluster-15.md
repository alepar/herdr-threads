# Final review, cluster 15 (src-host), epic ht-ma6i

Source: base cf13ed76c1c2871b321ad290d03229cb785c400e, head 1b058307490f29ece8bb4e3fe7a862f8356d992f, both verified with `git rev-parse --verify`. Caller checkout observed at the head SHA with a clean status. Read-only: no cargo run, no worktree created; `git show` and `git grep` at the head only.

Cluster input: `src/host/mod.rs` (slice 000072), five added lines from commit 7a87f5209 (ht-ma6i.6). It adds `pub const CONTINUITY_KEY_ENV` and chains it into `child_env_scrub_list`.

## Baseline trace inside this cluster

- B1 (continuity key): this cluster supplies the variable's one name and its place in the one scrub list, which delivers spec 4.2 "threads' own `launch` and `handoff` scrub the variable from what they start". The read side (`CallerEnv::continuity_key`, the hook, the watch, the Claude mod, the Hermes bridge) leaves this cluster and is the seam reviewer's to trace.
- B2 (pane close retires, undo reverses), B3 (agent exit keeps the seat): nothing in this cluster touches them.
- B4 (Herdr unchanged): the change is a local constant and a local list; nothing in Herdr changes.

No baseline row is undelivered inside this cluster.

## Scrub reachability from production

Each route by which threads starts a process that could run an agent was traced at the head:

- Detached daemon: `spawn_detached` applies `child_env_scrub_list` (`src/daemon/lifecycle.rs:458`) before the `daemon-detach` hop, and that hop inherits the scrubbed env. `detached_daemon_environ_drops_child_env_scrub_list` reads the real `/proc/<pid>/environ` and iterates the live list, with the variable also named in `SCRUB_NAMES_PROBE`.
- Launch-gate shim: `scrub_names` starts from `gate_shim_scrub_list` (`src/cli/launch_gate.rs:150`). The filter in `gate_shim_scrub_list` removes only `hc::ENV_VARS`, so the key stays. The shim's `HERDR_*` ambient filter covers it a second time; the list entry is what holds under an empty ambient, and `handoff_scrubs_the_continuity_key` pins that case.
- Peer launch child: `env_clear` plus `child_env` (`src/service/peer_launcher.rs:77`). The name is in neither `KEPT_ENV` nor any adapter's setup inputs.
- Herdr `agent.start`: argv only. Herdr `tab.create` and hc `terminal.run` do carry an `env`, so that claim needed checking. `payload.env` and `directory_context.env` come only from the adapter's `explicit_directory_env` (`src/cli/topology_handoff.rs:146`, `:456`, `:711`), never from the caller's environment.
- Remaining spawn sites: `SystemShellProbe` (`src/harness/launch.rs:68`), `bounded_output_by` (`src/harness/process.rs:88`), the updater's installer (`src/cli/update.rs:784`), the sync dialer and the machine lister. They run probes, an installer or transports, not agents. A daemon child also inherits the daemon's scrubbed env in the autostart case. A descendant in the caller's own pane would present the same key on the same pane, which spec 4.2 records as harmless.

## Code checks

- `[&CONTINUITY_KEY_ENV]` chains as `&&'static str` like the three slices before it, and the reference to a const is promoted to `'static`.
- The name is not a harness name, so the neutral core rule holds. `src/cli/caller_location.rs:45` re-exports it rather than copying the literal, and `lifecycle_holds_no_host_env_literal` guards the lifecycle file. The other in-tree literals are in JavaScript and Python integrations, which cannot share a Rust constant.
- The list keeps its no-duplicates property (`child_env_scrub_list_is_the_union`, whose dedup assertion runs over the whole list).
- Removing the new `chain` is killed by `launch_scrubs_the_continuity_key`, `handoff_scrubs_the_continuity_key` and the daemon environ test, because `SCRUB_NAMES_PROBE` sets the variable while the fixture asserts over the live list only; the first two are the effective kills.
- TRUST-POLICY.md (`Key inheritance outside threads`, ht-ma6i.6) states the same routes traced above.

## Verdict
ready

## Must fix before landing
- none

## Untested scope
- This review ran no tests at the head; the tests named above were read, not rerun, and the ledger defers the full-suite measurement to the caller.
- Whether a live harness filters the variable before its hook sees it is outside this run by spec 4.2, which assigns it to the source design's section 6 real-Herdr qualification.

## Deferred OK
- `src/host/mod.rs:27` and `:30` read slightly wider than the code: "nothing threads starts inherits it" holds for agents but not for the CLI's own shell and version probes or the updater's installer, and "the union of each host family's names" no longer describes a list that also holds a non-host name. Neither changes behaviour; a one-line rewording of each closes it.
