C1 · r1 · flag-sweep · ht-8lem.10 · kept — PROMOTE overruled: Q6 decided provisionally with its refusal string; daemon cap from .1, exercised in the sweep
C2 · r1 · GAP · R7 post-link overdue · applied — ht-8lem.3 owns COALESCE(deadline_node) in obligation_authored_here + offline-after-link test
C3 · r1 · GAP · R6 · applied — ht-8lem.3 owns line names SenderBlock::verify against node_key on every node
C4 · r1 · GAP · R9 · applied — ht-8lem.3 owns the 2.x fail/replay behaviour and help text; sweep adds the older-node case
C5 · r1 · GAP · duplicate request intake · applied — ht-8lem.6 owns duplicate-id intake; sweep re-send test
C6 · r1 · UNOWNED-SEAM · RemoteLaunch reply shape · applied — ht-8lem.1: answer unchanged, sender derives thread/message
C7 · r1 · UNOWNED-SEAM · startup input template · applied — ht-8lem.6 names existing prepare_startup_input API
C8 · r1 · UNOWNED-SEAM · profile projection · applied — ht-8lem.6 freezes one LaunchDirectoryContext consumed by .8/.9
C9 · r1 · GAP · R10 regression · applied — ht-8lem.1 required_cap test; sweep already lists bare launch
C10 · r1 · GAP · R4 stage-time refusal · applied — ht-8lem.7 failed re-check aborts with no fact
C11 · r1 · UNOWNED-SEAM · pair thread identity · applied — ht-8lem.7 deterministic direct_thread_id; sweep no-duplicate assertion
C12 · r1 · UNOWNED-SEAM · Refused/Unavailable rendering · applied — ht-8lem.10 renders, ht-8lem.4 leaves Unavailable Sent
C13 · r1 · GAP · R13 sizes for prompt v2 · applied — ht-8lem.5 acceptance states sizes
C14 · r1 · NEEDS-SPEC · ht-8lem.3, ht-8lem.14 · rejected — answered from the spec (§4.2 verify, §4.3 COALESCE) by amending ht-8lem.3's owns line; no re-run
C15 · r2 · GAP · R2 seat in the transaction · applied — ht-8lem.7 states the seat precedes staging (spec §3.2 order); facts go in one transaction
C16 · r2 · GAP · sender side of pair if sender dies · rejected — spec §3.2: the sender's home node authors the sender's join when it applies thread.direct (N4); no live sender needed
C17 · r2 · GAP · third node lacks sender key · rejected — spec §4.2: message needs the author's seat.created from the sender's log, so the key (genesis) is present before apply
C18 · r2 · GAP · duplicate overdue across linking · applied — ht-8lem.14 owns clear-on-ACK and dedup; acceptance test
C19 · r2 · UNOWNED-SEAM · Failed after staging vs ledger delete · applied — ht-8lem.1 Failed.staged field; .2 keeps row; .8/.9 set it; .10 renders (31 bytes+); sweep case
C20 · r2 · GAP · R13 help text size · applied — ht-8lem.3 acceptance
C21 · r2 · UNOWNED-SEAM · HANDOFF_SENDER_PROMPT_V2 advertise/check · applied — ht-8lem.5
C22 · r2 · UNOWNED-SEAM · ack_after_ms · applied — ht-8lem.1 names it, signed and restated
C23 · r2 · UNOWNED-SEAM · STAGE_V1 check in child · applied — ht-8lem.8
C24 · r2 · GAP · non-deterministic existing pair · applied — ht-8lem.7 resolves the pair canonically and reuses it
C25 · r2 · GAP · thread.direct authority · rejected — spec Background: thread.direct is EitherSeatHome; the target is the recipient's home
C26 · r2 · UNOWNED-SEAM · frozen context across the child boundary · applied — ht-8lem.8 derives from the stored plan through the same LaunchPolicy call
C27 · r2 · GAP · 3.0.0 bump, release notes, announcement · rejected — out of scope: coordinator owns the release; CHANGELOG lines are proposed at hand-back
C28 · r2 · GAP · R13 wire tolerance owner · applied — ht-8lem.1 owns line; sweep runs the checks
C29 · r2 · GAP · crossed thread.direct idempotence · applied — ht-8lem.7 cites idempotence and keeps a crossed test
C30 · r2 · GAP · sweep deps on .13 · rejected — no provenance value is coded without ht-8lem.13's approval (ht-8lem.7 note)
C31 · amend-design-1 · ORPHAN · ht-8lem.2 · rejected — already closed as superseded by ht-8lem.17
C32 · amend-design-1 · UNOWNED-SEAM · ledger state model in .14/.16 · applied — owns lines restated to overdue columns
C33 · amend-design-1 · UNOWNED-SEAM · migration 0015 DDL owner · applied — ht-8lem.1 owns the DDL, .17 the module
C34 · amend-design-1 · UNOWNED-SEAM · Failed.staged writer · applied — .18 sole builder; .6/.8/.9 depend on .18
C35 · amend-design-1 · UNOWNED-SEAM · obligation_authored_here · applied — .14 depends on .3
C36 · amend-design-1 · GAP · seat known, thread not held · rejected — spec §2.3: the requester opens the pair after Launched + wait_replicated, so seat known implies thread held
C37 · amend-design-1 · GAP · expiry vs alert · applied — .16: expiry is an exit, unshown alert does not survive; peer list line remains
C38 · amend-design-1 · UNOWNED-SEAM · link before settle · rejected — spec §2.3 transition table and ht-8lem.17 acceptance (guarded, linked terminal)
C39 · amend-design-1 · GAP · re-send replays frozen bytes · applied — ht-8lem.4 owns line
C40 · amend-design-1 · GAP · expiry accepted limit in TRUST-POLICY · applied — gate ht-8lem.19 to the coordinator; .13 carries it if approved
C41 · amend-design-1 · GAP · Refused/NoCap residue · applied — .17 deletes on Refused/NoCap/Failed without staged
C42 · amend-design-1 · GAP · mod channel rendering of the alert · rejected — spec Q4 'Accepted consequences': generic attention block on hooks/channels, one inbox call, as for every actionable warning
C43 · amend-design-1 · GAP · sweep depends on .13 · applied — C30 reconsidered now .13 is unblocked by approval
C44 · amend-design-1 · UNOWNED-SEAM · TTL vs retention constant · applied — .17 compile-time assertion naming RETENTION_MS
C45 · amend-design-1 · NEEDS-SPEC · .17/.18 · rejected — answered from the amended spec §2.3 by restating owns lines
G1 · graph · GRAPH-EDGE · ht-8lem.16 <- ht-8lem.14 · applied — repoint to ht-8lem.1: the alert reads columns from migration 0015; files disjoint from .14 (attention/inbox vs schema/deadlines)
G2 · graph · GRAPH-EDGE · ht-8lem.17 <- ht-8lem.1 · kept — consumes migration 0015 DDL
G3 · graph · GRAPH-EDGE · ht-8lem.14 <- ht-8lem.3 · kept — both edit obligation_authored_here in src/store/schema.rs
G4 · graph · GRAPH-EDGE · ht-8lem.3 <- ht-8lem.17 · kept — consumes handoff_obligations link transition
