# Task 15 JS epoch helper report

Status: DONE; awaiting fresh independent review and D application. No merge/application performed.

Workspace: `/Users/alepar/AleCode/herdr-threads/.worktrees/handoff-delivery-redesign/.worktrees/js-epoch-helper-15`.
Branch: `helper-ht-0lo.8.4-js-epoch`.
Copied frozen D baseline: `0f5d9bd27584903adabfcc20787f503754b2b73a`. Final helper commit: `dd5bfe864b9f95457a2922490f611a3f5c2361be`.
Only baseline-to-final delta is relevant; the copied prior D baseline must not be cherry-picked.

## Change

Modern invitation context dedupe includes unchanged invitation ID plus exact decimal topic revision and command context epoch. A JSON tuple string preserves every decimal bit and unambiguously separates fields. Missing/null epoch retains the prior scalar revision representation, including persisted historical records, and differs from explicit epoch `0`. The same context key controls complete-display suppression, queued duplicate suppression and completion recording. Existing per-item completion preserves a newer context queued during an older in-flight append.

No ACK, acceptance, token, lease or emit/flush authority was added. No model-visible string was added: expected new recipient-facing text size **0 bytes**. Existing framing and lazy idle behavior are unchanged.

## Actual TDD evidence

The real `scripts/test-claude-mod` resolved this workspace's `integrations/claude/mod` and ran strict plugin validation plus all three mod test files using Claude CLI 2.1.296 (historical evidence only). No skips.

Baseline: **102 pass, 0 fail**, runtime 0.67 s; log `/tmp/ht-js-epoch-baseline.log`.
Tests-only RED: **102 pass, 3 fail**, runtime 0.71 s; log `/tmp/ht-js-epoch-red.log`.
Meaningful failures before production edits:

- Adjacent epoch changes at fixed topic revision: expected three appends, received one.
- Missing epoch followed by explicit zero: expected two appends, received one.
- Changed epoch during complete append: expected both items retained, received only old item.

Minimal production fix GREEN: **105 pass, 0 fail**, runtime 0.70 s; log `/tmp/ht-js-epoch-green.log`.
Coverage includes adjacent epochs `9007199254740992`/`9007199254740993`, `18446744073709551615`, exact topic revision changes, unchanged complete suppression, missing historical metadata, partial-to-full delivery with modern epoch, same-context partial append in-flight preservation, changed-context complete append in-flight preservation, no synthetic ACK and no lazy idle submit. Existing decimal topic-revision precision, framing, stress and historical delivery tests also pass.

Each run used this command shape with a fresh temporary root:

```sh
task_tmp=$(mktemp -d /tmp/ht-js-epoch-<phase>.XXXXXX)
mkdir -p "$task_tmp/home" "$task_tmp/codex" "$task_tmp/config"
export HT_LEAK_RUN_ID=<run UUID>
HOME="$task_tmp/home" CODEX_HOME="$task_tmp/codex" CLAUDE_CONFIG_DIR="$task_tmp/config" scripts/test-claude-mod > /tmp/ht-js-epoch-<phase>.log 2>&1
# Capture status, remove only this task_tmp; script removes its own internal config.
```

RED run UUID `fece4575-7acf-4287-844b-627d7b178812`; GREEN run UUID `e66076c7-5a20-4e98-954f-9bd7cd4507fa`. After each run the exact `scripts/check-no-leaked-processes --run-id <UUID>` passed under approved escalation: **no leaked test processes**. Baseline's first scoped checker was blocked by sandbox `ps` permission; its generated UUID was not retained. A later check using `8f6a0be8-6490-4d3e-a2df-55598558ed31` passed but is diagnostic only, not proof for the unrecorded baseline UUID. No persistent helper/daemon was started; all actual plugin tests finished and owned temporary roots were removed. No real user configs were touched.

`git diff --check` passed before commit. No Cargo, network/model requests, tracker or thread commands ran; Rust and whole-branch gates remain coordinator-owned.

## Provenance and review inputs

Frozen baseline SHA256:
- register.js: `d5e6b204e6e9858692258b0fa41b72e0bd1a2755e0860f3fe4319d907cd4c78b`
- delivery.test.ts: `44974faf681b30f76d5a059615c440c5a8667f69953f2b1c3c5fd25847e1e263`

Final SHA256:
- register.js: `604e28d528c01e798476faf3a013f5e360cb3c68e8d1dbbfcc53bd1f57584fcd`
- delivery.test.ts: `76d3a55cb32612d561eb0796a1046039b2b8bcd18ea0f256f390f0efa508afb4`

Full net baseline-to-HEAD diff: `task-15-js-helper.diff` (production and tests, U10 context). Commit has truthful `Outside-adapter` trailer listing both paths. D's composed review still needs to verify Rust emits exact canonical decimal epoch strings and final Rust/JS interoperability. This helper makes no claim about unimplemented Rust allocation or final merge readiness.
