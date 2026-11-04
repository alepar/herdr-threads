# ht-8lem task tree (bead dump for the design roast)

## ht-8lem.1 — Seam contract: handoff wire, SenderBlock and schema [open]
deps: none

Spec §1, §1.1, §2.2 (SyncCommand claim), §4.1, §4.3, Key abstractions (SenderBlock). Lands only the unblocking artifacts, inert by default.
owns: every new wire field serde(default), // cap: PEER_HANDOFF_CAP on gated plan fields, wire-compat baseline additions; SenderBlock (fields per spec §1.1 including ack_after_ms, which is signed and restated by the predicate as duration_ms and the recipient deadline; new src/sync/handoff_block.rs: struct, HTHOFF01 signing bytes, sign, verify, BlockError, the single 'payload restates block' predicate); PEER_HANDOFF_CAP const; LaunchPlan fields profile_dir/handoff_prompt/message with LaunchPlan::handoff() and required_cap(); MessageSentPayload.handoff field; SyncCommand::RemoteLaunch claim field (LaunchState::Failed gains one serde(default) field 'staged' {thread, message}, set when the target failed after staging so the sender keeps its obligation; otherwise the RemoteLaunch answer and LaunchState are unchanged: the sender derives thread = direct_thread_id(sender, seat) and message = its minted id from the ledger; NoCap keeps its peer version) and local caps HANDOFF_ATOMIC_REMOTE_V1, PEER_HANDOFF_STAGE_V1, HANDOFF_SENDER_PROMPT_V2 consts (not advertised until their feature lands); migration 0015 (the whole DDL, per spec §2.3 as amended and 'Seat-addressed overdue (Q4)': handoff_obligations with request_bytes, state CHECK sent/launched/linked/expired, overdue_seq, thread_warned_at, alert_shown_at, expires_at and the partial index handoff_obligations_alert; peer_requests handoff_thread/handoff_message/handoff_prompt, messages.deadline_node).
Files: src/sync/handoff_block.rs (new), src/sync/mod.rs, src/sync/frame.rs, src/sync/facts/message.rs, src/protocol/sync_admin.rs, src/protocol/capabilities.rs, migrations/node/0015_*.sql, src/store/schema.rs (migration registration), scripts/wire-compat-baseline.txt.

Acceptance:
- [ ] SenderBlock signing bytes match a golden vector; sign/verify round trip; every tampered field fails verify; an HTJRNL01 entry signature by the same key does not verify as a block
- [ ] verify enforces v==1, priority==0, require_ack, ack_after_ms>=1 when set
- [ ] the restates predicate accepts an exact payload and rejects each differing field, two recipients, wrong thread
- [ ] LaunchPlan/MessageSentPayload/SyncCommand new fields are serde(default), gated plan fields carry // cap: PEER_HANDOFF_CAP; old JSON decodes unchanged
- [ ] migration 0015 applies on a fresh and an existing store; schema tests pass
- [ ] scripts/check-wire-compat and scripts/check-harness-neutral pass; clippy clean
- [ ] LaunchPlan::required_cap() returns the existing launch/terminal caps for a plan with no handoff fields (bare launch unchanged)

## ht-8lem.2 — HandoffObligations: sender request ledger [closed]
deps: ht-8lem.1

Spec §2.3 (ledger, linking, overdue before linking), §4.3. Key abstraction HandoffObligations.
owns: store::handoff_obligations (insert-or-ignore with minted message id, read back, closed state enum Sent/Launched/Linked/Overdue, delete on NoCap/Refused/Failed without 'staged' (a Failed with 'staged' keeps the row with its seat and thread so it links when the message replicates), block-field rebuild, link(message, seat), due query over unlinked rows, 7-day prune). Ledger API only; pre-link overdue is ht-8lem.14.
consumes: Seam contract (migration 0015 table, SenderBlock fields).
Files: src/store/handoff_obligations.rs (new), src/store/mod.rs, src/store/schema.rs (ObligationRef, obligation_authored_here), (no scheduler or CLI changes).
blocked-by ht-8lem.1: consumes SenderBlock, LaunchPlan/MessageSentPayload/SyncCommand fields and migration 0015 (boundary contract)

Acceptance:
- [ ] state transitions unit-tested, invalid transitions refused; repeat insert returns the same row and message id
- [ ] due query returns unlinked rows past deadline_at
- [ ] rows pruned after 7 days

## ht-8lem.3 — Handoff message authority: HandoffRecipientHome and materialize [open]
deps: ht-8lem.1, ht-8lem.17

Spec §4.2, §4.3 (deadline_node), §5. Extends AuthorRule and materialize_message.
owns: AuthorRule::HandoffRecipientHome (on every node: SenderBlock::verify against node_key(tx, block.sender_node), author's replicated home == sender_node, origin == recipient's home, plus the restates predicate; any failure -> FactError::Rejected, the existing failed/replayable path); obligation_authored_here reading COALESCE(deadline_node, origin_node), so after linking the ordinary receipt overdue fires only on the sender node and never on the target; the 2.x third-node limit (an older node applies SeatHome(author), origin is not the author's home -> rejected -> failed; replay after upgrade) with its 'sync unapplied' help text; store::materialize::node_key(tx,node); MessageSent::apply selecting the rule when payload.handoff is Some and calling the SenderBlock predicate; materialize_message setting deadline_node=block.sender_node and handling a local author with non-local origin (no sender binding, no preparation); calling handoff_obligations::link in the same transaction when block.sender_node is this node.
consumes: SenderBlock predicate and MessageSentPayload.handoff (Seam contract); HandoffObligations::link.
Also owns obligation_authored_here reading COALESCE(deadline_node, origin_node), and the 'sync unapplied' help text naming the 2.x third-node best-effort limit (spec §4.2).
Files: src/sync/facts/message.rs, src/store/schema.rs (obligation_authored_here), src/cli (sync unapplied help), src/store/materialize.rs, src/store/messages.rs, tests/sync/facts_message.rs, tests/store/materialize.rs.
blocked-by ht-8lem.1: consumes SenderBlock, LaunchPlan/MessageSentPayload/SyncCommand fields and migration 0015 (boundary contract)
blocked-by ht-8lem.17: consumes handoff_obligations::link
Amended by redesign cycle 1 (amend-design-1.md; spec §2.3 and 'Seat-addressed overdue (Q4)' win over older text here): obligation states are sent/launched/linked/expired with guarded monotonic transitions; overdue is the columns overdue_seq/thread_warned_at/alert_shown_at, never a state; re-sends replay the row's frozen request bytes; unlinked rows expire after OBLIGATION_TTL_MS (< 7 days).

Acceptance:
- [ ] a handoff message.sent from the recipient's home origin applies; each mismatch (signer not author home, payload differs from block, two recipients, wrong thread, bad signature, origin not recipient home) is Rejected -> failed, listed in sync unapplied
- [ ] older-node simulation: a registry with the pre-3.0 message.sent handler marks it failed; sync unapplied replay under the shipped registry applies it
- [ ] deadline_node set from the block on every node; obligation_authored_here true only on the sender node
- [ ] on the sender node the message materializes with local author, non-local origin, no sender binding; the obligation row links in the same transaction
- [ ] non-handoff message.sent behaviour unchanged (existing tests untouched)
- [ ] linked handoff message with --deadline: receipt overdue fires on the sender node only (target never), including with the target offline after linking
- [ ] the 'sync unapplied' help addition states its byte size in the commit

## ht-8lem.4 — PeerHandoffRequester: sender daemon sign, send, settle, re-send [open]
deps: ht-8lem.17, ht-8lem.1

Spec §2.2 (daemon side), §2.3, §7 (NoCap upgrade). Key abstraction PeerHandoffRequester.
owns (spec §2.3 as amended wins): sign once, insert-or-ignore the obligation with the signed request_bytes, and every send/re-send replays the row's request_bytes verbatim (never re-signs); run the expiry transition before each re-send pass and at start; service::peer_handoff (claim check vs requester_seat, local nonretired seat with open top-level binding -> CallerUnverified; record obligation; fill sender_node/message/requested_at/author_role and sign with NodeIdentity; send through launch_remote; settle answers into the ledger (Unavailable leaves the row Sent for re-send); after Launched + wait_replicated open the pair via resolve_direct_pair; daemon task re-sending unlinked rows on link-up and at most every 30 s); SyncAdmin::remote_launch delegating to it for plans with a draft; coordinator on_launch_remote using plan.required_cap() and answering NoCap{PEER_HANDOFF_CAP}; advertising HANDOFF_ATOMIC_REMOTE_V1 (the target-side link cap check is ht-8lem.6's).
consumes: Seam contract (SenderBlock, LaunchPlan::required_cap, SyncCommand claim); HandoffObligations.
Files: src/service/peer_handoff.rs (new), src/service/mod.rs, src/service/sync_admin.rs, src/sync/coordinator/remote_launch.rs, src/sync/coordinator.rs, src/daemon (task wiring).
blocked-by ht-8lem.1: consumes SenderBlock, LaunchPlan/MessageSentPayload/SyncCommand fields and migration 0015 (boundary contract)
blocked-by ht-8lem.17: consumes HandoffObligations ledger API
Amended by redesign cycle 1 (amend-design-1.md; spec §2.3 and 'Seat-addressed overdue (Q4)' win over older text here): obligation states are sent/launched/linked/expired with guarded monotonic transitions; overdue is the columns overdue_seq/thread_warned_at/alert_shown_at, never a state; re-sends replay the row's frozen request bytes; unlinked rows expire after OBLIGATION_TTL_MS (< 7 days).

Acceptance:
- [ ] a request with a draft and a non-matching or non-top-level claim is refused CallerUnverified, no ledger row
- [ ] repeat polls send byte-identical blocks (rebuilt from the row)
- [ ] NoCap/Refused/plain Failed delete the row; Launched sets seat+launched and opens the pair thread locally
- [ ] with the CLI gone, the daemon re-sends on link-up and every <=30 s until linked; a re-send never causes a second launch
- [ ] a peer without peer.handoff.v1 yields NoCap{peer.handoff.v1}, nothing sent
- [ ] plain launch/terminal plans still use their existing caps (required_cap)

## ht-8lem.5 — Prompt renderer v2: summary plus from line, one renderer [open]
deps: none

Spec §6 as amended by 'Coordinator answers' (Q1, Q6, Q7). Extends summary_prompt.
owns: advertising HANDOFF_SENDER_PROMPT_V2 and the local CLI's check of it (an older daemon keeps the v1 render); summary_prompt::render_v2 and SenderLine (name from replicated seats.name, id); one line on every host: '<summary> — from: <name> (<id>)' or '<summary> — from: <id>' (one renderer constant ' — from: '; no host-dependent separator, no newline); local handoff plans freezing sender_prompt behind HANDOFF_SENDER_PROMPT_V2, used by launch_prompt/pane_prompt/the V3 thread-free call site; v1/V3/bootstrap renderers and HANDOFF_SUMMARY_PROMPT_V1 byte-frozen for retained records; fallback first part 'Handoff' for a local v2 prompt with no summary.
Files: src/cli/handoff.rs, src/cli/topology_handoff.rs, src/protocol/handoff.rs, src/protocol/capabilities.rs (advertise).

Acceptance:
- [ ] golden bytes for render_v2 with and without a name; the prompt passes native argv validation (no line break)
- [ ] new local handoffs freeze and use v2; retained v1 records relaunch byte-identically (retained_prompts tests unchanged)
- [ ] commit states sizes: ' — from: ' 11 bytes; overhead 23 bytes for a 12-byte id without a name, 26 + name (<=64) with one; max 290 bytes; fallback 'Handoff' 7 bytes

## ht-8lem.6 — Target validation: handoff_refusal and frozen prompt in decide [open]
deps: ht-8lem.5, ht-8lem.18, ht-8lem.1

Spec §3.1, §1 (malformed checks). Extends PeerLaunchService::decide.
owns: duplicate-id intake (an existing peer_requests row for the request id answers its recorded outcome; never re-decides, re-validates or re-launches; Unavailable is unrecorded so a retry decides afresh); handoff_refusal step in decide after the existing checks, in order: link cap; block sender_node==from, author==requester_seat, request==id, signature under sync_logs.pubkey(from) -> Refused 55-byte detail; sender seat held as remote with home from (not replicated -> Unavailable 35-byte, unrecorded; other home -> Refused 44-byte); profile_dir absolute+is_dir+LaunchPolicy::explicit_directory_env (60-byte detail or adapter error); harness startup input via the existing adapter API (prepare_startup_input / HandoffLauncher::select_startup_input); the profile directory is frozen once as a LaunchDirectoryContext through LaunchPolicy::explicit_directory_env and that one context is what ht-8lem.8 and ht-8lem.9 consume. Inbound malformed bounds (body <=65536, handoff_prompt passes summary_prompt::validate, profile_dir <= MAX_LAUNCH_TEXT_BYTES). Render the prompt once with render_v2 and freeze it in peer_requests.handoff_prompt at row insert. Every refusal writes no fact. Also owns advertising PEER_HANDOFF_CAP by default, withholdable via HT_TEST_WITHHOLD_CAPS (src/sync/coordinator.rs:824-835,941-945); the branch merges as one unit, so advertising before .7-.9 land never ships.
consumes: Seam contract (SenderBlock::verify, LaunchPlan::handoff, migration columns); render_v2 (Prompt renderer v2).
Files: src/service/peer_launcher.rs, src/sync/coordinator/remote_launch.rs (malformed), src/store/peer_requests.rs.
blocked-by ht-8lem.1: consumes SenderBlock, LaunchPlan/MessageSentPayload/SyncCommand fields and migration 0015 (boundary contract)
blocked-by ht-8lem.5: consumes render_v2
Duplicate-id answers go through ht-8lem.18's recorded-answer builder (staged derivation).
blocked-by ht-8lem.18: consumes the recorded Failed answer builder

Acceptance:
- [ ] each refusal in order, each leaving peer_requests refused and no journal entry
- [ ] sender seat not replicated -> Unavailable, not recorded, a retry succeeds once it replicates
- [ ] valid handoff plan inserts the row with handoff_prompt frozen
- [ ] byte sizes of the new details stated in commit
- [ ] a re-sent request with the same id (while running, after launched, after refused) answers the recorded outcome and starts nothing new

## ht-8lem.7 — PeerHandoffStage: target one-transaction staging [open]
deps: ht-8lem.1

Spec §3.2 step 2, §3.3, §4.1. Key abstraction PeerHandoffStage.
owns: store::peer_handoff::stage (the seat already exists from the worker's guarded resolution or allocate_gate_seat; a seat with no staged facts is harmless and is not a fact about the sender; re-check row running and seat still the target's — a failed re-check aborts the transaction with no fact and the child records plain failed, which the sender ledger deletes; SenderBlock predicate; resolve the pair for (author, seat) through the canonical direct-pair lookup (an existing pair is reused, whatever its id) and author thread.direct with direct_thread_id(author, seat) only when no pair exists; thread.direct apply is already idempotent on the id across origins (EitherSeatHome, crossed-create skip, spec §2.3) — keep a test for the crossed case (the sender's own resolve_direct_pair yields the same id, so crossed creates never duplicate), the recipient's membership.joined, message.sent with handoff block and deadline anchored on requested_at; set peer_requests.handoff_thread/handoff_message; optional hc record_gate_launch in the same commit; idempotent on request id); local Command::PeerHandoffStage{request,seat} -> PeerHandoffStaged{thread,message}, served only for a running peer_requests row with a block, advertised as PEER_HANDOFF_STAGE_V1; peer_requests::gate_view send_staged = created && (no handoff_message || message row exists).
consumes: Seam contract (SenderBlock, MessageSentPayload.handoff, migration columns).
The daemon.log line is an audit line, not a provenance value (a provenance value needs ht-8lem.13's approval). Also owns one daemon.log line at staging naming request, thread, message and sender node (target-side audit, spec §8 item 1). Its deadline_node correctness on the authoring node is checked end to end in the integration sweep (materialize side is ht-8lem.3).
Files: src/store/peer_handoff.rs (new), src/store/mod.rs, src/store/peer_requests.rs, src/protocol (Command/Reply), src/service/dispatch.rs.
blocked-by ht-8lem.1: consumes SenderBlock, LaunchPlan/MessageSentPayload/SyncCommand fields and migration 0015 (boundary contract)

Acceptance:
- [ ] one transaction writes exactly thread.direct (when absent), the recipient join and message.sent; nothing else (no invitation, no receipt)
- [ ] repeat stage on the same request returns the same thread/message and writes nothing new
- [ ] row not running or seat moved -> error, no fact
- [ ] existing pair thread: thread.direct skipped, message added
- [ ] gate_view stays closed for a handoff row until its message exists; launch-only rows unchanged

## ht-8lem.8 — Herdr target: peer-handoff-launch child and StagingAuthor [open]
deps: ht-8lem.7, ht-8lem.6, ht-8lem.18

Spec §3.2 (stage_work change, Herdr target). Key abstraction StagingAuthor.
owns: StagingAuthor enum {Caller{call}, PeerRequest{request}} matched once at stage_work entry (PeerRequest -> one Command::PeerHandoffStage call; StagedWork gets thread+message, no invitation); new child 'internal peer-handoff-launch --request=<id> --pane=<pane>' (preflight -> stage_work(PeerRequest) -> NativeLauncher::launch with argv from the startup input template, the frozen handoff_prompt and the profile directory context; prints the launch envelope); the worker running it instead of plain 'launch --pane' for handoff plans (new tab: peer-target-create first, unchanged); failure before staging -> failed no fact; after staging -> Failed with 'staged' {thread, message}, facts stand. The child checks PEER_HANDOFF_STAGE_V1 on its daemon and maps a missing capability to failed with no fact; it derives its profile env from the stored plan's profile_dir through the same LaunchPolicy::explicit_directory_env call ht-8lem.6 validated (deterministic; no second policy) and the startup input through the same adapter API.
consumes: PeerHandoffStage (Command::PeerHandoffStage); frozen handoff_prompt (Target validation).
Files: src/cli/handoff.rs (stage_work), src/cli/handoff_delivery.rs, src/cli/internal*.rs / src/cli/mod.rs (child), src/service/peer_launcher.rs (worker), tests/multinode/remote_addresses.rs (LAUNCH_STUB answers peer-handoff-launch).
blocked-by ht-8lem.7: consumes Command::PeerHandoffStage
blocked-by ht-8lem.6: consumes frozen peer_requests.handoff_prompt
Failed.staged is built only by ht-8lem.18's recorded-answer builder: this bead records a plain failure on the peer_requests row and never sets staged itself.
blocked-by ht-8lem.18: consumes the recorded Failed answer builder

Acceptance:
- [ ] local handoffs unchanged (StagingAuthor::Caller), existing handoff tests pass untouched
- [ ] in-process/unit: the worker runs peer-handoff-launch for a handoff row (plain launch for others); the child stages before launching (stub launcher records order) and composes argv with the frozen prompt and profile env; multinode end-to-end is the integration sweep's
- [ ] bare launch --pane P@M still runs plain launch (unchanged)

## ht-8lem.9 — hc target: staged-Send gate for remote handoff [open]
deps: ht-8lem.18, ht-8lem.6, ht-8lem.7

Spec §3.2 (hc target). 
owns: gate_argv composing (config root from the LaunchDirectoryContext frozen by ht-8lem.6; the gate projects it through LaunchPolicy::explicit_directory_env as launch_gate.rs does today) the harness argv with the startup input carrying the frozen prompt and config_root Some(profile_dir); after creation the worker calls allocate_gate_seat then store::peer_handoff::stage (facts + gate record in one commit) instead of record_gate_launch for handoff plans; the gate releases only on the staged Send; a failure after staging answers Failed with 'staged'.
consumes: PeerHandoffStage (store::peer_handoff::stage, gate_view); frozen handoff_prompt (Target validation).
Files: src/service/peer_launcher.rs (gate_argv, run_to_outcome hc branch), tests/multinode/peer_launch_terminal.rs.
blocked-by ht-8lem.7: consumes store::peer_handoff::stage and gate_view
blocked-by ht-8lem.6: consumes frozen peer_requests.handoff_prompt
Failed.staged is built only by ht-8lem.18's recorded-answer builder: this bead records a plain failure on the peer_requests row and never sets staged itself.
blocked-by ht-8lem.18: consumes the recorded Failed answer builder

Acceptance:
- [ ] in-process/unit: gate_argv carries the frozen prompt and --config-root; for a handoff row the worker stages after allocate_gate_seat and the gate opens only then; multinode FakeHc end-to-end is the integration sweep's
- [ ] bare remote --new-terminal launch unchanged

## ht-8lem.10 — CLI initiator: remote handoff parse, draft and result [open]
deps: ht-8lem.1

Spec §2.1, §2.2 (CLI side), §7 upgrade text.
owns: remote branch of handoff parse accepting --summary (required for remote, Q6) and absolute --profile-dir; refusal when --summary is missing: '--summary is required for a handoff to another machine' (55 bytes); refusals for --harness-binary/--name (78 bytes), --thread/--new-thread/--thread-name/--topic/--goal (132 bytes), --invite-deadline, relative --profile-dir; --deadline -> ack_after_ms; plan with handoff_prompt and unsigned draft, then: None; run_remote sending the claim and requiring HANDOFF_ATOMIC_REMOTE_V1 (43-byte 'daemon predates remote handoffs; restart it'); removal of the follow-on TopologyHandoff for handoff plans; result line 'handoff seat= pane= thread= message=' and JSON; a Failed answer carrying 'staged' prints the failure plus '; message <m> stands in thread <t>' (31 bytes plus ids); rendering of target Refused/Failed answers in the existing '<label> refused the launch: <detail>' frame and exit codes as today, Unavailable keeps polling within the existing window; upgrade error 'machine <label> cannot take a handoff (needs peer.handoff.v1; version <v>); upgrade it to 3.0.0 or later' (94 bytes+); request_key tag v4; CLI help and user docs/skill text for remote handoff options.
consumes: Seam contract (LaunchPlan fields, SyncCommand claim, caps).
Files: src/cli/commands.rs, src/cli/mod.rs, src/cli/peer_launch.rs, docs (handoff docs), integrations/skill/SKILL.md if it documents remote handoff limits.
blocked-by ht-8lem.1: consumes SenderBlock, LaunchPlan/MessageSentPayload/SyncCommand fields and migration 0015 (boundary contract)
sp:demoted-by-session: promotion reviewer said PROMOTE (uncertainty: Q6 and the missing-summary string). Overruled: Q6 is decided provisionally (spec §2.1, Global Constraints) with the 55-byte refusal string; the HANDOFF_ATOMIC_REMOTE_V1 const comes from ht-8lem.1 and daemon behaviour is exercised in the integration sweep.
Amended by redesign cycle 1 (amend-design-1.md; spec §2.3 and 'Seat-addressed overdue (Q4)' win over older text here): obligation states are sent/launched/linked/expired with guarded monotonic transitions; overdue is the columns overdue_seq/thread_warned_at/alert_shown_at, never a state; re-sends replay the row's frozen request bytes; unlinked rows expire after OBLIGATION_TTL_MS (< 7 days).

Acceptance:
- [ ] parse tests for each refusal and for the plan contents
- [ ] body > 65536 or plan > MAX_PLAN_BYTES refused before sending
- [ ] all new strings' byte sizes stated in commit
- [ ] docs/help describe --summary/--profile-dir for remote handoffs

## ht-8lem.11 — TRUST-POLICY: N4, N5, N15 for atomic remote handoff [open]
deps: none

Spec §8. Edits TRUST-POLICY.md: N4 new bullet (embedded sender signature), N5 deadline owner, N15 title/bullet/sentence/peer-without-cap. Only N4/N5/N15 (approved in principle); wording beyond them is ht-8lem.13 (gated). The branch merges as one unit, so the policy lands in the same merge as the behaviour.
Files: TRUST-POLICY.md.


Acceptance:
- [ ] N4/N5/N15 text matches the behaviour as built (spec §8 wording, adjusted to the code)
- [ ] no wording beyond N4/N5/N15

## ht-8lem.12 — Gate: coordinator approves TRUST-POLICY wording beyond N4/N5/N15 (spec §8) [closed]
deps: none

Closed by the orchestrator when the coordinator (thread d5pFwv0Z4Ps) answers the 'Needs coordinator approval' list in spec §8 and Q1. Record the answer in run.md approvals.

Acceptance:


## ht-8lem.13 — TRUST-POLICY: approved wording beyond N4/N5/N15 [open]
deps: ht-8lem.19, ht-8lem.11, ht-8lem.12

Spec §8 'Needs coordinator approval' items, exactly as the coordinator approves them (gate ht-8lem.12): provenance peer_handoff:<node_uuid>; accepted limits ~2743-2747 and ~2760-2768 amended; H3 launch-gate paragraph ~3056; new accepted limits (2.x node holds the handoff message failed until replayed; target crash after the message transaction leaves the task with no agent, never resumed); the N5 exception wording if not already in ht-8lem.11. All approved 2026-11-04 (d5pFwv0Z4Ps #6); no Q4 accepted limit (the user chose a seat-addressed warning instead). Items the coordinator rejects are not written; record the answer in the commit.
Files: TRUST-POLICY.md.
blocked-by ht-8lem.12: Gate: coordinator approval of TRUST-POLICY wording beyond N4/N5/N15
blocked-by ht-8lem.11: consumes N4/N5/N15 text (same file, sequential edit)
Also, if approved (gate ht-8lem.19): the accepted limit 'an unanswered remote handoff stops being re-sent after OBLIGATION_TTL_MS (6 days) and is reported expired unconfirmed'.
blocked-by ht-8lem.19: Gate: coordinator approval of the expiry accepted limit

Acceptance:
- [ ] only coordinator-approved items written, matching the built behaviour
- [ ] the commit records which items were approved

## ht-8lem.14 — Pre-link overdue for handoff obligations [open]
deps: ht-8lem.3, ht-8lem.17

Spec §2.3 'Overdue before linking', Q4. Split off ht-8lem.2 (remainder half).
owns: resolution and dedup across linking: an overdue:handoff:<request> warn is cleared by the recipient's ACK (same receipt condition), and once linked, the ordinary receipt overdue for that message is not recorded again when the handoff overdue already fired; the ObligationRef::Handoff(request) arm in obligation_authored_here/record_overdue_inner; the scheduler::deadlines due scan over unlinked handoff_obligations rows: seat known and pair thread held -> warn event overdue:handoff:<request>, condition ('receipt', seat), payload = receipt overdue payload with obligation handoff and request (49 bytes more); seat unknown -> set overdue_seq on the row (state stays sent/launched; never a state 'overdue'; the seat-addressed alert's delivery is ht-8lem.16), one daemon.log line, and the peer list line 'handoff <req8> to <tag> overdue: not confirmed by that machine' (51 bytes plus id and tag) through the existing peer list read path (serde(default) protocol field if needed).
consumes: HandoffObligations ledger API of ht-8lem.17 (due query, overdue columns overdue_seq/thread_warned_at; seat known implies the pair thread is held because the requester opens it after Launched + wait_replicated, spec §2.3); obligation_authored_here COALESCE from ht-8lem.3.
Files: src/store/schema.rs (ObligationRef arm), src/scheduler/deadlines.rs, peer list path (src/service/sync_admin.rs, src/cli peer list output).
blocked-by ht-8lem.17: consumes HandoffObligations due query and state transitions
Amended by redesign cycle 1 (amend-design-1.md; spec §2.3 and 'Seat-addressed overdue (Q4)' win over older text here): obligation states are sent/launched/linked/expired with guarded monotonic transitions; overdue is the columns overdue_seq/thread_warned_at/alert_shown_at, never a state; re-sends replay the row's frozen request bytes; unlinked rows expire after OBLIGATION_TTL_MS (< 7 days).
blocked-by ht-8lem.3: consumes obligation_authored_here COALESCE(deadline_node) and the link transition

Acceptance:
- [ ] seat known + pair thread held -> one overdue:handoff:<request> warn, cleared by that seat's ACK
- [ ] seat unknown -> state overdue, one daemon.log line, peer list line (size stated in commit)
- [ ] the target node never records a handoff overdue
- [ ] target offline past the deadline, then online and the agent ACKs: exactly one overdue warning, cleared by the ACK

## ht-8lem.15 — Integration sweep: atomic remote handoff end to end [open]
deps: ht-8lem.10, ht-8lem.16, ht-8lem.4, ht-8lem.18, ht-8lem.14, ht-8lem.7, ht-8lem.3, ht-8lem.1, ht-8lem.17, ht-8lem.5, ht-8lem.8, ht-8lem.11, ht-8lem.6, ht-8lem.13, ht-8lem.9

Spec Acceptance criteria and Testing (multi-node). Configuration smoke: Herdr target (--pane P@M and --new-tab --space S@M, LAUNCH_STUB in tests/multinode/remote_addresses.rs) and hc target (--new-terminal --space @M, FakeHc in tests/multinode/peer_launch_terminal.rs). Verify the goal's main flow end to end, add the cross-task multinode tests, sweep for unwired fields/caps/config; fix small gaps inline, file blockers for big ones.
Files: tests/multinode/remote_addresses.rs, tests/multinode/peer_launch_terminal.rs, tests/multinode/*.rs (new handoff module if cleaner), src/test_support/multinode.rs.
blocked-by ht-8lem.1: consumes all leaves (integration sweep)
blocked-by ht-8lem.17: consumes all leaves (integration sweep)
blocked-by ht-8lem.3: consumes all leaves (integration sweep)
blocked-by ht-8lem.4: consumes all leaves (integration sweep)
blocked-by ht-8lem.5: consumes all leaves (integration sweep)
blocked-by ht-8lem.6: consumes all leaves (integration sweep)
blocked-by ht-8lem.7: consumes all leaves (integration sweep)
blocked-by ht-8lem.8: consumes all leaves (integration sweep)
blocked-by ht-8lem.9: consumes all leaves (integration sweep)
blocked-by ht-8lem.10: consumes all leaves (integration sweep)
blocked-by ht-8lem.11: consumes all leaves (integration sweep)
blocked-by ht-8lem.14: consumes all leaves (integration sweep)
blocked-by ht-8lem.16: consumes all leaves (integration sweep)
blocked-by ht-8lem.18: consumes all leaves (integration sweep)
blocked-by ht-8lem.13: consumes all leaves (integration sweep)

Acceptance:
- [ ] Herdr: handoff --pane P@M --summary S --profile-dir D -- task: target stores the message before the stub launch call; stub argv carries prompt (summary + from line) and the profile env; same for --new-tab
- [ ] hc: --new-terminal --space @M with FakeHc: gate holds until staging; gate argv carries prompt and --config-root
- [ ] a refused launch (bad pane, bad profile dir, bad signature, unregistered harness) authors no fact on any node and leaves no handoff_obligations row
- [ ] sender CLI killed right after the request, and separately sender daemon stopped/restarted: target writes the message and launches; message reaches the sender node; obligation state linked
- [ ] --deadline with the target partitioned before it answers: the sender's seat gets the seat-addressed alert (Q4) and peer list shows overdue; with the seat known: one in-thread overdue:handoff: warn; none on the target
- [ ] tampered sender block authored on a 3-node cluster (cluster.author_kind): failed on the other two nodes, listed in sync unapplied
- [ ] HT_TEST_WITHHOLD_CAPS=peer.handoff.v1 on the target: upgrade error, nothing sent, no obligation row
- [ ] bare launch --pane P@M and --new-terminal launch unchanged
- [ ] deadline_node on the authoring (target) node is the sender node; target never fires overdue
- [ ] no duplicate pair thread after the sender's resolve_direct_pair and the target's thread.direct cross
- [ ] a re-send while the launch is running never yields a second seat or agent
- [ ] older third node (registry with the pre-3.0 message.sent handler) marks the handoff message failed; replay after upgrade applies it
- [ ] target fails after staging: sender keeps the obligation, CLI prints the staged message, the message links when it replicates
- [ ] scripts/check-wire-compat and scripts/check-harness-neutral pass on the integrated branch

## ht-8lem.16 — Seat-addressed overdue alert for unanswered handoffs (Q4) [open]
deps: ht-8lem.14

Spec 'Seat-addressed overdue (Q4)' (user decision 2026-11-04). When the target never answers, the sender's seat gets a thread-less overdue alert naming machine and request.
owns: the pending-alert predicate over handoff_obligations (overdue_seq set, state sent, seat NULL, alert_shown_at NULL, non-retired author; expiry, linking, a late Launched, deletion and retirement are exits that bump wake_work; an unshown alert does not survive expiry — the peer list 'expired unconfirmed' line remains); wake_work bumps on every exit from it; attention digest count/token/frontier (src/store/attention.rs digest_for), queries::pending_warning_count, WAKE_PROBES probe 9 and wake_seat_attention merge (p0, wakes at every T), seats::mod_binding_view other_pending; InboxBatchV2Item::HandoffOverdue on a flagged first page behind capability inbox.seat_alerts_v1 (InboxQuery.seat_alerts), output_compact row 'warning overdue_handoff <req8> to <tag>: not confirmed by that machine' (68 bytes plus tag), assemble.rs/hook_inbox.rs mapping; AdvanceCursors.alerts (serde(default)) clearing alert_shown_at from top-level non-readonly bindings only.
consumes: overdue_seq written by ht-8lem.14's due scan; ledger columns and the alert index from migration 0015 (ht-8lem.1). The Claude mod channel and other hooks get the generic attention block only (spec Q4 'Accepted consequences'); no register.js change.
Files: src/store/attention.rs, src/store/seats/cooperative.rs, src/store/queries.rs, src/store/queries/inbox_v2/mod.rs, src/store/control.rs, src/service/dispatch.rs, src/protocol/results.rs, src/protocol/commands.rs, src/protocol/capabilities.rs, src/protocol/output_compact.rs, src/delivery/assemble.rs, src/cli/hook_inbox.rs, src/cli/mod.rs, tests/store/attention.rs, tests/store/wake_discovery_cost.rs.
blocked-by ht-8lem.14: consumes the overdue state transition with overdue_seq
Amended by redesign cycle 1 (amend-design-1.md; spec §2.3 and 'Seat-addressed overdue (Q4)' win over older text here): obligation states are sent/launched/linked/expired with guarded monotonic transitions; overdue is the columns overdue_seq/thread_warned_at/alert_shown_at, never a state; re-sends replay the row's frozen request bytes; unlinked rows expire after OBLIGATION_TTL_MS (< 7 days).

Acceptance:
- [ ] digest count, has_more and token key include the alert; check-in 'warnings: N pending' counts it
- [ ] an idle seat at T = 0, -1, 3 wakes once on a new alert and is not retyped after it is shown
- [ ] text inbox first page (capability set) shows the byte-exact row; absent without the flag and on cursor pages
- [ ] AdvanceCursors clears it from a top-level binding only (not subagent, readonly, --machine/--json)
- [ ] late Launched swaps to exactly one in-thread overdue; linking or Refused removes the alert
- [ ] an old CLI gets count and attention only; nothing breaks
- [ ] no change in src/harness, register.js or frame.rs; harness-neutral and wire-compat checks pass; byte size in commit

## ht-8lem.17 — HandoffObligations: frozen request bytes, monotonic states, expiry [open]
deps: ht-8lem.1

Owns store::handoff_obligations per the redesigned spec §2.3 (the migration 0015 DDL itself is ht-8lem.1's; this bead owns the module over it: request_bytes BLOB frozen at first insert, state CHECK sent/launched/linked/expired, overdue_seq, thread_warned_at, alert_shown_at, expires_at, and the Q4 partial index on overdue_seq IS NOT NULL AND seat IS NULL AND state='sent' AND alert_shown_at IS NULL); insert-or-ignore plus read-back so the winning row's bytes are what every send uses; the guarded transition table (Launched sent->launched, link sent or launched->linked, expiry sent or launched->expired, delete on NoCap/Refused/plain Failed guarded on sent, Failed with staged and non-answers no change; a guard miss is a no-op; linked and expired terminal); overdue as columns, never a state; OBLIGATION_TTL_MS = 6 days with a compile-time assertion < peer_requests::RETENTION_MS; the re-send set (sent or launched, overdue or not); the due query; pruning linked/expired 7 days after updated_at. Consumes the rest of migration 0015 and SenderBlock (ht-8lem.1). Consumed by ht-8lem.4 (sends row request_bytes, runs expiry before each pass and at start, answers an expired CLI poll), ht-8lem.14 (due scan sets overdue_seq/thread_warned_at instead of a state), ht-8lem.16 (alert predicate reads the columns), ht-8lem.10 (84-byte still-pending and 32-byte expired CLI lines, 63-byte --deadline refusal), ht-8lem.3 (materializer link is the guarded transition) | src/store/handoff_obligations.rs, src/store/mod.rs, migrations/node/0015_*.sql, src/store/schema.rs | covers: [Should-fix] §9 row 'Target crashes after staging, before launch'; Decisions 'Failed after staging' — Bead context: flags --parent ht-8lem --no-inherit-labels -l sp:ht-8lem; spec docs/superpowers/runs/2026-11-03-atomic-remote-handoff/2026-11-03-atomic-remote-handoff-design.md (amended by amend-design-1.md)
blocked-by ht-8lem.1: consumes migration 0015 and SenderBlock/LaunchState::Failed.staged (boundary contract)
Refused, NoCap and Failed without staged delete the row (guarded on sent), so a refused handoff never produces an overdue or alert. OBLIGATION_TTL_MS < peer_requests::RETENTION_MS is a compile-time assertion naming that constant.

Acceptance:

- [ ] row stores the frozen request bytes at first insert; every re-send replays them byte-identically
- [ ] guarded transitions: Launched sent->launched, link sent|launched->linked, expiry sent|launched->expired, delete on NoCap/Refused/plain Failed only from sent; a guard miss is a no-op; linked and expired terminal
- [ ] overdue as columns (overdue_seq, thread_warned_at, alert_shown_at), never a state; overdue rows stay in the re-send set
- [ ] OBLIGATION_TTL_MS < peer_requests::RETENTION_MS asserted at compile time; expired rows stop re-sending
- [ ] linked/expired rows pruned 7 days after updated_at

## ht-8lem.18 — Recorded Failed answer derives staged from handoff columns [open]
deps: ht-8lem.1

The one function that builds the recorded answer of a failed peer_requests row sets LaunchState::Failed.staged = Some{thread, message} exactly when handoff_thread and handoff_message are set, and serves the worker's failure, settle_at_start's interrupted rows and every repeat-id answer alike (whatever pane and unconfirmed hold). Consumes migration 0015 peer_requests columns and LaunchState::Failed.staged (ht-8lem.1); consumed by the sender ledger (keeps the row) and the CLI's '; message <m> stands in thread <t>' line (ht-8lem.10) | src/store/peer_requests.rs, src/service/peer_launcher.rs | covers: [Should-fix] §9 row 'Target crashes after staging, before launch'; Decisions 'Failed after staging' — Bead context: flags --parent ht-8lem --no-inherit-labels -l sp:ht-8lem; spec docs/superpowers/runs/2026-11-03-atomic-remote-handoff/2026-11-03-atomic-remote-handoff-design.md (amended by amend-design-1.md)
blocked-by ht-8lem.1: consumes migration 0015 and SenderBlock/LaunchState::Failed.staged (boundary contract)

Acceptance:

- [ ] the one recorded-answer builder sets Failed.staged exactly when handoff_thread and handoff_message are set, for the worker failure, settle_at_start interrupted rows and repeat-id answers
- [ ] a target crash after staging answers Failed with staged; the sender keeps its obligation

## ht-8lem.19 — Gate: coordinator approves the obligation-expiry accepted limit [open]
deps: none

Closed by the orchestrator when the coordinator (d5pFwv0Z4Ps) answers: redesign cycle 1 added OBLIGATION_TTL_MS (6 days): an unanswered remote handoff stops being re-sent and is reported 'expired unconfirmed'. Is that accepted limit approved for TRUST-POLICY?

Acceptance:


