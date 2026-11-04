Review findings (coordinator independent review @ cb122a5ca, d5pFwv0Z4Ps mnSGteUh6u5F) — verbatim source for beads ht-8lem.49+

BLOCKING 1. --profile-dir is never applied on a Herdr target. handoff_directory freezes env {CLAUDE_CONFIG_DIR|CODEX_HOME|HERMES_HOME: D}; launch.rs:784-795 puts it only into adapter environment.declared / explicit_profile_directory (hook admission runs against D), but NativeLaunchRequest (ports.rs:1970) has no env and agent.start sends name/kind/pane_id/args only ("native host carries argv only", launch.rs:1030). Remote --new-tab target create uses env: Default, directory_context: None. Result: `handoff --pane P@M --profile-dir /x` passes the hook check against /x, reports started, agent runs on the shell's default root. Only hc is right (gate_argv config_root). Tests check only the recorded LaunchRequest. Fix: remote --new-tab/--new-terminal carries the env into the target create (as local CreateTabRequest.env does); remote --pane with --profile-dir is refused, matching local guarded panes. Add a test asserting the env the started process/tab actually receives.

SHOULD-FIX 2. Launched-but-unreplicated row loses its overdue (handoff_obligations.rs:213-230, 336-341; peer_handoff.rs:666 settle(.., 0)). CLI poll times out -> row sent -> alert pending at deadline; background re-send gets Launched{seat} -> launched() sets seat, alert withdrawn; replica_wait 0 -> NotReplicated, open_pair skipped; target then offline -> due_step needs pair_thread_held (false), no alert (seat set), no peer list line (ledger_rows shows only sent). Sender hears nothing until 6-day expiry: the exact target-offline case the ledger is for. Keep the sender warned until the in-thread warning can be written (alert predicate stays true while the pair thread is not held, or show launched-overdue rows in peer list).

SHOULD-FIX 3. due_rows starvation (handoff_obligations.rs:503, HANDOFF_DUE_BATCH_LIMIT 16) is real given 2: rows with seat set, thread not held (or a retired author whose pair never opened) match every tick for 6 days, sort first; 16 of them starve every other handoff's overdue. Exclude rows due_step cannot advance, or page past them.

OPTIONAL (fix if cheap, else bead):
- peer_launch.rs:291 request_key tag v4 orphans pending 2.x plain `launch --pane P@M` ids (new id after upgrade); keep v3 for non-handoff plans.
- due_pass/expire_due/resend_pass stop on the first row error (?), one bad row halts all expiry/resends; isolate per row.
- prune only at daemon startup (peer_handoff.rs:606); long-lived daemon keeps expired/failed rows and peer list repeats them.
- materialize.rs:358 node_key LIMIT 1 no ORDER BY vs peer_launcher.rs:598 any-incarnation-newest-first: one shared helper.
- peer_handoff.rs:97-104 stage() idempotent early return skips the seat-mismatch check; move it above.
- peer_launch.rs ~742 Launched with handoff_message None prints `message=`; commands.rs to_string_lossy on profile/cwd: refuse non-UTF-8.
