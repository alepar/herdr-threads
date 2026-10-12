R1: New ordinary ACK-required messages have an original deadline at canonical committed send time plus frozen configured/validated duration.
R2: No recipient binding, availability, registration, acceptance, restart or rebind delays or resets the original message deadline.
R3: Lazy messages and human receipt waivers have no ACK obligation; receipt preserves its existing meaning and provenance.
R4: Canonical/effective projection, preparation, sparse/physical materialization, due cursors, soft poke and wake calculations agree without fabricating availability.
R5: Upgrade preserves running deadlines and terminal states while transactionally converting formerly dormant obligations using immutable send time and duration without rewriting request bytes/digests.
R6: Unavailable recipients have bounded indexed due discovery, including already-expired upgraded obligations.
R7: Explicit A6 catch-up extensions remain separate and visible from original deadlines.
R8: Originating-seat message warnings survive leaving the thread, make no causal claim, and clear on late ACK without deadline changes.
R9: Ordinary modern invitations default to Lazy with pending discoverability and no timed confirmation/native wake.
R10: Invitation --require-ack and --deadline request timed confirmation; deadline implies require-ack and --lazy conflicts with either.
R11: Timed invitation deadlines start at canonical creation and settle automatically only on explicit acceptance; registration is insufficient.
R12: Invitation expiry durably warns its originating seat regardless of registration or membership.
R13: Rejection/cancellation/retirement and attributed rejection notices retain truthful outcomes, including Lazy invitations.
R14: Required service memberships and human/operator invitation semantics remain unchanged; human mode cannot manufacture agent ACK obligations.
R15: Historical pending invitations/intents retain legacy timed interpretation, original bytes/digests and inviter attribution.
R16: Pending-invite duplicate calls preserve policy/deadline/wake behavior; already-joined creates no obligation.
R17: Invitation queries, compact/machine/JSON inbox, digest, mod/native wake and due scans honor requested mode.
R18: New invitation representations negotiate capability/version before mutation and refuse unsupported semantics before writes.
R19: Handoff accepts exactly one positional/file/stdin body source, bounded while reading, nonempty valid UTF-8 <=65,536 bytes, preserving exact content and separate topic/goal limits.
R20: Handoff freezes actual body bytes and native options before durable effects; retries never reread sources/environment.
R21: New-tab, empty-pane launch and existing-recipient handoff share body/deadline contracts and exact keys.
R22: Handoff always stages addressed ACK-required instructions with configurable --deadline, independent Lazy/default or explicitly timed invitation policy.
R23: Handoff policy flows through all public/preparation/bootstrap/legacy/existing/retry representations while preserving omitted historical fields and completed keyed child mutations.
R24: Handoff stages while invitation pending, requires joined sender and never auto launches after possible-start.
R25: Native startup contains fixed inbox instructions, exact canonical routing and pinned fallback, no body/path/arbitrary task argv; all three route fields are compared.
R26: Hook/setup/trust/profile readiness stays diagnostic, so staged work and structurally safe selected-harness launch proceed while structural/caller/empty-shell/native-argv guards and truthful native consent refusals remain.
R27: Handoff ACK or missed-confirmation warning is independent of launch, registration, acceptance or task adoption.
R28: Stable hook-event HARNESS EVENT dispatches registered per-event codecs, retains legacy hook syntax and rejects invalid pairs before daemon effects.
R29: Equivalent executable/route/harness/event commands are byte-identical across reinstall without collapsing distinct installations/routes.
R30: Exact installed group fingerprints and installation identity establish ownership, preserving foreign identical/unrelated groups and failing conflicts safely.
R31: Equivalent reinstall reuses exact owned group in position; explicit legacy UUID/shared-event migration is interruption recoverable.
R32: Unsetup removes only exact owned records, drift fails safely and no native trust/permission records are automatically edited.
R33: Status/trust compares actual per-event stable commands; Claude/Codex events, Hermes fixed bridge and bundled launch/mod canonical route remain compatible.
R34: Registry manifests/canaries/help/embedded skills and TRUST-POLICY agree with new command, invitation and timer semantics.
R35: Focused isolated configuration exercises and format/clippy checks cover changed behavior before terminal integration; default-feature checks establish merge readiness and owned test processes are stopped.
R36: Supported65,536-byte body including worst JSON escaping survives public wire/canonical/local retry under ordinary bounded metadata, with exact encoded preflight and conservative documented bounds.
