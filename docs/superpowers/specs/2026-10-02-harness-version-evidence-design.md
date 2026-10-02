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
| broken | contract violation observed here; or manifest `known_broken` for a version never verified here; or ladder Refused (older than every recipe floor) | degraded, one line naming the harness, version, failing event and field, and the action | same, plus the evidence trail |

`Supported` (native receipt) stays B6's separate, stronger claim; this epic does not change it.

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
- A **violation** is a well-formed JSON object for a known event kind with a required field missing or of the
  wrong type. Malformed or truncated stdin, unknown event kinds and extra fields are not violations (counted by
  B6's parse-failure counter, never degrading).

## Evidence store and recording

- A daemon table `harness_version_evidence(harness, version, contract_id, lifecycle_ok_at, tool_ok_at,
  violation_at, violation_event, violation_field, last_seen_at)`, keyed by (harness, version, contract_id), added
  by one migration (number assigned at implementation; B4/B6 also add migrations).
- **Attribution:** the version is that of the **running harness process**, not the binary currently first on
  PATH (both harnesses update in the background or in place while old sessions keep running the old build).
  1. *Find the harness process:* walk the hook's ancestors (at most 6 levels) past shells (`sh -c` from
     shell-form hook registration), `env`, and runtime wrappers, to the first process whose executable is a
     recognized harness: a `claude`/`codex` native binary, or `node` whose argv runs the Claude/Codex CLI script
     (npm installs). The Codex shared app-server daemon counts as the Codex harness process. No match → record
     nothing.
  2. *Read its version from its own executable:* run `<exe> --version` once per (canonical path, inode, mtime)
     and cache it (for npm layouts, read the CLI package's `package.json`). This covers per-version layouts
     (Claude `versions/<v>`, Codex `releases/<v>-<target>`) and fixed-path installs alike.
  3. *Refuse a replaced image:* if the harness process started before the executable's current mtime (the file at
     that path was replaced after the process started, e.g. an in-place npm or package-manager upgrade), record
     nothing for that process.
  A spike confirms the real ancestry for Claude and Codex (shared daemon and `--no-daemon`) before the
  attribution code is written; if some supported configuration offers no recognizable harness ancestor, that
  configuration records nothing and `doctor` says "version evidence unavailable: <reason>".
- **Recording is cheap and bounded:** a successful lifecycle check-in already reaches the daemon and records
  `lifecycle_ok_at`. For tool events, the hook sends a best-effort, non-blocking "payload ok" note at most once
  per session, and only while the daemon's last reply said this version is not yet verified; once verified the
  hooks send nothing extra. Violations are reported over the same channel B6 uses for parse failures.
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
- **Rows are per contract.** A row is keyed by (harness, version, contract_id). The canary tests the current
  `main` contract and the latest release's contract and keeps one row per live contract; a daemon uses only rows
  whose `contract_id` equals its own and treats the rest as no data.
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
  Opt-out: `harness_manifest = "off"` (default `"auto"`) in the instance settings file the daemon already reads,
  or `HERDR_THREADS_OFFLINE=1` in the daemon's environment; both are read at each fetch decision (no restart
  needed), and `doctor` prints the effective policy and the cache's fetch time. Implemented
  as a `curl -fsS --max-time 5` subprocess (no TLS stack added to the binary; curl ships with macOS and common
  Linux distributions); a missing curl means "offline".
- **Trust:** advisory and unsigned under the cooperative model. A bad manifest can at worst block a version never
  seen here (with a stated reason) or add a `doctor` note; it can never override local evidence that a version
  works.

## Canary and publishing

- Extends B6's `harness-canary` workflow: after each run, write the results as manifest rows (verified with its
  evidence tier, or known_broken with the failing event/field and last working version) to the
  `harness-manifest` branch with a direct bot commit (`contents: write` scoped to that branch; `main` stays
  protected). Issue filing for breaks is unchanged.
- **Only payload-contract failures write `known_broken`** (a captured hook payload that violates the declared
  contract); such rows must carry `broken_event` and `broken_field`. Every other canary outcome (Tier 0 setup /
  config-load / launch-flag checks, Codex schema-fingerprint drift, infra errors, inconclusive or flaky runs) stays
  issue-only and never reaches users as a manifest row.
- `latest_release` and `supported_since` come from the repository's releases and the recipe/contract tables; the
  canary obtains `contract_id` from `herdr-threads contract-id` built at the commit it tests.
- The release workflow reads the branch's file to embed.

## Deriving the state (one pure function)

Inputs: B6 ladder classification, local evidence row, manifest row (cached or embedded), contract_id. Order:
local violation → broken; ladder Refused → broken; local verified → working (manifest known_broken shown in
doctor only); manifest known_broken (same contract) → broken; manifest verified or listed recipe → working; else
admitted → new. Every combination is a row in one table-driven test.

## Coordination with B6

- B6's Optimistic Health note and doctor warning (`ht-p03.23`) become the "new" state's silent Health and single
  doctor line; this epic amends that rendering after it lands rather than editing B6's in-flight beads.
- B6's parse-failure counter stays; violations are a strict subset of what it counts.
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
- Attribution: a session started on version A keeps attributing to A after the PATH binary is upgraded to B
  (per-version layout); a binary replaced in place at the same path after the session started records nothing;
  the ancestor walk passes an `sh -c` hop and a `node` wrapper; an unrecognized ancestry records nothing and
  `doctor` names why.
- Manifest rows of another `contract_id` are ignored by a client.
- End to end (stand-in harness): a new unlisted version → no Health line → first payloads → working; a payload
  missing a required field → degraded with the "upgrade to X" action from a fake manifest.

## Post-Implementation Notes

> *As this design is implemented and iterated on — bug fixes, adjustments, anything that diverged from the assumptions above — append a dated note here, whether or not a formal debugging skill was used.*
