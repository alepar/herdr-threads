super-roast verdict: Should-fix (1 confirmed, fixed in 49f76af1d) [low coverage]
mode: PR        iteration: 1 of 3
profile (assumed): Local developer tooling installed per user; the installer writes into harness config roots (CODEX_HOME / CLAUDE_CONFIG_DIR), so a wrong write is user-data damage, but there is no network surface. Internal/production-grade bar for config writes.
inputs: bead/ht-lmjf@5f22106e2 vs main@50ffaf5fc (review fix re-checked at 49f76af1d)
coverage: unavailable — manual run, no coverage object; one fresh scout (opus, general adversarial lane), findings verified inline by the author
independence: partial (subagent scout, inline verification) · rung: manual fan-out

## Confirmed findings
- [Should-fix] src/cli/installer_skill.rs `inspect_shared` — the sort only moved this link's own record last. A sibling linked profile's stale record (also made before its skills/ became a symlink) ranked level with the real root's record, in raw `read_dir` order, and could be picked first. The result was "does not own unedited" instead of shared, depending on directory order. Several aisw profiles is the normal setup.
  verdict: confirmed (inline: walked the code. `manifests_of_kind` returns unsorted `read_dir` order (src/harness/setup.rs:448-467). The old loop returned on the first matching record.)
  fix: 49f76af1d tries every record of the real file in a fixed order (real path first, other roots, own link path last, ties by path). The first one installed and unedited owns the file. The test now links two profiles with stale records. A mutation that checks only the last-ranked record fails the test.

## Rejected (with reason)
- none

## Unverified nits (spot-checked)
- [Nit] tests/installer_integrations.rs — with one linked profile, the ordering assertion depended on directory order. Fixed together with the finding above (two stale linked profiles).

## FYI
- An unmanaged link target that holds a byte-identical copy of the skill is reported shared ("managed at <canonical>") when the profile's own record still matches it. This mirrors the permissions component's last-resort rule (a3ea367c9 M3). Nothing is written. The record goes stale at the next skill change and the profile then fails.
- Pre-existing, not changed here: `shared_rules` in src/harness/codex/setup.rs ranks permission records the same way the skill code did before this fix (own path last, others in `read_dir` order). Permission records fingerprint the rule, not the file, so a stale-sibling failure is less likely there. Candidate follow-up.

## Escalations (need human)
- none
