# Independent mod capture and literal review — 2026-10-11

Verdict: **literal oracle and capture provenance clear; consumer runtime gate open** for mod commit `51068c5a69299574b405881ae61f3f287238f72b` against preparation base `7322b89abdb774b250adc81fe81a63ff1b450db2`. One confirmed test-boundary defect is recorded below. This report does not clear the three new plugin tests.

Scope: the two-file mod delta, the complete supplied capture, `resume-mod-report.md`, production `hooks/register.js`, committed producer test, and parent-generated native/plugin logs. Independent single-seat static review; no nested agents or judge panel. No Cargo, plugin execution, Herdr command, daemon, service, configuration write or source edit was performed by this reviewer. A read-only Python literal decoder checked byte equality; runtime results below come from parent-owned logs.

## Confirmed quality finding

- **Should-fix — `integrations/claude/mod/tests/delivery.test.ts:1409`: the test's `session.append` hook returns `{}` without forwarding `next(e)`.** Parent's actual plugin engine rejects/skips that boundary with `returned an answer without next (the row is kept; next(e) keeps it)`. `/private/tmp/ht-mod-test.log` records exactly 118 tests, 115 passing and the three new capture tests failing, each at this hook. Consequently commit `51068c5` has no successful consumer replay proof. The correction belongs in the test boundary; no evidence here calls for changing production framing or parsing. Rerun the parent-owned plugin suite and review the narrow successor diff before clearing this finding.

No material literal, byte-preservation or provenance defect was found in the reviewed snapshot. Validation is independently readable as passed in `/private/tmp/ht-mod-validate.log`; manifest validation does not substitute for successful tests.

## Exact snapshot and hash mapping

| Artifact | Reviewed identity |
| --- | --- |
| Mod commit | `51068c5a69299574b405881ae61f3f287238f72b` |
| Mod committed tree | `e63300f215f61d0a892b4f8724cab627244dfae6` |
| `integrations/claude/mod/tests/delivery.test.ts` SHA256 | `0b1e3c40482b5eafcf2059e436e696d1c89d827bc048f8d1b03e6af38409451b` |
| `integrations/claude/mod/tests/fixtures/watch-activation.ts` SHA256 | `7ecb675374e71029c31043ea82611a718015deb68282e0050274e575978df9ce` |
| Unchanged production `integrations/claude/mod/hooks/register.js` SHA256 | `65898e9306e00a88fc8a5625bb70f517162e20c6fc8e707d87cc14ba38060348` |
| Producer committed source | `ec04361ed0fc7efbfa7118d296621a25cf5a1175` |
| Producer committed tree | `a9f9cadfd328180904d1ec5998bfdfd8d4dfb41e` |
| Producer `ec04361e:tests/integration/mod_delivery.rs` SHA256 | `58bfd423e2d9339eca4a7bc9a66d91264f4b6f70250d7af91d543a661a5894fe` |
| Captured CLI executable SHA256 / size | `df396e835cc58b1cff80e67a56f523f1978e494e5d114cd44d6c1e5e1e4b8596` / `45039408` bytes |
| `/private/tmp/ht-notice-green-capture.jsonl` SHA256 / size | `596bc16bc63004d37ef41454f4e71dbb608fe3ea14d7c94a3fcb69eb27f8f53c` / `1571` bytes |
| Decoded fixture `raw` SHA256 / size | Same capture SHA256 / `1571` bytes, exact byte equality |
| `/private/tmp/ht-notice-green-native.log` SHA256 | `4dff8142828094af600620948f028231f9fb3bdafa9a8a066e2df019bd041c83` |

The executable path is `<ROOT>/.worktrees/super-auto-handoff-delivery-redesign--task-ht-0lo.3.2/target/debug/herdr-threads`, where ROOT is `/Users/alepar/AleCode/herdr-threads/.worktrees/handoff-delivery-redesign`. Its bytes/size were checked during this review. Source-to-build correspondence remains explicitly the parent's run5 attestation: executed `ad9d8f176897c4ae3f390566533f75d26914b300` plus the exact one-line execution correction, then committed as `ec04361e`. Hash equality alone is not a new independent build proof.

One current-working-file read raced later producer work and returned SHA256 `a7626b7fdf09c2590a1d4e34b4983e89f7f8d87b7777e63b2639fd7fec1ac4e8`. This report deliberately anchors producer provenance to the immutable `ec04361e` blob above. Parent reports a later replacement-fixture-only successor `082385c7`; it does not replace the run5 capture source identity.

## Seven-line byte preservation and epoch audit

Decoded `raw` equals the supplied artifact byte for byte, with seven newline-complete lines and a final LF. No parse/reserialization or numeric reconstruction was used for that comparison. Line lengths are `65, 288, 300, 214, 344, 228, 132` bytes, summing to `1571`.

| Line | Captured item | Byte SHA256 |
| --- | --- | --- |
| 1 | connected `status:0` | `31989c6e96d1f95734bed3470713814fe1a028dfc5e4c5ab14276dd04a475b56` |
| 2 | lazy `mw7dy8CUc` | `c223b84cd1d6883934dddcd3a76e21070e37375e3a1cb2050f8c455e21d69d25` |
| 3 | message `mbwolAHtW` | `506c46958a86eee7516191eafb2ccbf015f8a47b78fd1b3060be8600727d1011` |
| 4 | notice `notice:nMJDRPQ3Z` | `d9a04eedbabe8bcf89eae94a499e6abfd6024360f8ab1b16af510b338da10802` |
| 5 | invitation `invitation:ivRL92tt0` | `21936cd5147456580260e16566c05afbbf0544cd4d838a84f37418ca18768c4a` |
| 6 | attention `attention:1` | `ff57d9b231c923f63a23a8e2050f8e1a7f1ce9c1f15e5042709de8f8cd01580a` |
| 7 | clear `attention_cleared:1` | `0f51c39606326cea12c62eac805ed6b9b10f6dd19421e17f06aa04d6c27cd9c3` |

The only clear follows the only attention, and both `presentation_epoch` values are exactly the decimal string `"1"`. Invitation `command_context_epoch` and `topic_revision` are exactly `"0"`. The attention version `3006609949771651170` remains preserved in raw bytes; JavaScript `JSON.parse` may round that numeric field, but the clear assertion uses string epochs rather than that number. There are no successor pages or additional clear epochs hidden by the selected test subsequences. `capturedWatchLines()` retains every line and asserts the first pre-clear attention ID and matching epoch.

## Independent literal oracle audit

The expected arrays contain literal strings; they never call production `frame()` or render expected output through it. I compared the literals directly to captured fields and the framing contract:

- Passive headers are Lazy → notice → invitation. Active headers are ordinary message → attention. The idle subsequence retains status, notice and invitation bytes in their captured relative order; the clear test feeds all seven lines.
- Both peer messages name thread `tmFxjHevw "mod delivery"` and sender `seM75yOxM`. Since sender name equals sender ID, there is no duplicated quoted name. Notice uses thread `activation-warn`; invitation uses `tncVlNIeG "activation invitation"`. Notice/invitation have no captured sender, so `from unknown` is correct. Captured agent/relay/intent fields produce no markers.
- Body literals correctly require two-space indentation for lazy text, ordinary text, the exact notice `body` command, all three invitation lines including goal and `accept tncVlNIeG`, and the exact attention `inbox` command. Notice/invitation commands are displayed context, not executed by this fixture.
- Lazy is complete with `ack_required=false`; ordinary is complete with `ack_required=true`; invitation is complete with `wakes=false`. The test uses these captured distinctions rather than treating every displayed item as ACK authority.

## Real parser path and honest effect boundaries

The plugin hook manifest loads `./register.js`. Exported `register` installs the production session/turn/tool handlers; its `session.start` calls private production `buildIo`, whose streaming parser buffers stdout until LF and calls `JSON.parse` before core delivery. `capturedWatchEngine` supplies stdout through the engine's mocked `process.spawn`, deliberately splitting the first line after 17 characters. It does not pass pre-parsed fixture objects to `createCore` or replace `frame()`/the production parser. This is the correct intended consumer path, pending repair of the invalid append test hook and successful execution.

The process and engine boundaries are doubles: fake Watch process, session/state/store/env/fs/clock APIs, append/submit outcomes, tool result, and ACK process reply. The ACK double admits only `watch ack --session capture-session --via ...`; it cannot silently treat acceptance as an ACK success. Intended exact requests are Lazy `mw7dy8CUc` via `append`, then ordinary `mbwolAHtW` via `context`; the idle nonmessage test permits no request. Invitation, notice and attention IDs never enter these ACK sets. The `settled` per-ID responses are manufactured by the double, and ledger `acked` checks observe the mod's handling of those replies. They do not prove a real daemon receipt transition, persisted engine append, native Claude child, wake uptake or downstream model uptake.

The committed native producer separately asserts actual elected capabilities/lease, ordered bounded output, exact Confirm subset, canonical notice frontier for the execution, pending ordinary/lazy receipt states after presentation, and pending invitation before an explicit later actual ACK/acceptance. It writes the optional capture only after those positive assertions and the matching real scoped clear. Native log records that producer PASS in `3.599s`, alongside lost-response Watch and two notice regression passes. The run5 selection as a whole is **4 PASS / 1 FAIL** because replacement Watch timed out; fixture provenance and checkpoint report state that honestly. Its later repair is a separate gate.

The checkpoint report's original empty-capture and unrun-runtime statements are explicitly historical and superseded by its capture-installation section. Neither the report nor fixture falsely claims plugin runtime success or actual daemon/model success from mocked consumer outcomes. Source preparation and literal/provenance approval remain usable for the successor test-only fix; runtime clearance requires a new exact commit/log mapping.

## Scoped successor review — `bda5331`

Source verdict: **clear for the narrow callback correction; parent runtime rerun pending**. Independently inspected clean helper HEAD `bda5331cd0eab7aa255b91d82e36764d37b71673`, tree `7a6140b2d14506d379a7d57f36aa94acad51317a`, and its complete delta from `51068c5a69299574b405881ae61f3f287238f72b`. The sole changed path is `integrations/claude/mod/tests/delivery.test.ts`. The exact two-line change adds the third `next` argument to the `session.append` test hook and changes `return {} as Any` to `return next(e)`. This directly addresses the engine's recorded forwarding requirement while keeping the inspected message capture and all assertions intact. No source-level regression or additional finding is introduced by the successor.

Successor test-file SHA256 is `52313dd8c76c8a91693ce730812bfd745394bc544d78e01b9f5af52a94b28b09`. Fixture SHA256 remains `7ecb675374e71029c31043ea82611a718015deb68282e0050274e575978df9ce`; production register SHA256 remains `65898e9306e00a88fc8a5625bb70f517162e20c6fc8e707d87cc14ba38060348`. Complete fixture bytes, literal oracle, epoch correspondence, producer identities and ACK evidence boundaries therefore retain the approval above. `git diff --check 51068c5 bda5331` passes.

The confirmed callback defect is corrected in source. Runtime resolution is not yet asserted by this review: parent is rerunning the actual plugin suite, and its successor result/log identity must be attached before consumer execution is cleared. This reviewer executed no Cargo, plugin or Herdr commands.

## Actual successor execution clearance

Final scoped verdict: **clear — callback finding resolved in source and runtime**. Independently read the complete parent-owned `/private/tmp/ht-mod-test-successor.log`: all three capture tests explicitly PASS, and the final result is `118 pass`, `0 fail`, `Ran 118 tests across 3 files. [0.63s]`. Log SHA256 is `571e27286aef2eed7e8ae4fd6cfc5c16a4cefa825038e23621cc3855913a3a0b`. Parent maps this actual execution to mod successor `bda5331cd0eab7aa255b91d82e36764d37b71673` and an isolated temporary HOME ending `ht-mod-successor-d1ed5w7k` with separate `CLAUDE_CONFIG_DIR=claude`. Environment/command ownership is the parent's attestation; this reviewer only read the produced evidence.

The parent-owned strict validation log `/private/tmp/ht-mod-validate.log` also explicitly ends `Validation passed`; its SHA256 is `b6c446e37bf8c1de9328bafa0d5ed0a8ec60ef7e6d546926824f409f7dc7d977`. It enumerates the production register/buildIo APIs and hooks reviewed above. The single test callback correction changes neither manifest nor production code, so that validation applies unchanged.

Parent transferred exactly the reviewed two files in producer integration commit `7bdb3678f8b5ba10a5b51a7bdb6aed2fa163f7d3`, tree `899d0e5fb42e8f645879c9c5caa14265064c88ed`. Independent committed-blob checks confirm its delivery test SHA256 `52313dd8c76c8a91693ce730812bfd745394bc544d78e01b9f5af52a94b28b09` matches `bda5331`, and its fixture SHA256 `7ecb675374e71029c31043ea82611a718015deb68282e0050274e575978df9ce` matches the approved capture fixture. `git show --stat` lists only those two paths. This clearance follows those exact bytes, not the unrelated producer integration suite as a whole.

Successful actual plugin execution now proves replay through production register/buildIo/parser/core with the reviewed literal order/body assertions, passive no-wake behavior, exact requested ACK IDs/via, replay dedupe and matching scoped-clear behavior. External ACK `settled` responses remain process doubles. No actual daemon receipt transition, live native Claude child, wake/model uptake or broader native-suite success is inferred from these plugin results. Earlier failure/pending sections above are retained as historical evidence and superseded by this scoped successor clearance.
