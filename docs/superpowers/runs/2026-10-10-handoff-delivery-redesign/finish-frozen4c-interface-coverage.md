# Frozen D interface/consumer coverage

Spec NOT CLEAN
Quality NOT CLEAN

Completed read-only inspection of D `4c4988d9ee937c333d602bac25dca25fc2c07d37` against `ee1e598c3ee96566ddc9b397185cdc97c8bf6218`, tree `1589136ce7568bcd72b0050fa8d947e6f49503ab`. Exact source-line findings, evidence and limits are in `interface-audit.md`. The 74-path final inventory and full cumulative diff were used; all 20 production entrypoint-map hashes matched current D. No Cargo, Git, Herdr, tracker, source mutations or subagents were used. Only the two interface audit Markdown files were written.

Depth labels: **consumer** means focused current-source/diff tracing of actual interface or consumer behavior; **seam** means changed seam/dependency or mechanical compatibility inspected, without claiming exhaustive file audit; **boundary** means interface-facing portions inspected while exhaustive canonical writers, migrations, queries/private loaders and schema proof remain assigned to the independent whole/schema reviewers. Tests listed as inspected are source evidence unless the audit names an actual existing log. No tests were run by this reviewer.

| Changed path | Depth and coverage |
| --- | --- |
| TRUST-POLICY.md | seam: emitted-control authority and accepted physical-emission limits |
| integrations/claude/mod/hooks/register.js | consumer: decimal epochs, scoped clear, passive Notice, invitation context cache, downstream inflight limit |
| integrations/claude/mod/tests/delivery.test.ts | consumer: epoch/clear, mixed delivery, invitation revision/goal and inflight cases; historical helper evidence distinguished |
| migrations/0040_invitation_presentation.sql | boundary: durable outbox/profile/control masks; exhaustive migration review delegated |
| migrations/0041_presentation_relevance.sql | boundary: relevance continuation model; exhaustive migration review delegated |
| src/cli/hook.rs | consumer: original connected completion scope, matching native claim, exact write/flush gate |
| src/cli/launch.rs | consumer: optional selected hook fingerprint, prelaunch and executable recheck |
| src/cli/watch.rs | consumer: capability/lease admission, ordered capture, frame bounds, write/flush, exact Confirm, reply validation, close/continuation |
| src/daemon/control.rs | seam: registered owner lease plumbing |
| src/daemon/transport/watch_connection.rs | consumer: actual accepted-outcome omission while inactive; advertised helper arm distinguished |
| src/delivery/assemble.rs | seam: invitation classification/content extraction |
| src/delivery/invitation.rs | consumer: exact canonical topic/goal/revision/command rendering and partial-context limits |
| src/delivery/ledger.rs | consumer: 256-offer bound, retention/removal and settled-offer starvation trace |
| src/delivery/mod.rs | seam: presentation module export |
| src/delivery/page.rs | seam: invitation class/page accounting |
| src/delivery/presentation.rs | consumer: registered codecs, actual capped native serialization, ordered WatchLine+LF and candidate/RPC budgets |
| src/harness/adapter.rs | consumer: finite static descriptors and inert completion defaults |
| src/harness/agy/decode.rs | consumer: native System/User envelopes and capped encoding |
| src/harness/agy/delivery.rs | consumer: selector snapshots, reply projection, frozen encoding, typed captured completion/subset and hint boundaries |
| src/harness/agy/launch.rs | consumer: advisory readiness with unchanged native admission safety |
| src/harness/agy/mod.rs | seam: codec/completion adapter wiring |
| src/harness/agy/paths.rs | consumer: stable installed native hook-event argv |
| src/harness/registry.rs | consumer: codec/completion registry forwarding |
| src/ports.rs | seam: neutral completion port |
| src/protocol/capabilities.rs | consumer: V2 defined and still absent from ADVERTISED |
| src/protocol/commands.rs | consumer: V2 claims, validation and distinct explicit Confirm |
| src/protocol/delivery.rs | consumer: required render, deny unknown fields, matching consumer, canonical token/lease/epoch and bounded unique stamps |
| src/protocol/output_compact.rs | seam: optional invitation goal/revision compact output |
| src/protocol/results.rs | consumer: typed confirmation reply versus stale scalar fixture |
| src/protocol/watch.rs | consumer: optional lease/control epochs, omission and additive decoder compatibility |
| src/scheduler/mod.rs | seam: passive publication observation plumbing |
| src/service/delivery_presentation.rs | consumer: canonical admission, owner permit lifetime, frozen render, ACK subset health arming, settled ledger defect |
| src/service/delivery_pull.rs | consumer: shared Hook/Watch admission, legacy ledger cleanup and V2 coexistence |
| src/service/dispatch.rs | consumer: V2 routing and actual receipt/manual ACK ledger behavior |
| src/service/kicks.rs | seam: publication drain signals rather than health arming |
| src/service/mod.rs | seam: service module wiring |
| src/service/mod_channels.rs | consumer: exact live lease, permit replacement/close, grace refusal, atomic cutoff and old receipt stall |
| src/store/attention.rs | boundary: invitation-bearing delivery seam; exhaustive producer accounting delegated |
| src/store/control.rs | boundary: canonical context/relevance seam; exhaustive writer accounting delegated |
| src/store/invitation_content.rs | consumer/boundary: exact transaction scope, fixed replay slots, frozen profile, partial/idempotent Confirm, control applicability, page geometry and ordinary starvation; exhaustive SQL/private-ID budgets delegated |
| src/store/mod.rs | seam: invitation/presentation module wiring |
| src/store/presentation_relevance.rs | boundary: publication/relevance before replay and Unknown suppression; exhaustive traversal/query proof delegated |
| src/store/queries.rs | boundary: canonical content query seam; exhaustive physical/query proof delegated |
| src/store/schema.rs | boundary: schema registration; exhaustive migration proof delegated |
| src/store/seats.rs | boundary: binding/retirement seam; exhaustive canonical writer inventory delegated |
| src/store/service_controls.rs | boundary: service-control publication seam; exhaustive writer inventory delegated |
| src/test_support/mod.rs | seam: scoped fixture wiring |
| tests/cli/hook.rs | seam: mechanical optional step-completion additions |
| tests/cli/hook_neutral.rs | consumer: actual writer failure/flush/success completion seam; not external native-process proof |
| tests/cli/launch.rs | consumer: advisory readiness success and native safety regression |
| tests/cli/lazy_display.rs | seam: mechanical optional goal/revision additions |
| tests/cli/output.rs | seam: compact tuple/optional output fixture updates |
| tests/cli/watch.rs | consumer: success, replacement, failed write/flush, foreign/lost/refused reply and continuation close; Notice-only empty-control success gap recorded |
| tests/combined.rs | seam: explicit test module registration |
| tests/daemon/watch_connection.rs | consumer: actual inactive transport omission plus helper-only active branch |
| tests/delivery/extraction.rs | consumer: invitation content+Attention classification, goal and no-cache behavior |
| tests/fixtures/agy/plugin/hooks.json | seam: installed hook-event argv fixture |
| tests/harness/agy_decode.rs | consumer: native codec shapes and actual encoded limit |
| tests/harness/agy_delivery.rs | consumer: actual canonical native producer/codec/confirm bounds and complementary scripted reply failures/subsets |
| tests/harness/agy_launch.rs | consumer: missing/stale/disabled/unreadable advisory readiness and unsafe native argv |
| tests/harness/agy_setup.rs | consumer: actual installed native event routes |
| tests/hook_entrypoint/agy.rs | seam: native hook entrypoint output/callback fixture |
| tests/integration/agy_sweep.rs | seam: stable event route integration fixture; not rerun |
| tests/protocol/delivery.rs | consumer: V2 shapes, old piggyback rejection, framed maximum Confirm; misplaced token-negative coverage gap recorded |
| tests/protocol/lazy_inbox_v2.rs | consumer: exact old omission and optional goal/revision roundtrip |
| tests/protocol/watch.rs | consumer: old/new nested decoder and lease/epoch omission |
| tests/service/delivery_pull.rs | consumer: actual canonical subset/restart/idempotence, stale raw replay fixture, encoded Watch/native bounds, health lifecycle and admitted-scope fixtures |
| tests/service/kicks_contract.rs | seam: passive publication contract fixture |
| tests/service/mod_channels.rs | consumer: real owner admission/rotation/grace, permit replacement exclusion and exact cutoff tuple |
| tests/store/attention.rs | boundary: canonical producer changes; exhaustive writer proof delegated |
| tests/store/control.rs | boundary: canonical context changes; exhaustive writer proof delegated |
| tests/store/invitation_content.rs | boundary: fixed replay/mask/partial/query fixtures; exhaustive SQL/private-loader proof delegated |
| tests/store/queries.rs | boundary: query interface fixtures; exhaustive physical/query proof delegated |
| tests/store/schema.rs | boundary: schema fixture updates; exhaustive migration proof delegated |

The report records two open findings: I2 stale raw V2 replay fixture (observed missing-render failure in parent's fresh frozen88 sweep; scalar response expectations also stale), and I3 settled ordinary ledger capacity starvation (independently source-confirmed shared production defect; helper owns regression/fix). Preparation I1 lease omission is source-fixed and actual inactive transport test passes in the parent's fresh sweep.

Actual existing evidence inspected includes the final affected24 log/hash, relevant stall RED/GREEN/owner/publication logs/hashes, and parent's frozen88 log in full (87PASS/1FAIL, raw replay missing render; no saturation regression). Historical warning-containing Watch helper logs were not promoted to final source evidence. Parent reports scoped leaks0; this reviewer ran no leak check.

Explicit unclosed coverage: advertised transport/native caller activation; nonempty Attention/Cleared successful ordered Watch flush through exact captured Confirm; explicit Confirm token-negative fixtures; saturation regression for settled ordinary offers; separately owned bounded private-ID loaders/query physical proof; final repaired cumulative SHA gates and scoped leaks. No capability approval or final integration-readiness claim is made. Inspection is complete and this reviewer is quiescent on frozen D; successor repairs require a new exact-source recheck.
