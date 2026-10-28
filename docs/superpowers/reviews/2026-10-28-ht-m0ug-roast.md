super-roast verdict: Should-fix (1 confirmed) [low coverage]
mode: PR        iteration: 1 of 3
profile (assumed): Internal developer tooling. The diff touches only the opt-in Hermes path of the local wake e2e driver (scripts/wake-e2e.py) and its self-test; no shipped binary or wire surface changes. Blast radius is the operator's own machine, chiefly whether the driver writes outside its scratch Hermes home.
inputs: bead/ht-m0ug@9ae9fbff0 vs 6e918785b
coverage: unavailable — manual run, no coverage object · one fresh opus scout (correctness, isolation, self-test quality), checked against a local Hermes 0.22.5 source clone; findings verified inline by the author
independence: partial (subagent scout, inline verification) · rung: manual fan-out
seat-agreement: omitted (no full panels ran)

## Confirmed findings
- [Should-fix] scripts/wake-e2e.py `hermes_env` — an inherited HERMES_HOME redirects `--profile default` to another Hermes root, so `config set` (and the existing `plugins enable` and cleanup `gateway stop`) could write the operator's real Hermes config.
  verdict: confirmed (inline: hermes_cli/profiles.py `resolve_profile_env` uses `profile_root_for_env_home(os.environ.get("HERMES_HOME"), ...)`, which returns the exported value when set)
  evidence: hermes_cli/profiles.py:2430-2453 (Hermes 0.22.5); `hermes_env` dropped only CALLER_SESSION_ENV, CODEX_HOME, HERDR_PANE_ID.
  fix-shape hint: drop HERMES_HOME in `hermes_env`; the self-test asserts it is absent with HERMES_HOME exported. Fixed.

## Unverified nits (spot-checked)
- [Nit] `hermes_passthrough` — Hermes 0.22.5 returns `[]` (rc 0) for the unset key (config_defaults.py:327); the test only faked the "not set" reply. Fixed: added a `[]` case.
- [Nit] `hermes_passthrough` — non-JSON stdout escaped as a bare JSONDecodeError. Fixed: reported as "not a list" with the clipped output; test added.
- [Nit] cleanup comment still said "The recipient's pane". Fixed.

## Rejected (with reason)
- (FYI) Concurrent Hermes recipient starts: Hermes arbitrates racing gateway starts itself (gateway_runtime_start.py:39); cleanup's gateway-stop wait would report a leftover gateway. No change.

## Escalations (need human)
- [FYI] Whether `hermes config get --json` exists on pre-0.22.1 Hermes is unverified (no history in the clone); there setup now fails with a clear RuntimeError. Not acted on.
- [FYI] No self-test pins that `scratch()` calls `hermes_passthrough()`; deleting the call fails no test. Left as is (scratch() has no fake harness today).
- The change is unverified against a live Hermes.
