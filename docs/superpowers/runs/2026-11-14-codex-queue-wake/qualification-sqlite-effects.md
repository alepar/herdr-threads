# Isolated populated SQLite helper effects

Astra accepted this bounded experiment in message mnSGTLJSFggR (design thread227). The [aggregate comparisons](sqlite-effects/aggregate.json) and [probe](sqlite-effects/probe.py) retain aggregate evidence only; the probe has the evidence-only privacy redaction described below; no copied database, receiver routing file, credentials, config or session files remain. The [initial namespace failure](sqlite-effects/initial-failure.json) is preserved as a failed pre-protocol attempt. Its helper exited because bwrap could not create a destination under the read-only root; no-database startup probes established the corrected masked-home mount layout before repetition.

The exact receiver was identified privately by matching the current session writer-lock FD and one queue database identity. The captured executable device/inode was rechecked before use; its content hash is in the aggregate. Historical --version diagnostic reported codex-cli0.167.0; this is evidence, not an operational gate. Each of six existing SQLite databases was backed up independently using SQLite mode=ro/backup while live writers could continue. The backups were consistent individually, not one atomic cross-database snapshot. The live source was never checkpointed, repaired or passed to a helper; no claim is made that its contents stayed stationary.

The helper ran against private on-disk backups and an empty private HOME/CODEX_HOME. bwrap disabled network, isolated PID visibility and masked the real user home, user runtime sockets and temporary directory. The captured executable inode was bind-mounted under an isolated alias, and the copied database/home paths used a private namespace alias. Environment, initialize payload, content-free queue marker and bounded SIGKILL cleanup mirrored production; these namespace/path differences are explicit isolation adaptations. This did not establish production canonical enrollment or receiver loadedness. No load/resume/start/model/auth operation ran. The existing copied receiver metadata was addressed only inside private copies; correlation UUIDs were generated.

Three successful cycles each performed initialize/list/add/list/delete in0.766/0.757/0.758seconds, then killed/reaped the owned group. All15calls succeeded. The snapshots initially contained7,765 state threads,59,257 history items and28,924 log rows, so the comparison was not an empty-database fixture. Actual observed FD families included goals,logs,memories,queue,state and their WAL/SHM files. History was copied and compared but not observed open during sampling.

Across all three comparisons, all six schema hashes were identical. Every table count and multiset hash in the five nonqueue databases stayed identical, including backfill_state and maintenance/job aggregates. Only queue queued_thread_revisions and sqlite_sequence changed; queued_items ended empty after deletion. No logs writes were observed. This is evidence for these short successful control paths on this populated snapshot; it is not proof of queue-only access, future-version behavior, migration safety for every prior schema, deferred maintenance behavior, or all error/timeout paths.

Source already preserved in [control-source-inspection.json](control-source-inspection.json) shows the shared sqlite_home owns logs/goals/memories/queue/state/history families and full runtime opening can migrate schemas. A full app-server helper therefore has broader potential database effects than queue APIs alone. The design now states that fact; queue identity verification does not constrain every other opened database. Future detected breakage must degrade the affected capability, not rely on an exact-version gate.

All private copies and routing metadata were removed, owned helper groups killed/reaped, and scoped process check8475f025-ee34-49e3-9aca-2c950750255d passed. The experiment did not create a temporary-file ledger; absence of its private directories was checked directly. The failed initial run85009b9e also had clean process checking and explicit private-copy removal.

## Evidence-script privacy correction

Main65 requested removal of the operator home literal before public integration.
The preserved script now uses `/tmp/ht-effect` inside its already isolated tmpfs,
and derives the real-home mask from `Path.home()` at runtime. This is a path-only
edit to the preserved evidence script, not a rerun or a runtime implementation
change. The original successful experiment remains attributed to its historical
script hash; the current script is not represented as the exact executed bytes.
[Privacy manifest](qualification-sqlite-effects-privacy.json) records both script
hashes and the unchanged aggregate/initial-failure hashes. Python syntax and the
exact replacement-only diff were checked without invoking the experiment. The
final9779-test PASS remains attributed to source7090.
