super-roast verdict: clean (2 nits) [low coverage]
mode: PR        iteration: 1 of 3
profile (assumed): internal developer tooling (local multi-agent messaging plugin, same-user cooperative trust model). Model-visible bytes are a first-class cost; wire compatibility with older peers is enforced by scripts/check-wire-compat.
inputs: bead/ht-5gip@206f00fa2 vs main@6e918785b
coverage: unavailable — inline run, no coverage object · lanes reviewed inline: correctness, test-causality-lifecycle, test-isolation-environment, test-suite-cost, docs/model-visible text, wire-compat, harness-neutrality
independence: none (inline) · rung: inline

## Confirmed findings

None.

## Rejected (with reason)

- tests/integration/config_smoke.rs — "the e2e test passes vacuously". Rejected: mutating `DomainService::wake_status` to map `Some(false)` to `None` makes the test fail at the `--nudge` assertion (run during the change); it exercises the CLI send path, the daemon's per-(observer, seat) report set and both doctor renderings.
- tests/integration/config_smoke.rs — "order-dependent or racy". Rejected: one daemon, sequential CLI calls; the report set is in daemon memory and nothing restarts the daemon between calls; the synthetic hook binding is committed before `seat resolve` returns (same pattern as `synthetic_four_adapter_hook_configuration_and_canonical_replay`).
- integrations/skill/SKILL.md — "dropping the sentence leaves agents unable to act on the warning". Rejected: no shipped harness can produce it (`Registry::has_idle_wake` is `Some(true)` for claude, codex, agy and hermes; the registry test now enforces it over every non-test registration), and the line itself (`warning: SEAT wake=no_wake`) stays unchanged. A future shipped harness without a wake fails `idle_wake_classification_of_the_builtins` with a pointer to restore the sentence.
- Wire compatibility — `WakeBlock::NoWake`, `Command::WakeStatus` and `WAKE_STATUS_V1` are untouched; `scripts/check-wire-compat` passes.
- Harness neutrality — the only harness id added in core is in a `#[cfg(test)]` test in `src/harness/registry.rs` (registration file, already names every builtin); `scripts/check-harness-neutral` passes.
- Test isolation — the new test uses `TestIsolation::command` (scrubbed env, scratch HOME/XDG) and `SyntheticDaemon` (owned FakeHost, owner-watched daemon); no real config is touched.

## Unverified nits (spot-checked)

- [Nit] src/cli/wake_warning.rs module doc — names a test function and file; a rename would leave the pointer stale. Acceptable: matches the repo's existing doc-to-test cross references.
- [FYI, preExisting] src/cli/mod.rs — every p0 send, invitation and ACK request still makes one capability check plus one `WakeStatus` round trip that cannot produce output with today's builtins. Local RPC, no model-visible bytes; kept deliberately so the diagnostic needs no client change when a harness without a wake appears.

## Escalations (need human)

None.
