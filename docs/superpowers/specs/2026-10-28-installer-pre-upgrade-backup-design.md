# Installer pre-upgrade backup and pruning (2.5.0)

The user: "installer should do that automatically, while cleaning up older backups as it goes".

## Today

RELEASE.md §7.1 is a manual step: a `sqlite3 .backup` copy of the store plus `node.key` and
`node.json` (directory 0700, files 0600) under
`~/.local/state/herdr/plugins/herdr-threads-pre-vX.Y.Z-backup-*`. Nothing prunes them (laptop:
23 directories, 1.5 GiB). The installer assumes `bash curl tar uname mktemp` and `shasum` or
`sha256sum`; it does not assume `sqlite3` (blacky has none) or `python3` (macOS's
`/usr/bin/python3` is a stub that prompts for the Command Line Tools).

## Design

### Where the work runs: a hidden helper in the binary

`herdr-threads internal pre-upgrade-backup --state-dir DIR --target-version TAG` (hidden, like
`internal json-field`; local only, never contacts the daemon; `--state-dir` is the existing
global flag, required here). The binary bundles SQLite
(rusqlite `bundled`), so the backup needs no `sqlite3`/`python3` on either OS, and the
policy is unit-tested in Rust against real SQLite files.

The installer runs it with the **new package's** executable (`$package/bin/herdr-threads`,
extracted and checksum-verified, not yet installed). If that build lacks the helper (an
explicit `--version` of a release before 2.5.0), it uses the installed executable when that
one has it (a rollback from >= 2.5.0). If the package's helper fails, the installed
executable's helper is tried before giving up (a rollback whose older SQLite cannot read a
newer store). If neither has it, the installer refuses (fail closed) and names `--no-backup`. Capability is probed with `internal pre-upgrade-backup
--help` (a help query, no effects), as `installer_integrations()` does today. So that a probe
failure never reads as "no helper" when the download cannot execute at all (a noexec
`TMPDIR`; roast design-1), the installer first runs the package's `--version` and, if that
fails, refuses naming the executable and `TMPDIR` (≈ 130 bytes).

### When

After the download is verified and `changed` is known, **before** the old daemon is stopped
and before anything under the prefix is touched, when `changed = 1` (a new package will be
installed: upgrade, downgrade, or install over kept state) and a state directory holds a
store. State directories: the two default roots the installer already scans for daemons
(`${XDG_STATE_HOME:-~/.local/state}/herdr/plugins/herdr-threads` and
`~/.local/state/herdr/plugins/herdr-threads`, deduplicated by physical path). A store is
`<state>/node/herdr-threads.db`: any entry at that path, a symlink included, is passed to the
helper, which refuses anything that is not a regular file, so the install fails closed
(`--no-backup` is the escape) instead of migrating a symlinked store unbacked (independent
review of 6631a0b90). Legacy `instances/*/threads.sqlite3`
stores are not backed up: the daemon never opens or migrates them, so an upgrade cannot
damage them. An identical re-install (`changed = 0`) and a fresh install with no store take
no backup.

### Naming and location (standard, owned)

The user, relayed 2026-10-28: "just derive your own standardised naming for backups, and care
for that cleanup only, disregard what we have accumulated manually so far, we are responsible
for cleaning that up". So the installer defines one name grammar in one directory it owns, and
prunes only entries that match it exactly. The manual `herdr-threads-pre-v*-backup-*`
directories beside the state directory are never read, counted or removed.

- Root: `<parent>/herdr-threads-backups/` (0700), `<parent>` the state directory's parent
  (`~/.local/state/herdr/plugins`). A sibling of the state directory rather than inside it,
  so resetting state by removing the state directory keeps the backups.
- Entry: `pre-v<TAG>-<YYYYmmddTHHMMSSZ>` (UTC, always), `<TAG>` the version being installed
  (`[0-9A-Za-z.+-]+`), with `-2`, `-3`, … appended when that name exists. Exact grammar
  (prune matches only this): `^pre-v[0-9A-Za-z.+-]+-[0-9]{8}T[0-9]{6}Z(-[0-9]+)?$`.
- Contents: `herdr-threads.db`, `node.key`, `node.json` (0600).

### The backup (helper)

1. The root is created 0700 when missing; an existing root that is a symlink or not a
   directory is refused.
2. Built in a private staging directory `<root>/.staging-<pid>` (0700; its name does not
   match the grammar, so pruning never sees it), removed on any failure. A killed run
   (SIGINT under `curl | bash`, SIGKILL, power loss) leaves one behind; each run first sweeps
   `.staging-<pid>` directories whose pid is not a live process (roast design-1).
3. Store: `VACUUM INTO` from a read-only connection to the live store (busy timeout 10 s).
   It copies one consistent snapshot (its own read transaction; SQLite refuses it inside an
   explicit one), so it is safe while the daemon runs. The copy is chmod 0600 and fsynced.
4. Check: the copy opened read-only must return exactly `ok` from `PRAGMA integrity_check`;
   its `user_version` is the schema reported and used by pruning.
5. `node.key` and `node.json`: copied byte for byte when present (`fs::copy`, never read into
   a string), chmod 0600, fsynced.
6. The staging directory is fsynced and renamed to the entry name, then the root fsynced; a
   failure of that last fsync leaves a complete backup in place and is a warning
   (`herdr-threads: warning: backup in place, but syncing its directory failed: <reason>`,
   ≈ 70 bytes + reason), not a failed backup.
7. Prints `herdr-threads: backed up <state> to <dir> (schema N, integrity ok)` (≈ 60 bytes +
   two paths). Any failure: `herdr-threads: error: pre-upgrade backup failed: <reason>` on
   stderr (≈ 50 bytes + reason), exit 1.

Never printed: `node.key`'s contents. The helper never formats key bytes; errors name paths
only.

### Pruning (helper, before the copy and after it)

It runs twice. Before the copy it keeps the newest 2 instead of 3 (the other rules
unchanged): in the steady state that removes exactly the entry the second pass would remove
once the copy lands, so a full disk can still take the copy, and it never drops the newest
backup of a schema, which is what a rollback needs (roast pr-2). After a successful copy it
runs with the rules below and the new entry. A failed copy still reports the first pass's
deletions and so leaves one entry fewer than the newest 3.

Candidates: entries of the root whose name matches the grammar exactly and that are real
directories (`symlink_metadata`; a symlink is never followed or removed).

- **time**: the UTC stamp in the name (the grammar is ours, so it is parsed; directory and
  file mtimes are unreliable, since opening a WAL copy creates sidecars and moves them). Ties
  break by the `-N` suffix, then by name, so the order is total.
- **schema**: `PRAGMA user_version` of `<entry>/herdr-threads.db`, opened read-only with
  `immutable=1` (no sidecar is created, nothing changes). Missing or unreadable: unknown.

Keep: the backup just taken (by path, unconditionally); the newest 3; the newest per schema;
every entry whose schema is unknown, with one warning each:
`herdr-threads: warning: kept <dir>: cannot read its store schema (<reason>)` (≈ 60 bytes +
path + reason). Delete the rest (`remove_dir_all`, which removes symlinks inside without
following them), printing `herdr-threads: pruned old backup <dir>` (≈ 38 bytes + path) each.
A failed deletion warns `herdr-threads: warning: could not prune <dir>: <reason>` and
continues: pruning never fails the install.

### Installer

- New flag `--no-backup`: skip the backup (and pruning). Loud: `herdr-threads: warning:
  --no-backup: no pre-upgrade backup of <state>; a one-way schema migration cannot be rolled
  back` (≈ 105 bytes + path) per store. Warranted: a full disk, or an explicit old `--version`
  where neither executable has the helper, would otherwise block the install with no way
  forward. Help text: one line (≈ 70 bytes).
- Failure (helper exit != 0, or no helper available): `die "pre-upgrade backup of <state>
  failed; nothing was stopped or replaced. Fix the cause and re-run, or pass --no-backup to
  install without a backup"` (≈ 150 bytes + path), exit 1, before any stop or replace.
- `HERDR_THREADS_NO_BACKUP` is not added: flags only, to keep the opt-out deliberate.
- Uninstall takes no backup (state is kept).

### Docs

- RELEASE.md §7.1: the installer backs up and prunes automatically; a manual backup is needed
  only for a store outside the default state roots, or with `--no-backup`.
- docs/install.md: a "Pre-upgrade backup" paragraph (location, contents, retention, opt-out,
  restore by hand: install the older version with the current installer (it backs up the
  store being replaced; `--no-backup` only if that fails, after copying `node/` aside; roast
  pr-1), stop the daemon and keep it
  from restarting, delete `node/herdr-threads.db-wal` and `-shm` (a leftover WAL replayed onto
  the restored file corrupts it; roast design-1), copy the three files into `node/`, then
  `daemon ensure`; the old manual
  `herdr-threads-pre-v*-backup-*` directories are not managed and can be removed by hand).

## Acceptance criteria

1. Upgrade with a store present: exactly one new `herdr-threads-backups/pre-v<new>-<stamp>`
   directory (0700) beside the state dir, holding `herdr-threads.db`, `node.key`, `node.json`
   (0600); the copy passes `integrity_check` and has the source's `user_version` and rows; `node.key`
   bytes equal the source's. Taken before the old daemon's stop.
2. A failing backup (unwritable backup root) exits 1 before any stop action and before the
   prefix changes; the old version stays installed and its daemon keeps running.
3. `--no-backup` installs without a backup and prints the warning.
4. A fresh install (no store) and an identical re-run take no backup and print nothing about
   one.
5. Pruning keeps the newest 3 by name stamp, the newest per schema, unknown-schema entries
   (with a warning) and the new backup; deletes the rest; never touches a symlink, a name
   off the grammar (including every manual `herdr-threads-pre-v*-backup-*` directory beside
   the state directory, which stays byte-identical) or the staging directory.
6. Reading a backup's schema creates no `-wal`/`-shm` and leaves its mtimes unchanged.
7. `node.key`'s contents never appear in installer output (success or failure).
8. The helper refuses (exit 1) when the store is unreadable or the copy fails its check, and
   leaves no staging directory behind.

## Tests

- Rust (`src/cli/pre_upgrade_backup.rs` unit tests, real SQLite under
  `test_support::scratch::temp_path`): AC 1 (helper level), 5, 6, 8.
- `tests/release/install_test.sh` (real binary, real daemon): AC 1 on the upgrade, AC 2
  (backup root made read-only), AC 3, AC 4, AC 7 (a `node.key` with a known marker).
- New `tests/release/installer_backup_test.py` (stub package, helper delegated to the real
  binary given on argv): pruning end to end through the installer, with manual
  `herdr-threads-pre-v*` directories present and untouched (AC 5), the helper-capability
  fallback and its refusal; wired into CI and `scripts/release-qualify`.

## Risks

- A store outside the default roots (custom `--state-dir`) is not found, as the daemon stop
  is not: documented.
- Disk: one more full copy per upgrade, bounded by pruning (3 + one per older schema).
- The manual `herdr-threads-pre-v*-backup-*` directories are left for the user to remove;
  docs/install.md says so.
- A backup copied by hand into `herdr-threads-backups/` under the grammar is pruned like the
  installer's own: the directory is documented as installer-owned.
- A symlinked state directory puts the backup root beside its physical target (the installer
  resolves `pwd -P`); documented.
- The staging sweep decides liveness with `kill(pid, 0)` in the helper's own PID namespace: a
  concurrent installer in another namespace sharing this state directory could have its
  staging directory removed (its rename then fails closed). Accepted: concurrent installs
  already collide on the prefix swap and daemon stop.
- Pruning trusts the UTC stamp in the name: an entry taken with a far-future clock holds one
  newest-3 slot until real time passes it. Accepted: the just-taken backup and the newest per
  schema are kept regardless.
