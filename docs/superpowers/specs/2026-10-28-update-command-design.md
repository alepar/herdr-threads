# `herdr-threads update` and the periodic update check (ht-r4eg)

Status: design, for approval. Ships as 2.7.0 (a new user-facing feature: MINOR).

## Goal

One safe command upgrades an installer-managed node: `herdr-threads update` (`ht update`,
since `ht` links the same executable). It replaces what is done by hand today: fetch the
release, back up the store, replace the package, restart the daemon, reconcile setup. A daemon-side
daily check records when a newer release exists, and text CLI commands show one short,
rate-limited line suggesting the update.

Non-goals: updating Herdr or the harnesses, updating dev checkouts or `cargo install` builds,
downgrades, a system timer.

## Decision: verify natively, then run the release's own `install.sh`

The installer already holds the hard-won upgrade logic, about 600 lines: the pre-upgrade backup
with fallback executables; the stop with the old executable before the swap; the crash-safe
package swap; the post-swap stop that catches a hook restarting the old daemon; and the ht-xe3v
loop that replaces daemons of this installation and does not replace foreign ones. It also covers
Herdr registration, `ensure` and setup reconciliation, including aisw profiles. Porting it to
Rust forks that logic, and every future installer fix would then have to land twice. So `update`
does only what the installer cannot do safely itself (resolve, download, verify twice, preflight,
keep the previous package, judge the result, roll back). Everything that changes the
installation goes through `install.sh`.

**Which `install.sh`.** The release archive does not contain one today. From 2.7.0,
`scripts/package-release.sh` adds `scripts/install.sh` to the archive, so the
script is covered by `SHA256SUMS` and the GitHub asset digest like the executable. `update`
runs the script from the verified, extracted target archive. Every target `update` accepts is
newer than the running version, which is at least 2.7.0, so its archive has the script. A
target archive without it is refused.

**Trust story.** Running that script adds no trust beyond what any upgrade already needs. It
comes from the same verified archive as the executable that will run the daemon with the
same privileges a minute later. It is pinned to the release, unlike `curl .../main/scripts/install.sh | bash`, which runs
whatever `main` holds, unverified. It is not fetched separately from `raw.githubusercontent.com`, which no
checksum covers. And it carries the target version's own upgrade knowledge, not the knowledge of the
release being replaced (an `include_str!` of the running version's script would do the reverse).

Rejected: a native Rust port (the fork above); the tag's script from raw GitHub (unverified);
the running version's embedded script (its upgrade knowledge is stale for the target it installs).

## Command

```text
herdr-threads update [--version vX.Y.Z] [--dry-run | --check]
                     [--setup | --no-setup] [--without-permissions] [--no-aisw-profiles]
                     [--no-herdr]
```

- `--version` accepts `vX.Y.Z` or `X.Y.Z`. clap's automatic `--version` belongs to the
  top-level command only (no `propagate_version`), so a subcommand option of that name is free;
  the implementation checks this with a test.
- **Install-time opt-outs are remembered** (roast design-1). From 2.7.0 `install.sh` records
  `--no-herdr`, `--without-permissions` and `--no-aisw-profiles` in its marker
  (`herdr=0`, `permissions=0`, `aisw_profiles=0`), and `update` replays them, so a plain update
  never reverses an operator's choice. A flag given to `update` wins over the recorded one and
  is recorded in turn. A marker written before 2.7.0 records nothing, so the first `update`
  from such an install needs the opt-outs repeated by hand; docs/install.md says so.
- The setup flags pass through unchanged to `install.sh`. Default: owned integrations update
  silently and missing ones are not added. `install.sh` runs with its output captured, so it has
  no terminal and asks nothing; `--setup` adds missing integrations. `update` therefore never
  prompts, and an agent can run it.
- No `--no-backup`: `update` always backs up and fails closed. The installer keeps its flag for
  manual cases.
- `--check` runs the update check now (below), prints the result and changes nothing else.

## Flow

1. **Locate the installation.** Resolve `current_exe()` physically. It must be
   `P/bin/herdr-threads` with the installer marker `P/.herdr-threads-install`. Otherwise
   refuse: `not an installer-managed install (<path>); use scripts/install.sh`. The bin
   directory comes from `bin_dir=` in the marker, which `install.sh` writes from 2.7.0; a marker
   without it means the default `~/.local/bin`.
2. **Lock.** Take an exclusive `flock` on `P.update.lock`, a sibling of P. A second concurrent
   `update` refuses at once with `another update is running`.
3. **Resolve the target.** `curl -fsS --max-time 10` fetches
   `https://api.github.com/repos/alepar/herdr-threads/releases/latest` or `.../releases/tags/vX.Y.Z`.
   The response must not be a draft or prerelease. It supplies `tag_name` and, for each asset,
   `name`, `digest` (`sha256:<hex>`) and `browser_download_url`. Tests point
   `HERDR_THREADS_RELEASE_API` (default: the URL above, without `/latest`) at a `file://` tree
   whose JSON names `file://` assets. That is the only test seam, and it uses the same code path.
4. **Compare versions** (semver, numeric). Equal: `up to date: 2.7.0`, exit 0. Lower: refuse
   with `downgrade refused`, exit 1, and no `--force`. A downgrade is the only way to cross a one-way schema migration
   backwards, and the store refuses a schema newer than the binary
   (`store is newer than this binary`), so refusing downgrades is the one-way-schema refusal.
   The documented manual restore (docs/install.md, "Pre-upgrade backup") stays the way back.
5. **Download** `herdr-threads-<os>-<arch>.tar.gz` and `SHA256SUMS` into a fresh work directory
   beside P, `<parent(P)>/.herdr-threads-update.XXXXXX`. That keeps it on P's filesystem and
   avoids the noexec `/tmp` case the installer warns about. The work directory is removed on
   every exit, except a work directory whose rollback is needed (step 10).
6. **Verify.** Every check below must pass; any mismatch is a refusal with nothing changed:
   - `sha256(SHA256SUMS)` equals the API digest of the `SHA256SUMS` asset;
   - `sha256(archive)` equals both the `SHA256SUMS` line for the asset and the asset's API
     digest;
   - extraction with the system `tar`, the same tool the installer uses, into an empty
     directory gives one top-level `herdr-threads/` directory;
   - `VERSION` equals the tag without `v`, and `TARGET` equals `<os>-<arch>`;
   - `bin/herdr-threads --version` runs and prints that version;
   - `scripts/install.sh` is present.
   A missing API digest also refuses. GitHub records digests for current uploads (checked live
   for v2.6.3), and every target `update` accepts is 2.7.0 or newer.
7. **Preflight** (read-only). For each state directory with a node store under the installer's
   default roots (the same `store_states` scan, done in Rust):
   - record `PRAGMA user_version` through a read-only connection;
   - record whether a daemon runs (from `endpoint.json` and a live pid) and its executable;
     a daemon is **owned** only by the installer's rule (`owned_daemon`: started from P or
     `P.old`). A foreign daemon (a dev checkout, another prefix) is recorded as foreign and
     never judged (roast design-1). It is not necessarily left running: the installer's
     pre-swap `stop_with_installed` stops every live daemon under the default state roots,
     whoever owns it, and only its post-swap replace loop checks ownership. `--dry-run` and
     the result name each foreign daemon that will be (or was) stopped and not restarted
     (review 2);
   - where an `endpoint.json` exists, run `daemon ensure` with this executable first. After
     ht-tyt7, ensure heals a dead boot whose socket device was renumbered, and does so
     silently. Then check that this executable reaches the daemon.
   A remaining `socket pathname was substituted` error (ht-tyt7: the inode changed too, or a
   live owner has a mismatched socket) stops the update with nothing changed. The output
   prints the error, whose recovery text names the `endpoint.json` to remove. It is matched
   with `daemon::ownership::is_socket_substituted` once ht-tyt7 is merged, and by its stable
   message prefix until then. An update into a daemon that cannot start helps nobody.
8. **`--dry-run` stops here** and prints the plan (Output).
9. **Journal and keep the previous package.** Write `P.update.json` (target, previous
   version, work directory, recorded `user_version`s and owned daemons) by atomic rename. Copy P
   to `P.prev.tmp`. Rename the existing `P.prev`, if any, to `P.prev.old`, rename `P.prev.tmp`
   to `P.prev`, then remove `P.prev.old`. A rename never targets a non-empty directory (review
   2), and an interrupted copy never replaces a good `P.prev`. Leftover `.tmp` and `.old`
   siblings from a killed run are removed at the start of this step. `install.sh --uninstall` removes `P.prev`, `P.update.json` and `P.update.lock` from 2.7.0 on.
10. **Install.** Lay the verified archive and `SHA256SUMS` out as a `file://` mirror in the work
    directory (`download/vX.Y.Z/...`, the layout `HERDR_THREADS_RELEASE_URL` already accepts)
    and run `<extracted>/scripts/install.sh --version vX.Y.Z --release-url file://<mirror>
    --prefix P --bin-dir B <pass-through and replayed flags>` with `TMPDIR=<work dir>`, so the
    installer's own extraction also runs on P's filesystem and not on a noexec `/tmp` (roast
    design-1). Its stdout and stderr go to `P.prev/update.log`.
    It re-verifies the checksum, backs up every store (fail closed), stops the old daemon with the old
    executable, swaps, runs the ht-xe3v verify and replace loop, ensures and reconciles setup.
    Nothing in it is reimplemented.
11. **Judge** with `update`'s own check, independent of the script's exit status:
    - `P/.herdr-threads-install` says the target version;
    - `P/bin/herdr-threads --version` prints it;
    - each state directory that had a running **owned** daemon at preflight has one now,
      running the target version (`daemon ensure` with the new executable, which does the version and
      boot handshake);
    - each state directory with a store whose daemon was **not** running at preflight is
      opened by the new executable at the target schema, with no daemon: a new hidden
      `internal store-check --state-dir S`. It runs the migration now, while rollback is still
      possible, instead of at the next start (review 2). `daemon ensure` cannot do this: it
      needs a reachable Herdr host. The directory therefore stays daemon-less, as it was
      (review 2 follow-up).
    The script's exit 3 for setup alone (`setup incomplete`, not linked) is reported but does
    not fail the update. The binary and the daemon are what an update changes.
12. **Roll back or report** (Rollback below).
13. **Finish.** Write the check state (below) with `latest = target`, print the result,
    remove `P.update.json` and the work directory.

**Interruption** (roast design-1). From step 9 to step 13, `update` ignores SIGINT, SIGHUP and
SIGTERM and runs `install.sh` in its own session (`setsid`), so a terminal Ctrl-C, a closed
pane or an agent's tool timeout cannot stop it between the backup and the judge. A `kill -9`
still can. The `flock` of step 2 stays close-on-exec: inherited by `install.sh`, it would also
pass to every daemon the installer starts and hold the lock for the daemon's lifetime (roast
pr-1). Instead the journal records the installer's pid, which leads its own process group
(`setsid`). An `update` resuming a journal whose installer group is still alive waits for it,
bounded at 120 s, then refuses with `an earlier update's installer is still running (pid N)`
(review 2). Children get the default SIGINT, SIGHUP and SIGTERM dispositions back, so a daemon
started during the update stays stoppable (roast pr-1). The journal covers the rest: any `update` (including `--check` and `--dry-run`,
which only report it) that finds `P.update.json` under the lock first finishes the earlier
run, judging and then completing or rolling back, before it compares versions. A retry
after an interrupted run therefore never prints "up to date" over an unjudged install.

## Running agents during the update

- **Daemon gap.** The daemon is down from `install.sh`'s first stop to its `ensure`, normally a
  few seconds. In that window the store is durable and nothing is lost.
  - A CLI call or hook may fail once. Hooks are best effort, the next turn's hook delivers, and
    re-offers are bounded by the ledger.
  - A client's `ensure` may restart the daemon early: the old one from a pre-swap client, which
    the ht-xe3v loop replaces, or the new one, which is fine.
  - The Claude mod's watch reconnects with its backoff (1, 2, 5, 10, 30 s, `register.js`).
- **Old code in live sessions.** Running Claude sessions keep the old mod JS until they restart.
  Old mod and old hook clients talk to the new daemon over the version-tolerant wire
  (`check-wire-compat`). Setup reconciliation updates the files on disk for new sessions.
- **Swap.** The installer's swap stays as it is: `mv P P.old; mv P.new P`, two renames on one
  filesystem. That leaves a sub-millisecond window in which `P/bin/herdr-threads` does not
  exist, and a hook that hits it fails once, like any call during the daemon gap. An exchange
  syscall layer (`renameat2` and `renamex_np`) is not worth it for that window (review 2). A
  running daemon keeps its open inode (the ht-xe3v concern, unchanged).

## Rollback

What `update` does on its own, with no question asked:

| Failure | State | Automatic action |
|---|---|---|
| Steps 1-8 (resolve, download, verify, preflight) | nothing changed | none; reason printed |
| `install.sh` fails before the swap, nothing stopped (backup refused) | P untouched (the judge sees the old marker) | none; reason and log path |
| `install.sh` fails before the swap after stopping the daemon (for example ENOSPC copying `P.new`) | P untouched, daemon down | `daemon ensure` with the old executable for each state directory that had an owned daemon; the output says the daemon was stopped and restarted |
| `install.sh` fails in the swap | the installer itself restores `P.old` | none beyond it |
| Judge fails after the swap, and every recorded `user_version` is unchanged | new package, old schema | **binary rollback** |
| Judge fails, and a `user_version` advanced (one-way migration ran) | new package, new schema | **none**; restore steps printed |

**Binary rollback.** Pack `P.prev` into a `file://` mirror as
`herdr-threads-<os>-<arch>.tar.gz` plus its `SHA256SUMS`. Run the **new** archive's
`install.sh --version v<prev>` against that mirror. It backs up the store again (harmless),
stops the new daemon with the new executable (the right direction across a wire change),
swaps, verifies and reconciles setup back with the old executable. `update` re-reads every
`user_version` just before starting the rollback. A client `ensure` can still start the new
executable and migrate inside the rollback's own stop-to-swap window. If the rollback's judge
then fails with the store newer than the old binary, `update` falls to the restore branch
below (roast design-1). Judge again with the previous version as the target. Success prints `rolled back to <prev>`, exit 1. If the
rollback also fails, `update` prints both log paths and the manual steps, exit 1.

**Why no automatic store restore.** Restoring the backup discards every message written after
the backup, while live agents may have kept writing to the new daemon. Discarding data is the
operator's decision. `update` prints the exact backup entry, the one the installer named as
`pre-v<target>-<stamp>`, and points to the restore procedure in docs/install.md.

`P.prev` is never deleted by `update` itself. It is the previous binary the brief asks to keep,
about 15 MB.

## Periodic update check

**Where it runs.** The check runs inside the daemon, not on a system timer and not from the
CLI. That meets "a CLI command runs the check on a schedule" this way: the daemon runs the
same code as `update --check` daily, and `update --check` is the CLI entry for running it
by hand. The daemon is long-lived on every node; a CLI-triggered background fetch would need a
detached child from arbitrary commands, which leaks processes in tests and on short-lived calls.
The pattern already exists: the harness manifest fetch, a detached `curl --max-time` at most
every 24 h, cached in the node directory and switched off by a setting or by
`HERDR_THREADS_OFFLINE`. The update check reuses that fetch path and policy shape.

- **Cadence.** The check is due 24 h after the last persisted attempt. At daemon start it
  runs after 30 s when already due, otherwise a timer fires at the due time. A daemon that
  idle-exits after 10 minutes (`IDLE_EXIT_AFTER`) therefore still checks on its next start
  once a day has passed, and restarts never re-check early (roast design-1). One GET of `releases/latest` per node per day stays well under
  GitHub's unauthenticated limit of 60 per hour per IP.
- **Network failure.** The request is bounded (`--max-time 10`) and runs off every request path
  in a detached thread. Failure records only `attempted_at`, so the next try comes in 24 h. A
  missing `curl` counts as offline. Nothing is ever logged above debug level, and nothing is
  shown to anyone.
- **Eligibility.** The check runs only when the daemon's own executable is installer-managed
  (marker present). Dev checkouts, CI, the test suite and `cargo install` builds never check.
- **Off switch.** `settings.json` takes `"update_check": "auto" | "off"`, read at daemon start
  like `harness_manifest`. `HERDR_THREADS_OFFLINE=1` in the daemon's environment also turns it
  off. `doctor` reports the policy and the last check. Because of `deny_unknown_fields`, a node
  that sets the key cannot run older releases, but downgrades are refused anyway. This is noted
  in docs/install.md.
- **`update --check`** runs the same check in the foreground (bounded 10 s), prints
  `2.7.1 available (installed 2.7.0): herdr-threads update` or `up to date: 2.7.0`, and writes
  the state.

**State: a node-local file, not the store.** The file is `<node_dir>/update-check.json`,
`{"attempted_at_ms", "checked_at_ms", "latest", "notified": {"<key>": ms}}`, written mode
0600 by atomic rename. `<key>` names the caller locally, with no daemon round trip: for an
agent, `<harness>:<native session id>` (one session holds one seat); for a person, the pane
(`HERDR_PANE_ID`); `-` when there is neither. Entries older than 24 h are pruned on each
write. It is advisory and node-local; it must not ride peer sync or need a schema
migration, and a lost or corrupt file only means "unknown". Concurrent writers (the daemon's
check, the CLI's `notified` entries) are last-writer-wins on a tiny advisory file. The worst
case is one extra notice.

**Invalidation.** The notice shows only while `latest` is newer than the reading executable's
own `CARGO_PKG_VERSION`. An update therefore silences it by itself, with no write. `update`
also writes `latest = target`.

**The notice.** One line written to the command's stdout writer before its own output, the
same way as the pending hint. When both show, the notice comes first: it is written as soon
as the caller is known, and the pending hint later, during dispatch:

```text
[herdr-threads] 2.7.1 available: herdr-threads update
```

That is 53 bytes plus a newline for a five-character version (AGENTS.md (f)). It is shown only
when all of these hold:

- the check is not off: the CLI reads the same `update_check` setting and
  `HERDR_THREADS_OFFLINE`, so "off" silences the notice as well as the fetch (roast design-1);
- the gate is exactly the pending hint's: format text and presentation not `Machine`. An
  agent's non-tty call is presentation `Auto`, not `Machine`, so agents do see it. Explicit
  `--machine` and `--json` never do (review 2: the user asked for it "the same way" as the
  pending hint);
- the caller is top-level or a person, never a subagent;
- the command is not `hook`, `internal`, `daemon run`, `watch`, a delivery or check-in path,
  or `update` itself;
- at most once per 24 h **per caller** (`notified[<key>]`), so the first agent does not use
  up the person's notice (review 2).

Its cost is one small file read on the local disk: no daemon round trip and no network. A
missing or unreadable file prints nothing. Hook output, delivery output and subagents never
see it.

**Skill line.** `integrations/skill/SKILL.md` gains one line, loaded once per session with the
skill, about 110 B:

```text
An "N available: herdr-threads update" line is informational: tell your user; run update only if asked.
```

Model-visible cost per agent: at most 54 B a day, plus the skill line once per session.

## Output

`update` writes plain lines, harness-neutral and without color, each with the `herdr-threads: ` prefix
the installer uses. Sizes below are without paths.

- Success (2 lines, about 110 B plus paths):
  `herdr-threads: updated 2.6.3 -> 2.7.0 (sha256 verified twice; backup <entry>)` and
  `herdr-threads: daemon 2.7.0 running (<n> state dir(s)); previous package kept in <P.prev>`.
- Setup incomplete (exit 3): the success lines, then the installer's final status line verbatim
  and `log: <path>`.
- Up to date: `herdr-threads: up to date: 2.7.0` (31 B).
- Refusal (exit 1): one `herdr-threads: error: <reason>` line, plus `nothing changed`.
- Rolled back (exit 1): `herdr-threads: error: <reason>; rolled back to 2.6.3; log: <path>`.
- No rollback (exit 1): the reason, the backup entry and
  `restore: docs/install.md#pre-upgrade-backup`, plus the log path.
- `--dry-run` (exit 0, one line per step and per daemon):

  ```text
  herdr-threads: would update 2.6.3 -> 2.7.0 (linux-x86_64, sha256 <hex> verified)
  herdr-threads: would back up <state> (schema 4)
  herdr-threads: would restart daemon pid 12345 (<state>)
  herdr-threads: would stop foreign daemon pid 23456 (<state>, runs <exe>); not restarted
  herdr-threads: would reconcile owned harness integrations (no missing ones added; --setup adds)
  herdr-threads: would keep <P> as <P.prev>
  herdr-threads: dry run; nothing changed
  ```

  `--dry-run` downloads and verifies, because otherwise "verified" would be a guess. It writes
  only its own work directory, which is removed.

Exit statuses follow the installer: 0 updated or up to date, 3 updated but incomplete (setup or
link), 1 refused, failed or rolled back.

## macOS

- **APFS `st_dev` (ht-tyt7).** The preflight in step 7 refuses with nothing changed, as above.
  After the ht-tyt7 fix, a dead-boot descriptor heals in `ensure` and the preflight passes.
  Coordination with the laptop-bugs tab (seat snSGnh4zLHdh) is open on whether to key on an
  error kind rather than text.
- **Swap.** `renamex_np(RENAME_SWAP)` works on APFS. The fallback covers HFS+ and network
  volumes.
- **Platform.** `uname`-equivalent detection gives `macos-aarch64` or `macos-x86_64`, matched
  against `TARGET`.
- **Quarantine.** Downloads made by `curl` carry no quarantine attribute, the same as the
  installer path.

Linux behaves the same, with `renameat2`. The installer's experimental-Linux warning stays the
installer's.

## Changes outside the new command

- `scripts/package-release.sh`: add `scripts/install.sh` to the archive. Update
  `validate-package.sh` and the release workflow's content checks if they list files.
- `scripts/install.sh`:
  - write `bin_dir=` and the opt-outs (`herdr=0`, `permissions=0`, `aisw_profiles=0`) to
    the marker;
  - remove `P.prev`, `P.update.json` and `P.update.lock` on `--uninstall`.
- `src/cli/`:
  - an `update` module: resolve, verify, preflight, run, judge, roll back;
  - an `update_check` module: state file, notice, and the shared fetch with the daemon;
- Daemon: schedule the check, following `ManifestService`, plus a `settings.json` key and a
  `doctor` line.
- docs/install.md: an "Update" section, the setting, and the notice. README: one line.

No wire, store schema or harness-adapter change is needed, and `check-harness-neutral` is
unaffected.

## Testing

None of these tests touches the network, the real config or the real store. Every
installation, HOME and state directory is a scratch directory, created through
`test_support::scratch`.

- **Unit:**
  - version parsing and comparison;
  - parsing the release JSON (draft, prerelease, missing digest);
  - matching `SHA256SUMS` lines;
  - each verification refusal;
  - notice gating (format, role, command, rate limit, version comparison);
  - the check cadence and state file round trip;
- **Integration** (`tests/`, through `HERDR_THREADS_RELEASE_API` and a `file://` release tree;
  how the "newer" package is produced follows `tests/release/install_test.sh`):
  - update from an installed old package to a new one with `--no-herdr`-style isolation (no
    Herdr on PATH);
  - up to date;
  - downgrade refusal;
  - each digest mismatch;
  - `--dry-run` changes nothing (tree hash before and after);
  - the lock;
  - an interrupted run, killed after the swap, is finished by the next `update`;
  - recorded opt-outs are replayed;
  - a foreign daemon does not fail the judge, and `--dry-run` names it as stopped;
  - a second `P.prev` rotation (the second update) succeeds;
  - an orphaned installer holding the inherited lock makes the next `update` wait, then refuse;
  - a state directory without a running daemon is ensured and judged;
  - notice gating: `Auto` shows, `Machine` and `--json` do not; per-seat rate limit;
  - automatic binary rollback, when a target whose executable fails `--version` after the swap
    is simulated;
  - the no-rollback report after a simulated schema advance.
- **Installer:** the existing `tests/release/*` installer suites stay green with the archive and
  marker changes.
- **Release (manual):** before tagging 2.7.0, a real `update` from 2.6.3 is impossible, since
  2.6.3 has no `update`. The first real `update` is 2.7.0 to 2.7.1. For 2.7.0,
  `RELEASE.md`'s install step still uses `install.sh`. Its record notes that the update path is
  covered only by the `file://` integration tests until 2.7.1.

## Risks

- **GitHub API availability or rate limit.** A failure refuses (`update`) or stays silent (the
  check). There is no fallback to unverified downloads.
- **Script interface coupling.** `update` 2.7.x drives the target's `install.sh` flags. A future
  installer flag rename breaks older `update`s, so the flags `update` uses (`--version`,
  `--release-url`, `--prefix`, `--bin-dir`, the setup flags) become a stable interface, noted in
  the script's header.
- **First-release gap.** As above, the first real upgrade through `update` is 2.7.0 to 2.7.1.
- **Notice cost.** Agents see the notice: at most 54 B per seat per day, plus the skill line.
