# Harness version evidence: frictionless harness upgrades

Status: draft (2026-10-02). Lands after B6 (`ht-p03.13` admission ladder, `ht-p03.23` version honesty, `ht-p03.14`
canary) merges into `main`; builds on those, does not duplicate them.

## Goal

A routine Claude Code or Codex upgrade never produces a degraded state or any Health noise. herdr-threads
reports a harness version as broken only when it has observed that version break the hook payload contract (or
the published canary has, for a version never seen locally), and then says exactly what to do. Marking a new
harness version as working or broken needs no herdr-threads build.

## Background and problem

Harness versions change almost daily; herdr-threads' actual dependency on them is narrow (a handful of hook
payload fields and the hook registration format). Version allowlists (recipes) therefore turn every routine
upgrade into "unsupported" or "live-unverified" noise until someone ships a build, which teaches users to ignore
the signal. B6 softens this with an optimistic admission ladder (an unlisted newer version is admitted as
Optimistic) and a daily canary that files issues, but it still keys trust on the version number and still needs a
build to record new evidence.

## Decisions (with the user, 2026-10-02)

1. **Verified by use.** The hooks themselves are the test: payload parses are recorded per (harness, version); a
   version that has delivered one valid lifecycle event and one valid tool event is verified on this machine.
2. **Update check only on failure.** A contract violation triggers a manifest lookup for a newer herdr-threads
   release that supports the version; only if none exists is the state degraded.
3. **Published manifest, no rebuild.** Version evidence (verified / known_broken per harness version) is a
   static file the canary updates; the daemon fetches it at runtime, with a copy embedded at build time as the
   fallback.
4. **Network policy (Q1 = A):** fetch only when needed (a version with no local evidence and no embedded row, or
   a fresh violation), at most once a day per harness, on by default with an opt-out; a plain GET of a static
   file that sends nothing about the user.
5. **Local vs. manifest (Q2 = A):** local evidence wins for "works"; the manifest decides only for versions never
   seen here; a locally observed violation always wins.
6. **Hosting (Q3 = A):** a data-only branch `harness-manifest`, pushed to directly by the canary, served raw
   (or via GitHub Pages from that branch).
7. **User-visible states (Q4 = A):** contract-first — **working**, **new**, **broken**; B6's ladder remains the
   internal classifier.

## User-visible states

| State | When | Health | `doctor` |
| --- | --- | --- | --- |
| working | verified by use here; or a manifest/embedded row `verified`; or a listed recipe version | nothing | one line: version, evidence source |
| new | admitted (ladder: Listed/SchemaMatched/Optimistic) but no evidence yet | nothing | one informational line: "new version, not yet seen working; verified on first use" |
| broken | contract violation observed here; or manifest or recipe `known_broken` for a version never verified here; or below the supported floor (older than every recipe minimum) | degraded, one line per source: violation → harness, version, event, field, action; `known_broken` → harness, version, the row's broken event/field, action; below floor → "<harness> <version> is below the supported floor <min>; upgrade <harness>" | same, plus the evidence trail and each verdict's source (local, canary, manual, recipe) |

`Supported` (native receipt) stays B6's separate, stronger claim; this epic does not change it.

**Which version is evaluated.** The state is computed per attributed harness version (the version the running
harness wrote into its transcript, see Attribution), not per PATH-detected binary. Health shows the worst state
among the versions with a session seen in the last 24 h (`last_seen_at`); a version with no session in that window
drops out of Health. Before any payload has been attributed, the daemon's admission observer's PATH version is
evaluated for `doctor` only (state "new" or the manifest's verdict), never for Health.

The broken line's action comes from the manifest, in order: "upgrade herdr-threads to X (supports
<harness> <version>)"; else "pin <harness> to ≤ <last working here or in the manifest>"; else "report:
<issue URL>" (the canary's issue when one exists).

A manifest `known_broken` row for a version already verified here is shown in `doctor` only ("the canary reports
a break in <event/field>; it has worked here"), never degraded.

## The payload contract

- Each harness module declares its contract as data: per hook event kind herdr-threads consumes (lifecycle:
  SessionStart; tool: PreToolUse/PostToolUse; plus any other event it parses), the required fields and their JSON
  types. The declaration is derived from what the existing parsers in `src/harness/claude.rs` and
  `src/harness/codex.rs` actually require; a test fails if a parser requires a field the declaration omits.
- `contract_id` = the first 16 hex chars of SHA-256 over a canonical serialization of the declaration (JSON with
  sorted keys, no whitespace, fields sorted within each event). `herdr-threads contract-id [--harness H] --json`
  prints it so the canary and manifest writer use the binary's own value; a test pins the current value, so any
  change to it is deliberate. It changes only when herdr-threads changes what it reads; that is the one case that
  needs a build.
- A **violation** is a well-formed JSON object, classified against the event the hook was **registered** for (the
  installed hook command carries `--event <name>`, which `setup` adds to each per-event registration; the payload's
  own discriminator is not trusted to pick the event. A registration without `--event` (installed before this
  epic) falls back to the discriminator, and `doctor` suggests re-running `setup`), with a required
  field missing or of the wrong type — including a missing or renamed discriminator. Malformed or truncated stdin, unknown event kinds and extra fields are not violations (counted by
  B6's parse-failure counter, never degrading).

## Evidence store and recording

- A daemon table `harness_version_evidence(harness, version, contract_id, lifecycle_ok_at, tool_ok_at,
  violation_at, violation_event, violation_field, last_seen_at)`, keyed by (harness, version, contract_id), added
  by one migration (number assigned at implementation; B4/B6 also add migrations).
- **Attribution** (redesigned after design roast r1): the version is the one the running harness recorded in its
  own session transcript, read from the payload's `transcript_path`. Claude: the `version` field of the newest
  JSONL entry that carries a `version` field, found by scanning backward from the end over complete lines only (a
  partial trailing line and versionless record types such as cost-state or mode are skipped); the read window
  starts at 64 KB and grows in 64 KB steps up to 1 MB, beyond which the event is unattributed. Codex: `cli_version`
  of the rollout's head `session_meta` record (its first line; nothing else is read). The spike found that a resumed
  rollout keeps its creator's single `session_meta` and appends turns with no version field, so a Codex session known
  to be resumed is never attributed: the hook's per-session gate records `resumed` on a `SessionStart` with source
  `resume`, and that SessionStart and every later event of the session are sent unattributed ("codex resume: rollout
  version is the creating CLI's"); the daemon does not hold that SessionStart. A hook with no state directory, or a
  resumed session idle past the 24 h gate-file pruning, falls back to head attribution (accepted). The hook reads it in-process (no exec, no process tree walk, no inode/mtime
  comparison) and never blocks on it. A Claude `SessionStart` with source `resume` is never attributed from the old
  file's entries; it is buffered like any unattributed SessionStart (below). If the transcript is absent (e.g. a SessionStart
  before the first entry is written), unreadable, or has no version field, nothing is recorded for that event and
  `doctor` says "version evidence unavailable: <reason>". **Unattributed SessionStart:** its outcome is sent with
  the session id and no version; the daemon holds it per session id and attributes it to the version of that
  session's first attributed event (held at most 24 h, then dropped), so the lifecycle half of verification and
  SessionStart violations are never lost to a not-yet-written transcript. A spike first confirms, for Claude and Codex (shared daemon and `--no-daemon`),
  when the transcript appears, how resume and fork behave, and that the recorded version is that of the process
  now writing.
- **Recording is cheap and bounded:** every hook event's outcome travels in `HarnessEvidence` (below); the lifecycle
  check-in itself records nothing in this table. SessionStart always sends its outcome. A tool event sends `ok` at
  most once per session, and only while the daemon's last reply said this version is not yet verified; the
  verified gate suppresses only `ok`: `violation` and `malformed` are always sent, at most once per (session,
  event, field). Each session also sends one `ok` heartbeat per hour (exempt from the gate), and the daemon touches
  `last_seen_at` on every `HarnessEvidence` it receives for that (harness, version), which keeps long sessions in
  the Health window. **Transport:** all evidence (payload-ok and violation) travels in one new
  capability-gated hook→daemon message `HarnessEvidence{harness, attributed version, contract_id, event,
  outcome: ok | violation{field} | malformed, session_id}`, sent for every admission tier and regardless of the ladder
  verdict (a refused version still sends evidence, which is what makes its below-floor or known_broken line reach
  Health) and not gated on Optimistic or on a
  Herdr pane (B6's parse-failure report is unchanged and still counts malformed payloads). The daemon keys rows by
  the hook-sent `contract_id`, so hook/daemon build skew records evidence against the contract the hook actually
  checked; a daemon that does not advertise the capability gets no message (the hook skips it silently).
- A violation sticks to (harness, version, contract_id). It clears when the harness version changes or
  herdr-threads' contract changes (a new `contract_id`), not when a later payload parses.

## Manifest

- **One producer chain.** B6's in-repo generator (from the recipe tables) is upgraded to emit schema_version 2,
  leaving the canary-only fields null; the canary reads the `harness-manifest` branch file as its baseline (falling
  back to the in-repo file) and writes the branch file; `scripts/canary/versions.py` accepts schema 1 and 2 (1 is
  upgraded on read). A daemon that meets an unsupported schema_version treats it as "no manifest" and falls back to
  the next source (cache → embedded).
- **Retention.** The canary writer keeps, per harness, the newest 50 versions plus every `known_broken` row and
  every recipe-listed version; it fails the workflow loudly above 80% of the size cap rather than publishing an
  oversized file.
- **Rows are per contract, but only the status is scoped.** A row is keyed by (harness, version, contract_id).
  The canary tests the current `main` contract and the latest release's contract and keeps one row per live
  contract. `contract_id` gates only the status a client takes from a row (`verified` / `known_broken` apply only
  under the client's own contract); release-pointer fields (`latest_release`, `supported_since`, `last_working`,
  `issue_url`) are read for the (harness, version) from any row, so "upgrade herdr-threads to X" is reachable from
  a row written under the newer contract X ships.
- **Format:** `harness-versions.json`, schema_version 2, extending B6's generated file: `generated_at`;
  `latest_release` (herdr-threads); `contracts: {harness: contract_id}`; `rows: [{harness, version, status
  (verified | known_broken), evidence (live | no_model | schema), contract_id, supported_since (oldest
  herdr-threads release that handles it), broken_event, broken_field, last_working, issue_url}]`. Unknown fields
  are ignored; size capped at 256 KB.
- **Embedded copy:** the release build embeds the `harness-manifest` branch's file (falling back to the
  in-repo generated file); local builds embed the in-repo file.
- **Fetch policy:** triggered only by (a) a harness version with no local evidence and no embedded or cached row,
  or (b) a newly observed violation. At most once per day per harness, 5 s timeout, conditional GET (ETag), result
  cached in the state dir with its fetch time. Any failure falls back silently to cache, then embedded copy.
  Opt-out: the JSON key `"harness_manifest": "off"` (default `"auto"`) in `<instance>/settings.json`, which the
  daemon loads at start and which rejects unknown keys (so an older binary refuses a settings file carrying the
  new key: downgrading herdr-threads means removing it), or `HERDR_THREADS_OFFLINE=1` in the daemon's environment.
  Both take effect at daemon start (`herdr-threads daemon stop` then `ensure`); `doctor` prints the effective
  policy and the cache's fetch time. Implemented
  as a `curl -fsS --max-time 5` subprocess (no TLS stack added to the binary; curl ships with macOS and common
  Linux distributions); a missing curl means "offline".
- **Trust:** advisory and unsigned under the cooperative model. A bad manifest can at worst block a version never
  seen here (with a stated reason) or add a `doctor` note; it can never override local evidence that a version
  works.

## Canary and publishing

- Extends B6's `harness-canary` workflow: after each run, write the results as manifest rows (verified with its
  evidence tier, or known_broken with the failing event/field and last working version) to the
  `harness-manifest` branch with a direct bot commit. The workflow's `contents: write` token is repository-wide;
  `main` and `v*` tags stay safe only through a repository ruleset restricting them, which this epic adds (or
  documents as a prerequisite) and the workflow checks before pushing. Issue filing for breaks is unchanged.
- **Only payload-contract failures write `known_broken`** (a captured hook payload that violates the declared
  contract); such rows must carry `broken_event` and `broken_field`. Every other canary outcome (Tier 0 setup /
  config-load / launch-flag checks, Codex schema-fingerprint drift, infra errors, inconclusive or flaky runs) stays
  issue-only and never reaches users as a manifest row.
- `latest_release` and `supported_since` come from the repository's releases and the recipe/contract tables; the
  canary obtains `contract_id` from `herdr-threads contract-id` built at the commit it tests.
- The release workflow reads the branch's file to embed.

## Deriving the state (one pure function)

Inputs (per harness version, under the newest `contract_id` the daemon has seen from that harness's hooks): the
attributed version's B6 ladder classification (split into below-floor and recipe known_broken),
local evidence row, manifest row (cached or embedded), contract_id. Order: local violation → broken; below floor
→ broken; local verified → working (manifest and recipe `known_broken` shown in doctor only); manifest
`known_broken` (same contract) or recipe `known_broken` → broken; manifest verified or listed recipe → working;
else admitted → new. Every combination is a row in one table-driven test.

## Coordination with B6

- B6's Optimistic Health note and doctor warning (`ht-p03.23`) become the "new" state's silent Health and single
  doctor line; this epic amends that rendering after it lands rather than editing B6's in-flight beads.
- B6's parse-failure counter stays for malformed payloads; violations travel in the new `HarnessEvidence`
  message (see Transport).
- B6's canary stays the producer; this epic adds the manifest writer.

## Non-goals

Telemetry or any upload; auto-updating herdr-threads or downgrading harnesses; harnesses beyond Claude and Codex;
signing the manifest.

## Testing

- Table test of the state function over every input combination.
- Contract declarations vs. parsers (drift test); violation vs. malformed-payload classification with the existing
  payload fixtures.
- Evidence recording: verified after one lifecycle + one tool payload; hooks stop sending notes once verified;
  attribution refusal when the version is ambiguous; violation sticks until version/contract change.
- Fetch policy with a fake fetcher: triggers only on (a)/(b), once per day, opt-out and offline honored, cache and
  embedded fallbacks, size cap.
- Canary manifest writer: offline selftest cases (all pass, payload break, known broken persists, flaky,
  non-payload Tier 0 failure → issue only, retention and size-cap failure, schema-1 baseline upgrade), and a test
  that the writer's `contract_id` equals the binary's.
- Attribution: a session whose transcript says A keeps attributing to A after PATH moves to B, with or without an
  in-place replacement of the binary; a missing transcript or version field records nothing and `doctor` names why.
- Mixed-contract manifest: another contract's status is ignored, its `supported_since` drives "upgrade to X".
- Event classification uses the registered event (`--event`): a payload with a renamed discriminator is a
  violation; a registration without `--event` falls back to the discriminator.
- Claude reader: transcript ending in versionless records and with a >64 KB line still attributes; resume
  SessionStart is buffered, not attributed from the old file. Unattributed SessionStart is attributed on the
  session's first attributed event. A violation after verification flips the state to broken. A refused
  (below-floor) version's evidence reaches Health. After a contract change the newest contract's row decides.
- Codex reader: only the head `session_meta` is read; a session resumed after an upgrade records nothing for either
  version (no verified, no violation).
- Evidence transport: sent for Listed, SchemaMatched and Optimistic versions alike; a daemon without the
  capability receives nothing and the hook is unaffected.
- End to end (stand-in harness): a new unlisted version → no Health line → first payloads → working; a payload
  missing a required field → degraded with the "upgrade to X" action from a fake manifest.

## Post-Implementation Notes

> *As this design is implemented and iterated on — bug fixes, adjustments, anything that diverged from the assumptions above — append a dated note here, whether or not a formal debugging skill was used.*

**Changes vs. original design (2026-10-02, super-auto run 2026-10-02-harness-version-evidence):**
- Codex resumed sessions are never attributed: the spike (docs/compatibility/harness-transcript-version.md) found a resumed rollout keeps the creator's single `session_meta`, so the hook marks the session resumed on SessionStart source=resume and records nothing for its later events (ht-xoc.18). Untested live: that Codex sends source=resume under the same session_id (the capture was refused by the permission layer).
- The payload parse-failure note moved from Health to `doctor`; Health renders `VersionRefused` as nothing (ht-xoc.5).
- One `settings.json` schema serves the service config and `harness_manifest` (ht-xoc.12).
- Open (report.md Remaining): the evidence step runs before `observe_harness_in` on the hook path and shares its budget; `doctor` would say "working" for a locally verified version inside a recipe `known_broken` range (both lists empty today).
