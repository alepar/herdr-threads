# Mod capture completion adoption

## 2026-10-11 — prepared source adopted; HOLDQUIESCENT

Status: NEEDS_CONTEXT. The genuine successful elected Watch stdout artifact remains an external dependency for source completion. No fixture bytes, positive evidence, validation result or live model uptake is invented. This report supplements `mod-activation-report.md`; the original author's report is unchanged.

Read the owned checkout's actual AGENTS.md and TRUST-POLICY.md, plus this workspace's constraints, Task 3/mod activation briefs, activation proposal and scope review. Parent retains finish ownership, Cargo/native execution, tracker and Herdr mutations. No subagents were spawned.

### Exact adopted source mapping

- Checkout: `/Users/alepar/AleCode/herdr-threads/.worktrees/handoff-delivery-redesign/.worktrees/finish-mod-activation`
- Branch: `helper/handoff-finish-mod-activation`
- HEAD: `7322b89abdb774b250adc81fe81a63ff1b450db2`; committed tree: `44b9dc6ab28b63583c49b9e578f1ec580386ee81`.
- Prepared changes remain uncommitted: modified `integrations/claude/mod/tests/delivery.test.ts` and untracked `integrations/claude/mod/tests/fixtures/watch-activation.ts`. Adoption made no source changes and discarded nothing.
- Working delivery test SHA256: `a2f21f3b86856bbf133254de297638e86a712d663b2e4311a45ab1cad012f3a3`.
- Empty guard fixture SHA256: `ca741906f1f7385deec22d8da469ef3528cf1e08f628d0a2c05394dd60d2dc80`.
- Production `integrations/claude/mod/hooks/register.js` SHA256: `65898e9306e00a88fc8a5625bb70f517162e20c6fc8e707d87cc14ba38060348`; tracked blob: `3361029f6ae937819eebba09cc1e2b860a5680ab`.
- Independently read D HEAD: `f8c25103a6ba1b2834e09b97cd2fca2924f0ce14`, tree `c78989b49c8c3801537e34b4e4b93bd7450bb228`, clean at inspection. D's production register hash equals the helper's. Its delivery test hash `d76fe46d9c36d9219b54794fcd877b63865bb1b60aa7eae7e750d92fd2361c89` equals the helper's committed base test. `git diff 7322b89a f8c25103 -- integrations/claude/mod` was empty. No mod source transplant is currently needed.
- D producer `tests/integration/mod_delivery.rs` inspected SHA256: `7b0a515b9edd0a5e650b430283921ef8b4515f32de5d8d614c3a293a65f29b19`. These identities are inspection snapshots, not a future capture's provenance; remap if D changes.

### Prepared contract and retained coverage

The three added test names are exactly those in `mod-activation-report.md`. The mixed test replays captured pre-clear bytes through production `buildIo` JSONLines parsing and production exported register handlers, splits process chunks without rewriting their concatenated bytes, checks three passive deliveries in captured order, preserves existing tool context, checks ordinary/attention context order, permits only lazy append completion and ordinary context ACK, checks no wake, then replays for deduplication. The idle test selects original status/invitation/notice lines without reserialization and checks two passive deliveries with no submit or process mutation. The clear test replays the entire capture and checks the matching queued attention alone is retracted while the ordinary message reaches context and receives its eligible ACK. These are prepared assertions, not executed results.

The capture reader throws `NEEDS_CONTEXT` while `raw` is empty. It requires newline-complete JSONLines, initial connected state and a scoped clear with string presentation epoch. Pending literal expected arrays are empty; no positive fixture or expected headers exist yet.

Source comparison confirmed all 106 original delivery test declarations and their order are retained; exactly three declarations were added. Other plugin test files are untouched. The parent's actual 115-test PASS remains prior baseline evidence; a source declaration count is not an executed test count. The parent's current advertised D result, 35 tests = 32 PASS / 3 Watch FAIL, is parent-reported and is not rerun or reclassified here.

### Minimal remaining source delta

1. Receive a raw stdout artifact from a **successful** actual `activation_v2_elected_watch_capabilities_lease_output_confirm` run, emitted by the elected advertised CLI chain. Failed-run leftovers or placeholder/generated JSON cannot satisfy this dependency. The inspected producer writes its optional `HT_ACTIVATION_CAPTURE_PATH` only after its positive assertions, but parent must still associate it with the completed successful command result.
2. Record artifact SHA256, byte length, source producer HEAD and tree (including any dirty-source manifest), producer test-file hash, actual CLI executable resolved identity/path and SHA256, build/source correspondence, exact command/filter, nonzero executed test count, result/log and capture path. The runtime selection witness must cover the same child's capability, accepted lease, Pull/Confirm and successfully emitted captured controls. Version facts, if present, are diagnostics rather than gates.
3. Install the complete UTF-8 stdout in the existing small fixture without changing any decoded byte, including ordering, escaping, CR/LF and final newline. Check the fixture's reconstructed raw bytes against the supplied artifact hash. Do not parse and reserialize to construct the stored stream.
4. Hand-review and populate literal lazy/ordinary/attention IDs and passive/active/nonmessage/message header arrays from the actual stream and framing contract. Expected headers must not call production `frame()` or derive their oracle from its result. Confirm exact passive order, eligible ACK set/provenance, invitation/notice no-ACK/no-accept/no-wake, and matching-epoch clear behavior. The existing assumptions of three initial passive items and one scoped clear remain provisional until those bytes arrive. If the successful capture requires a change, limit it to these three additions/fixture/provenance and report the observed reason; preserve the old tests.
5. Parent executes the real plugin validation/tests under isolated HOME and CLAUDE_CONFIG_DIR using the existing workflow or its same direct official commands: `claude plugin validate --strict integrations/claude/mod` and `claude plugin test integrations/claude/mod`. Record actual test count/result, source mapping and executable identity. No wrapper rewrite or native trust automation is needed. If these additions expose an engine-contract issue, correct only their assigned test scope and rerun through the parent.
6. Once genuine fixture and successful execution evidence are available, commit only assigned test/fixture/small provenance paths on the helper branch with a truthful `Outside-adapter:` trailer naming them. Parent owns integration and final gates; no commit is made for this empty checkpoint.

Required proof remains captured replay consumed through the real plugin engine with mocked external API outcomes. A live native Claude session spawning this child and downstream model uptake remain unproven optional external evidence.

### Performed checks and quiescence

Source-only `git diff --check` passed. Git status/hash/diff and test-declaration inspection are the only checks performed. No Cargo, fmt, lint, native plugin command, target directory, Herdr/tracker write, real configuration write, daemon/helper child or owned server was started. No commit was created. Helper is HOLDQUIESCENT until the parent supplies the genuine successful capture and provenance for follow-up completion.
