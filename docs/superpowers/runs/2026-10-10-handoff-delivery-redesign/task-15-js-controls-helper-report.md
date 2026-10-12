# Task15 JS Notice/epoch-controls helper

Status: DONE, awaiting fresh independent review. No D application, merge or Rust changes.

Workspace `/Users/alepar/AleCode/herdr-threads/.worktrees/handoff-delivery-redesign/.worktrees/js-epoch-helper-15`; branch `helper-ht-0lo.8.4-notice-controls`.
Baseline `dd5bfe864b9f95457a2922490f611a3f5c2361be`; final `9cb6f2c93b5d4f43f3e07a652f59e0b8999e3dcd`. Only the delta after this baseline returns to D. The prior epoch helper/copy commits are not part of this new delta.

## Implemented contract

D author explicitly confirmed exact producer schema before changes: WatchItem Notice has warning/thread/body and line ID `notice:<warning>`; Attention/Cleared add optional `presentation_epoch` exact decimal strings and IDs `attention:<epoch>` / `attention_cleared:<epoch>`.

Notice ingestion reuses the existing compact frame and delivered-ID persistence. Notice is always passive, nontruncated and nonwaking even if unexpected message flags appear; it never adds unacked, attentionVersions or invitationVersions. Normal replay is suppressed. Passive retry after append denial never submits an idle wake. No receipt/acceptance/token/lease or emit-confirm authority was added.

Present Attention/Cleared epochs must be string-valued canonical u64 decimal: `0` or nonzero-leading digits, <=20 ASCII digits and <=18446744073709551615. No Number conversion. Present null/undefined, leading zeros, signs, fractions, whitespace/newline, overflow and numeric values are ignored; they never invoke historical global-clear behavior. Absent field retains historical global unsent-clear and runAttention reset exactly.

Scoped clear removes only noninflight queued Attention with its exact epoch and deletes only that epoch's Attention run key. Other epochs, delivered history, in-flight evidence and unrelated message/invitation bookkeeping survive. Duplicate old clears are harmless. Existing stream status/run closure drops noninflight queue entries; successful already-inflight output can still be recorded, consistent with selected A7 asynchronous closure limit.

Per coordinator ruling, **existing active/passive scheduling remains**. Canonical ordered Watch projection is D's producer duty. JS Notice follows existing Lazy/passive-invitation immediate append behavior; active Attention/messages use existing tool-context/idle gates. No stronger physical order across these separate gates is promised. Tests prove passive Notice/Lazy/Invitation batch order and active Attention/message order while queued Notice retains its passive retry lane.

## Actual TDD and verification

All runs used actual `scripts/test-claude-mod`, selected this helper workspace's `integrations/claude/mod`, performed strict plugin validation and executed all three test files. Claude CLI 2.1.296 is historical evidence, not an operational version gate. No skip. Logs retained in /tmp.

| Phase | Actual outcome | Test runtime | Log | HT_LEAK_RUN_ID |
| --- | --- | --- | --- | --- |
| Baseline | 105 pass / 0 fail | 0.59s | `/tmp/ht-js-controls-base.log` | `5506e930-9961-4fd3-8f4f-64828956787a` |
| Initial tests-only RED | 108 pass / 5 fail of113 | 0.59s | `/tmp/ht-js-controls-red.log` | `4d82696f-925d-43d8-b4be-7df718fb43e2` |
| Expanded tests-only RED | 108 pass / 7 fail of115 | 0.59s | `/tmp/ht-js-controls-red2.log` | `98955026-bf24-43d0-9c1f-a7d4abc21982` |
| First implementation run | 113 pass / 2 fail of115 | 0.59s | `/tmp/ht-js-controls-green.log` | `7054b9fc-b03c-49b2-b734-01f75d171a49` |
| Final GREEN | 115 pass / 0 fail | 0.59s | `/tmp/ht-js-controls-green2.log` | `39198c45-8ae9-4962-8b6a-9b5e695a65c4` |

Initial five meaningful pre-production failures: Notice invisible; mixed passive Notice omitted; old clear erased newer queued Attention; old clear reset newer delivered dedupe; malformed present clear fell back to global deletion. Two additional observed pre-production failures were denied-Notice retry visibility and Notice presence in the initial busy-context fixture. The latter fixture also contained an overstrong expectation that passive Notice joins active tool context, corrected under the coordinator's scheduling ruling; it is not evidence of a product requirement to combine those lanes.

First implementation run's two failures were **fixture assumptions, not product RED**: (1) Notice had legitimately entered its own passive append, so it was in flight rather than queued when close arrived; (2) existing onToolCall excludes passive items. Fixtures were narrowly corrected: explicitly deny Notice append before asserting queued close removal, and assert passive retry independently while active context preserves insertion order. No scheduler source was changed to satisfy either assumption.

Final coverage: actual Notice visibility/dedupe/passive retry/noACK/noSubmit, mixed passive batches, active queue order with separate passive Notice retry, adjacent epochs above2^53 and u64MAX, matching-only queued removal, preserved newer run key after replay/lost-upstream-confirmation scenario, same/newer in-flight preservation, malformed-present refusal, omitted V1 global clear, observed close cleanup and stale old-run callback rejection. Existing invitation partial/full/inflight epoch tests, framing goldens and randomized stress remain GREEN.

Each run used fresh temporary HOME/CODEX_HOME/CLAUDE_CONFIG_DIR; script additionally owns/removes its internal config. Exact command shape:

```sh
task_tmp=$(mktemp -d /tmp/ht-js-controls-<phase>.XXXXXX)
mkdir -p "$task_tmp/home" "$task_tmp/codex" "$task_tmp/config"
export HT_LEAK_RUN_ID=<recorded UUID>
HOME="$task_tmp/home" CODEX_HOME="$task_tmp/codex" CLAUDE_CONFIG_DIR="$task_tmp/config" scripts/test-claude-mod > /tmp/ht-js-controls-<phase>.log 2>&1
# Capture status; remove only owned task_tmp.
scripts/check-no-leaked-processes --run-id "$HT_LEAK_RUN_ID"
```

Exact scoped leak checker ran after **every** tabled run under approved escalation (ps access): all returned `no leaked test processes`. All test commands finished, no helper/daemon remains owned, temporary config roots were removed. No real user config, Cargo, tracker, threads, network/model requests or foreign process changes. `git diff --check` passed before commit. Rust lint/default-feature/whole-branch checks remain coordinator-owned.

## Model-visible size and provenance

No static explanatory string or framing boilerplate was added. Notice now renders through the existing dynamic compact frame. Representative block `[herdr-threads] notice notice:warning-1 in T1 from unknown:\n  inspect warning-1` is **79 UTF-8 bytes**, plus existing shared page header/separators. Actual body is the existing canonical warning inspection marker; output grows only with required identity/thread/body/indentation. No epoch-control text is injected for a clear. All machine-only epochs remain <=20 ASCII bytes.

Baseline SHA256: register.js `604e28d528c01e798476faf3a013f5e360cb3c68e8d1dbbfcc53bd1f57584fcd`; delivery.test.ts `76d3a55cb32612d561eb0796a1046039b2b8bcd18ea0f256f390f0efa508afb4`.
Final SHA256: register.js `65898e9306e00a88fc8a5625bb70f517162e20c6fc8e707d87cc14ba38060348`; delivery.test.ts `d76fe46d9c36d9219b54794fcd877b63865bb1b60aa7eae7e750d92fd2361c89`.

Full baseline-to-final commit/stat/U10 diff: `task-15-js-controls-helper.diff`, SHA256 `6a5ba6f134e1bac083f4547331ab8498f4a8424056c3a94561e8151aa6f91a1a`. Only two allowed source/test paths changed; truthful Outside-adapter trailer lists both. Fresh review precedes quiescent D application. D's composed review must additionally prove canonical ordered Watch producer fields/bytes and token/flush/relevance semantics; this helper makes no claim that those Rust obligations are complete.
