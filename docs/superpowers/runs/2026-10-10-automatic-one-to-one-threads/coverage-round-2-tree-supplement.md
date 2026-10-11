ht-e99 (epic) Automatic one-to-one threads
parent: none
blocking deps: (none)
description:
Implement automatic target-seat-only send and threadless handoff. Spec: docs/superpowers/runs/2026-10-10-automatic-one-to-one-threads/2026-10-10-automatic-one-to-one-threads-design.md

ht-e99.1 (epic) Canonical pair resolver and immutable membership boundary
parent: ht-e99
blocking deps: (none)
description:
Spec: docs/superpowers/runs/2026-10-10-automatic-one-to-one-threads/2026-10-10-automatic-one-to-one-threads-design.md. Deliver capability-gated EnsureDirectThread, next additive pair migration, canonical sorted-seat indexed identity, atomic create/invite/reuse, A2/history replay and permanent third-seat/service ownership guards. owns: EnsureDirectThread protocol and canonical pair resolver/membership invariant; consumes: existing accountable transaction and ordinary invitation semantics. Files: src/protocol/{commands,results,authority,capabilities}.rs, src/store/{mod,control,messages,service_substrate,schema}.rs, migrations, TRUST-POLICY.md, focused store/protocol tests. Acceptance: reversed concurrency converges, replay does not recreate, invalid inputs have no effects, pair membership guard covers ordinary/service/operator entry points, lifecycle refusal guidance correct, raw sends cannot address third seats, ordinary threads unchanged, migration tests and focused checks pass. Keep only boundary/mechanism and its own acceptance tests; CLI and handoff adoption belong to siblings.

ht-e99.1.1 (task) Pair schema and compilable resolver protocol contract
parent: ht-e99.1
blocking deps: (none)
description:
Own the next additive pair migration, sorted unique pair/index/FKs, EnsureDirectThread command/result/permit/hash/capability, inert dispatch adapter and old-wire compatibility fixtures. owns: pair schema and EnsureDirectThread compilable interface; consumes: ordinary protocol/accountability conventions. Files: migrations, src/protocol/{commands,results,authority,capabilities}.rs, src/store/schema.rs and inert src/store/mod.rs adapter, focused schema/protocol tests. Acceptance: schema upgrades/current data intact, strict roundtrip and old fixtures pass, unsupported default does not advertise resolver. No usable pair creation until guarded mechanism lands. Spec: docs/superpowers/runs/2026-10-10-automatic-one-to-one-threads/2026-10-10-automatic-one-to-one-threads-design.md and parent nested spec.
Also owns additive default-omitted SendMessage.direct_recipient optional marker and absent-field byte/hash fixtures; this is not an ACK recipient.
Files-touched scope also includes all existing Rust SendMessage/SemanticMutation literal fixture sites needed for the additive default-None field; mechanical updates preserve ordinary semantics and serialization.
Verification acceptance: cargo fmt; nice cargo clippy --locked --all-targets --all-features -- -D warnings; relevant target-selected nice cargo test --locked --all-features. Honor isolated config/owned process cleanup; no routine full suite.

ht-e99.1.2 (task) Atomic pair resolver and permanent audience guards
parent: ht-e99.1
blocking deps: ht-e99.1.1
description:
Implement accountable EnsureDirectThread atomic creation/reuse/replay and create/invite projections, canonical recipient checks, pair-only ordinary/service/operator enrollment and raw-send preparation/publication checks; activate capability only with full mechanism. owns: canonical pair resolver implementation and immutable pair guard; consumes: pair schema and EnsureDirectThread interface. Files: src/store/{control,mod,messages,service_substrate}.rs and related service entry points, TRUST-POLICY.md, focused store tests. Acceptance: reverse concurrency one pair, invalid caller/target no effects, invited reverse caller guidance, archived/left/rejected explicit recovery, binding and namespace revalidation, no third participant/send audience, existing normal channels unchanged, operation replay historical, indexed bounded lookup. Schema contract is prerequisite (needs: ht-e99.1.1). Spec: docs/superpowers/runs/2026-10-10-automatic-one-to-one-threads/2026-10-10-automatic-one-to-one-threads-design.md and parent nested spec.
blocked-by ht-e99.1.1: consumes pair schema and EnsureDirectThread interface
Also owns direct_recipient canonical pair validation and common audience staging/deduplication in lazy/ordinary preparation/publication; keep ordinary lazy ACK-argument refusal unchanged. Tests exercise invited-target lazy delivery without receipt/wake and ordinary human waiver.
Acceptance configuration smoke: execute real-store Ensure then ordinary SendMessage/direct_recipient to resolved-unbound target and to Human target; assert invitation reservation, exact frozen audience, pair reuse, no synthetic binding/acceptance, and normal Human waiver. owns: guarded resolver/direct-recipient capability activation.
Verification acceptance: cargo fmt; nice cargo clippy --locked --all-targets --all-features -- -D warnings; relevant target-selected nice cargo test --locked --all-features. Honor isolated config/owned process cleanup; no routine full suite.

ht-e99.2 (task) Durable target-seat-only send CLI
parent: ht-e99
blocking deps: ht-e99.1.2
description:
Spec: docs/superpowers/runs/2026-10-10-automatic-one-to-one-threads/2026-10-10-automatic-one-to-one-threads-design.md. Deliver send --seat SEAT, preserved positional ordinary send, lazy/nudge/ACK/body matrix and frozen DirectSend compound journal with ensure/send keys, original caller/namespace, progress equality, pending-ops and exact retry/completed presentation. owns: DirectSend semantic/CLI coordinator and normal-send adapter; consumes: EnsureDirectThread protocol and canonical pair invariant. Files: src/cli/{commands,mod,journal,retry}.rs, src/cli/direct_send.rs if cohesive, tests/cli/{commands,journal,direct_send}.rs, tests/combined.rs. Acceptance: all body sources/modes, invited lazy target frozen audience, nudge target addressed, extra recipients/self invalid before effects, human send retained, lost ensure/send replies and restart do not duplicate, cleanup/historical replay preserve original actor, old journals unchanged; focused checks pass.
Promotion override: sp:demoted-by-session — parser/journal/coordinator/recovery are inseparable parts of one reviewable compound CLI behavior, not five independent merge-worthy deliverables; root spec fixes each decision, so further split would create ceremony below the cohesion floor.
blocked-by ht-e99.1.2: consumes EnsureDirectThread protocol and canonical pair invariant
Supply frozen direct_recipient marker to ordinary send; never place lazy target in ACK-only invited_recipients. Marker schema/guard is produced by ht-e99.1.2 (needs: ht-e99.1.2).
owns: local direct-send capability negotiation before compound journal publication and first effects. Acceptance: unsupported daemon direct send refuses before local compound publication or pair/send effects.
Verification acceptance: cargo fmt; nice cargo clippy --locked --all-targets --all-features -- -D warnings; relevant target-selected nice cargo test --locked --all-features. Honor isolated config/owned process cleanup; no routine full suite.

ht-e99.3 (epic) Automatic channel handoff composition
parent: ht-e99
blocking deps: (none)
description:
Spec: docs/superpowers/runs/2026-10-10-automatic-one-to-one-threads/2026-10-10-automatic-one-to-one-threads-design.md. Deliver explicit automatic channel variant/version for newly prepared handoffs, canonical Ensure child registration/attachment, exact namespace/fence thread matching, all destination paths including --new-tab downstream. owns: automatic handoff plan/fence composition and recovery; consumes: EnsureDirectThread protocol and canonical pair invariant. Files: src/cli/{handoff,handoff_delivery,topology_handoff,journal}.rs, src/protocol/handoff.rs, src/store/{handoff,topology_handoff}.rs and runtime modules, tests/cli/{handoff_delivery,topology_handoff}.rs, tests/handoff_topology_authority.rs. Acceptance: execute one fake-host request each for existing-seat, existing-pane, launched-pane and new-tab omission early; explicit --thread/--new-thread unchanged; V1 golden byte/hash/report compatibility; lost ensure/invite/send/start and child-complete/outer-incomplete recovery; no autoaccept/ACK/relaunch; canonical fences protected, focused checks pass.

ht-e99.3.1 (task) Automatic handoff plan and canonical attachment contract
parent: ht-e99.3
blocking deps: ht-e99.1.2
description:
Implement new-only automatic variant/version and strict compatibility validation, shared client helper signatures, registered Ensure child keys, exact canonical namespace/thread attachment and restricted phase validation. owns: automatic handoff serialized/shared adapter boundary and canonical Ensure attachment; consumes: EnsureDirectThread resolver protocol and canonical pair guard. Files: src/protocol/handoff.rs, shared src/cli/{handoff,journal}.rs types, src/store/{handoff,topology_handoff/runtime}.rs, focused golden/authority tests. Acceptance: V1 exact byte/hash/report fixtures unchanged; automatic Ensure cannot attach arbitrary thread, replay/live original caller/namespace correct; local producer paths stay unsupported until implemented; daemon deciding path is complete in this leaf. Consumed resolver available (needs: ht-e99.1.2). Spec: docs/superpowers/runs/2026-10-10-automatic-one-to-one-threads/2026-10-10-automatic-one-to-one-threads-design.md and parent nested spec.
blocked-by ht-e99.1.2: consumes EnsureDirectThread protocol and canonical pair invariant
owns: automatic-handoff daemon capability descriptor, complete canonical deciding adapters and activation; consumes: guarded resolver capability. Acceptance: advertise daemon auto-handoff only when exact Ensure attachment/restricted effects usable; CLI producer presence never gates descriptor; V1 unaffected. Adapter signatures are shared with producers but canonical deciding adapters are complete in this leaf, not stubs awaiting producers.
Verification acceptance: cargo fmt; nice cargo clippy --locked --all-targets --all-features -- -D warnings; relevant target-selected nice cargo test --locked --all-features. Honor isolated config/owned process cleanup; no routine full suite.

ht-e99.3.2 (task) Existing-seat and pane automatic handoff producer
parent: ht-e99.3
blocking deps: ht-e99.3.1
description:
Wire omitted channel in parser and existing-recipient/pane/native launch plans through shared automatic helper; freeze original target/caller/namespace/ensure keys and preserve override metadata rules. owns: parser omission and existing-recipient automatic producer; consumes: automatic handoff shared plan/attachment contract. Files: src/cli/commands.rs, src/cli/{handoff,handoff_delivery}.rs, tests/cli/{commands,handoff_delivery}.rs. Acceptance: one fake-host full request each existing-seat, existing-pane and launched-pane; explicit --thread/--new-thread unchanged; lost ensure/invite/send/start replies, retry/completed cleanup and recipient lifecycle preserve no duplicate effects/autoaccept/ACK. Shared contract available (needs: ht-e99.3.1). Spec: docs/superpowers/runs/2026-10-10-automatic-one-to-one-threads/2026-10-10-automatic-one-to-one-threads-design.md and parent nested spec.
blocked-by ht-e99.3.1: consumes automatic handoff shared plan and canonical attachment contract
owns: local automatic existing/pane producer capability negotiation before compound publication and first effects. Acceptance configuration smoke: unsupported daemon refuses each automatic existing/pane path before local auto journal publication or canonical/host effects; supported paths already have complete request exercises.
Verification acceptance: cargo fmt; nice cargo clippy --locked --all-targets --all-features -- -D warnings; relevant target-selected nice cargo test --locked --all-features. Honor isolated config/owned process cleanup; no routine full suite.

ht-e99.3.3 (task) New-tab automatic handoff producer
parent: ht-e99.3
blocking deps: ht-e99.3.1
description:
Wire threadless new-tab into shared automatic downstream plan after guarded recipient attachment, with unchanged topology/possible-start recovery and absorbing completion. owns: automatic topology downstream producer and reconciliation; consumes: automatic handoff shared plan/attachment contract. Files: src/cli/topology_handoff.rs, topology-specific store/runtime adapters if necessary, tests/cli/topology_handoff.rs, tests/handoff_topology_authority.rs. Acceptance: one fake-host full new-tab invocation; lost attachment/ensure/send/start response, child-complete/outer-incomplete restart recovery, exact selected pair and no repeated topology/start; explicit old new-thread modes remain. Shared contract available (needs: ht-e99.3.1). Spec: docs/superpowers/runs/2026-10-10-automatic-one-to-one-threads/2026-10-10-automatic-one-to-one-threads-design.md and parent nested spec.
blocked-by ht-e99.3.1: consumes automatic handoff shared plan and canonical attachment contract
owns: local automatic topology producer capability negotiation before compound publication and first effects. Acceptance configuration smoke: unsupported daemon refuses automatic new-tab before local auto journal publication or topology creation; supported path already has complete request exercise.
Verification acceptance: cargo fmt; nice cargo clippy --locked --all-targets --all-features -- -D warnings; relevant target-selected nice cargo test --locked --all-features. Honor isolated config/owned process cleanup; no routine full suite.

ht-e99.4 (task) Public guidance for automatic direct channels
parent: ht-e99
blocking deps: (none)
description:
Spec: docs/superpowers/runs/2026-10-10-automatic-one-to-one-threads/2026-10-10-automatic-one-to-one-threads-design.md. Deliver README/manual/embedded agent skill/help examples covering send --seat, all threadless handoff targets, explicit override forms, lazy default, invitation acceptance, two-seat reservation and archive/leave/reject recovery. owns: user-facing direct-channel documentation; consumes: syntax and semantics fixed in root spec (no code artifact dependency). Files: README.md, skills/herdr-threads/SKILL.md or actual embedded source, docs/manual.md or current manuals. Acceptance: repo search reconciles live send/handoff required-thread statements and examples; no claim launch accepts/ACKs or lazy wakes, no broad permission/config changes. Verify current guidance sources before editing; keep unrelated historical specs intact.

Verification acceptance: cargo fmt; nice cargo clippy --locked --all-targets --all-features -- -D warnings; relevant target-selected nice cargo test --locked --all-features. Honor isolated config/owned process cleanup; no routine full suite.
