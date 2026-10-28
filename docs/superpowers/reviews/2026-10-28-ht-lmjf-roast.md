super-roast verdict: Should-fix (2 confirmed, both fixed) [low coverage]
mode: PR        iteration: 1 of 3
profile (assumed): Local developer tooling installed per user; the installer writes into harness config roots (CODEX_HOME / CLAUDE_CONFIG_DIR), so a wrong write is user-data damage, but there is no network surface. Internal/production-grade bar for config writes.
inputs: bead/ht-lmjf@5f22106e2 vs main@50ffaf5fc (review fixes re-checked at 49f76af1d and the follow-up commit)
coverage: unavailable — manual run, no coverage object; one fresh scout (opus, general adversarial lane), findings verified inline by the author
independence: partial (subagent scout, inline verification) · rung: manual fan-out

## Confirmed findings
- [Should-fix] src/cli/installer_skill.rs `inspect_shared` — the sort only moved this link's own record last. A sibling linked profile's stale record (also made before its skills/ became a symlink) ranked level with the real root's record, in raw `read_dir` order, and could be picked first. The result was "does not own unedited" instead of shared, depending on directory order. Several aisw profiles is the normal setup.
  verdict: confirmed (inline: walked the code. `manifests_of_kind` returns unsorted `read_dir` order (src/harness/setup.rs:448-467). The old loop returned on the first matching record.)
  fix: 49f76af1d tries every record of the real file in a fixed order (real path first, other roots, own link path last, ties by path). The first one installed and unedited owns the file. The test now links two profiles with stale records. A mutation that checks only the last-ranked record fails the test.

- [Should-fix] src/cli/installer_skill.rs `inspect_shared` (found by the coordinator's independent review of 5aade628a, reproduced) — when the default root is itself a symlink (CODEX_HOME=~/.codex -> ~/dotfiles/codex), no record's lexical path equals the canonical one. A valid sibling profile's record then won by path order, and the verdict named that profile's link path ("managed at …/aisw/b/skills/herdr-threads/SKILL.md"). Reporting only; nothing was written.
  verdict: confirmed (new test `installer_names_the_real_skill_when_the_default_root_is_a_symlink` fails on 5aade628a with exactly that output)
  fix: every candidate record already resolves to the canonical file, so the verdict now names the canonical path whichever record validates. The ordering no longer affects any outcome and was removed.

## Rejected (with reason)
- none

## Unverified nits (spot-checked)
- [Nit] tests/installer_integrations.rs — with one linked profile, the ordering assertion depended on directory order. Fixed together with the finding above (two stale linked profiles).

## FYI
- An unmanaged link target that holds a byte-identical copy of the skill is reported shared ("managed at <canonical>") when the profile's own record still matches it. This mirrors the permissions component's last-resort rule (a3ea367c9 M3). Nothing is written. The record goes stale at the next skill change and the profile then fails.
- Pre-existing, not changed here: `shared_rules` in src/harness/codex/setup.rs ranks permission records the same way the skill code did before this fix (own path last, others in `read_dir` order). Permission records fingerprint the rule, not the file, so a stale-sibling failure is less likely there. Candidate follow-up.

## Escalations (need human)
- none
