# Composed C capacity timeout triage

Status: **exact failure stage confirmed; transport-hold hypothesis requires bounded runtime metadata.** This is not the earlier resumed `Page.more` failure. No assertion or timeout waiver is proposed.

## Source, scope and evidence

Read-only inspection of ROOT's `.worktrees/upstream-composition-478ec4a9`, clean HEAD `b046c123be452957d3f7f10d8fbdc60b18a4533e`, tree `ecc9977a535a932b5514ab8820650d1c832691a5`. Integration file SHA256 is `766a85c775227d26cf22b350a729f929baec6198a3c56284962aad820462bc01` (Git blob `11c7590200aba9af1123210f49627b949d1e2aa5`), unchanged from `f7aa1cc2`. Read the current TRUST policy and applied the already-read systematic-debugging workflow. This report is the sole authorized write. No source/Git mutations, Cargo, tests, Herdr, subprocess services or subagents.

`/private/tmp/ht-C-composed-b046c123/nextest-run.log:444–459` records capacity FAIL after27.485 seconds, panic at `tests/integration/mod_delivery.rs:1288`, `timed out waiting for item mAlNZmLGe`. Whole focused run:385 tests,377 PASS/8 FAIL,44.992 seconds. Execution metadata records focused, mandatory and final scoped leak exit0 under `28b15c51-3941-483f-83b0-2ef768028f03`. Parent reports the separate mandatory six passed. Other failures are outside this triage.

The retained panic starts with connected status and a complete ordinary ACK-required message, then truncates partway through the next6000-byte body. `Rig::start_daemon` deliberately saves only the first8192 bytes of each panic (`mod_delivery.rs:520–540`). Therefore that retained prefix does **not** show how many lines Watch actually emitted, its final status, its child exit state or stderr tail. Do not infer that it emitted only one message. A body dump is the wrong evidence surface here.

## Exact stage

Within this case the only `wait_item` is `watch.wait_item(&sent[255])`, immediately after `flow.release_reply(terminal)`. Thus `mAlNZmLGe` is message256, an ordinary RequireAck message seeded before Watch; it is not a new lazy message or the retained257th message.

Before this timeout the following passed:

1. One actual Watch owner connected.
2. Initial daemon Pulls reached a terminal response within64 observations.
3. The union of actual offered IDs equals exactly messages1–256, with no duplicates or ID257.
4. The paced barrier completed every canonical send_attention job, with complete==total>=257.
5. The stored normal carry contains ID257 with its exact6000-byte body.
6. The fixture released the terminal response permit.

The timeout occurs before receipt/no-ACK checks, before any explicit Attention forwarding, and before the resumed Pull. Neither an unexpected resumed `more` nor a missing queued Attention is established by this run.

Canonical selection and physical output are distinct: the offered set comes from real daemon replies observed **before** the relay forwards them. In particular, release_reply only updates a condition-variable permit; it does not prove that downstream frame writing succeeded.

## Strong hypothesis: fixture hold outlasts real Watch RPC budget

The terminal response remains held while the paced canonical-work barrier runs. That barrier allows the existing STEP20-second observation budget. The actual Watch issues each Pull with `cooperative_budget` (`src/cli/watch.rs:619–630`), which is a fixed5000-millisecond monotonic deadline (`src/cli/mod.rs:3050–3054`). `LocalSocketClient` spends the remaining deadline waiting for the response; expiry returns `UnknownOutcome` (`src/client/local.rs:183–205`). Watch treats an error as closing/error (`watch.rs:703`), without printing an unreceived page.

The relay calls `hold_reply` before `activation_write(downstream, reply_bytes)` (`mod_delivery.rs:1750–1769`). Its serve errors are discarded by `let _ = serve()`. A release after the client deadline can therefore leave the canonical terminal page observed but physically unreceived; if message256 belongs to that held page, the subsequent wait_item fails exactly where observed.

This is a complete possible call flow, but the original C log does not report terminal hold duration, target membership in the terminal page, child status or relay write result. Hence **not yet CONFIRM**. A client render/validation failure, different transport error, or output-reader issue must remain alternatives until the compact metadata distinguishes them.

Composition changes `LocalSocketClient` automatic-response limits/validation, but DeliveryPullV2 still selects the ordinary MAX_FRAME_BYTES limit; the inherited5000ms read timeout is unchanged. Watch and this fixture are byte-identical to f7. A passing D314 run does not establish that a longer held interval under C385 load remains within the real request deadline.

## Smallest diagnostic experiment

Parent owns execution. Add bounded existing-fixture observations, no additional Pulls, messages, sleeps, timeout changes or production logging:

- At terminal observation, record ordinal, result kind, more, offered IDs/item IDs (at most32), whether sent[255] is in that actual page, and a monotonic Instant.
- Record barrier start/end and terminal permit-release elapsed time. These are diagnostic values, not new timing-based success criteria.
- Record the relay's actual downstream write success/error for this terminal ordinal; `release_reply` itself is insufficient evidence. Keep at most65 compact per-response results if reusing the existing flow state.
- Around the existing wait_item failure, retain normalized Watch observations: kind/id/status/reason/exit, total seen count, target seen flag, bounded last statuses, bounded stderr and child try_wait state. Omit all message bodies.
- Include CapacityFlow request/ready/released counts, attention_seen/forwarded counts and worker failure state. These distinguish a dead client from a held/failing relay.
- Save the diagnostic directly in `rig.panics` before resuming the original failure, as the earlier retained-barrier diagnostic did; otherwise daemon stderr redirection and the8192-byte generic panic cap will hide it again.

One exact-case probe can confirm the hypothesis if the actual terminal contains the missing target, its release is beyond the real5s request budget, the child reports closing/error or exits before receiving it, and relay downstream write fails or cannot establish receipt. A passing exact-case run is only a non-reproduction; retain the timing/status metadata and compare the same focused selection if parent authorizes it. Do not rerun unchanged tests blindly.

## Conditional smallest correction if confirmed

Release the already-proved terminal false/Unchanged response **before** running the paced completion barrier, then observe message256 stdout. Keep Attention credits at zero throughout. A Watch that received terminal false parks on the channel stream, so canonical job completion can proceed without holding an outstanding5s Pull RPC. Keep the canonical barrier unchanged and take the normal baseline after it, then explicitly forward the real queued Attention and require the first resumed response to be quiet.

The existing `requests==terminal` / successful-frame-order gate must still prove that no extra Pull occurred during the barrier. This preserves real channel causality, the64 initial observation bound, all256 output/receipt/no-ACK checks, retained257th identity and exact normal carry tuple, and the strict resumed more=false assertion. It changes fixture ordering rather than transport timeout or product behavior. This correction is conditional on the actual diagnostic and is not yet granted execution clearance.

Static triage complete; awaiting parent-owned runtime evidence. No processes were started.
