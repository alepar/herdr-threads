# Atomic remote handoff: design (bead ht-8lem, ships in 3.0.0)

## Goal

A handoff to another machine (`handoff --pane P@M`, `--new-tab --space S@M`, `--new-terminal --space @M`) is one
signed request. The target node writes the task message before it starts the agent, so the agent starts with
the prompt and the message already there, whatever happens to the sender afterwards. `--summary` and
`--profile-dir` work as they do locally, and a refusal writes nothing.

Decisions agreed with the user are not reopened here. This document makes them concrete. Open questions where the
code contradicts or under-determines a decision are listed at the end; each has a recommendation.

## Background: today's flow

Initiator, today:
- The CLI parse of a remote handoff refuses `--harness-binary, --name, --profile-dir and --summary`
  (`src/cli/commands.rs:3874-3883`). It builds a `LaunchPlan` (target, kind, argv, cwd) and a follow-on
  `topology_handoff::Request` with `profile_dir: None` and `summary: None` (`src/cli/commands.rs:3914-3946`).
- `run_selected` runs `peer_launch::run_remote` with the caller's seat as `requester_seat`. After `Launched{seat}`
  it rewrites the action to `--existing --seat <seat>` and runs the ordinary topology handoff, so the sender's
  node authors the thread, invitation, message and ACK request (`src/cli/mod.rs:3259-3326`). TRUST-POLICY N15 calls
  this "a cross-node handoff is the sender's facts" (`TRUST-POLICY.md:1954-1988`).
- `run_remote` derives the request id from the frozen intent (`request_key`, `src/cli/peer_launch.rs:243-260`). It
  keeps that id in `node_dir/remote-launch/<key>.id` and polls `SyncCommand::RemoteLaunch` every 2 s for up to
  120 s (`src/cli/peer_launch.rs:136-138,489-679`). A missing cap gives
  `machine <label> cannot launch for other machines (needs <cap>; version <v>)`
  (`src/cli/peer_launch.rs:641-656`).
- The daemon resolves the machine and sends the frame. It answers `Launched` only once the seat has replicated
  (`src/service/sync_admin.rs:827-907`, `wait_replicated`, `REPLICA_WAIT_MS` = 5 s at line 50). The coordinator
  checks `peer.launch.v1`, plus `peer.launch.terminal.v1` for a new terminal
  (`src/sync/coordinator/remote_launch.rs:140-185`).

Wire, today:
- `LaunchRequest { id, requester_seat, plan }` and `LaunchPlan { target, kind, argv, cwd }` carry no task
  (`src/sync/frame.rs:551-576`). The caps are at `src/sync/frame.rs:67-73`, the text bounds at 75-78, and
  `LaunchState` (`Running`, `Launched`, `Refused`, `Failed`, `Unavailable`) at `src/sync/frame.rs:620-660`.
- The inbound bounds check is `malformed` (`src/sync/coordinator/remote_launch.rs:96-123`).
- `MAX_PLAN_BYTES` = 131 072 is the `peer_requests.plan` CHECK (`src/store/peer_requests.rs:25`,
  `migrations/node/0010_peer_requests.sql`), enforced by `plan_refusal` (`src/service/peer_launcher.rs:622-641`).

Target, today (`src/service/peer_launcher.rs`):
- `decide` (463-608): an existing row answers a repeat; then `plan_refusal`, placement, cwd and the hold check (all
  refusals are recorded); then a slot claim, the `peer_requests` insert (plus `open_gate` for an hc target), and a
  worker thread.
- `run_to_outcome` (1032-1155): for a new tab or terminal it runs the child `internal peer-target-create`. On hc,
  `gate_argv` (675-735) starts the launch-gate shim with `config_root: None` (line 1046). The worker then calls
  `allocate_gate_seat` and `record_gate_launch`, which releases the gate. On Herdr it runs the child
  `launch --pane=<pane> --kind=<k> -- <argv>` (`launch_args`, 738-749).
- The gate's view of a peer row: `send_staged: created.is_some()`. "There is no local Send on a peer launch"
  (`src/store/peer_requests.rs:173-205`). `service::launch_gate::authorize` releases only on `send_staged`
  (`src/service/launch_gate.rs:120-163`).
- So today an agent starts with no task. If the sender dies after `Launched`, the agent stays idle and taskless
  (accepted limit, `TRUST-POLICY.md:2743-2747`).

Local handoff, for comparison:
- The order is preflight (seat by guarded resolution), then `stage_work` (create thread, invite, send p0 with ACK,
  `src/cli/handoff.rs:153-252`), then launch (`execute_steps`, `src/cli/handoff.rs:1186-1325`;
  `HandoffLauncher`, 350-422).
- `stage_work` calls `SemanticMutation`s through `client.call(... Some(claim))`, so it assumes a local caller
  (lines 1198-1203; also `src/cli/handoff_delivery.rs:995-1010`).
- An hc handoff's gate waits for the staged Send (`src/store/topology_handoff/automatic.rs:1304-1317`).
- Prompt v1 is `summary_prompt::render`: `Handoff: <summary> — full task in thread <t> (arrives as a herdr-threads
  message; if not shown, run `…`).` The V3 variant without a thread is `render_thread_free`
  (`src/cli/handoff.rs:1800-1882`). It is selected through `frozen_prompt`, `launch_prompt` and
  `HANDOFF_SUMMARY_PROMPT_V1` (`src/cli/handoff.rs:1902-1932`, `src/protocol/capabilities.rs:73`).

Facts and signatures:
- An entry is signed `ed25519(node key, SHA-256(canonical bytes))` with `verify_strict`, in the domain set by the
  magic `HTJRNL01` (`src/sync/envelope.rs:14-29`). Handshakes use `HTPEER1-AUTH\0` (`src/sync/link.rs:62`).
- Every log's key comes from its genesis entry and is held in `sync_logs.pubkey` (`src/sync/chain.rs:38-42`,
  `src/store/journal.rs:460`).
- `message.sent` v1 (`src/sync/facts/message.rs:49-73`) is applied under `AuthorRule::SeatHome(author)`
  (`message.rs:175-190`; `src/store/materialize.rs:324-414`). `require_author` skips the check for local origin.
  An unknown payload field is ignored (`src/sync/facts/mod.rs:9-14`). A rejected fact is `failed` and can be
  replayed (N4).
- `thread.direct` is authorized from either pair seat's home node (`AuthorRule::EitherSeatHome`). It is authored by
  `resolve_direct_pair` (`src/store/control/direct.rs:248-330`).
- Pair ids are `direct_thread_id(a, b)` (`src/protocol/ids.rs:265`). Message ids are `m<tag><8>`
  (`src/protocol/ids.rs:209`). `messages.body` is at most 65 536 bytes
  (`migrations/node/0001_node_baseline.sql:546`).
- Overdue is decided only where `obligation_authored_here`: `messages.origin_node` is local
  (`src/store/schema.rs:1844-1864`). That check feeds `record_overdue_inner` (1866 ff.); the due scan lives in
  `src/scheduler/deadlines.rs`.

## Design

### 1. Wire: capability, plan fields, signed block

New capability in `src/sync/frame.rs`, next to the launch caps:

```rust
/// Capability: this node takes a LaunchPlan carrying `profile_dir`, `handoff_prompt` and `message`
/// (atomic remote handoff, TRUST-POLICY N15), writes the message before it launches, and
/// verifies `message.sent` facts that embed a sender block (N4).
pub const PEER_HANDOFF_CAP: &str = "peer.handoff.v1";
```

The cap is advertised by default. `HT_TEST_WITHHOLD_CAPS` can withhold it, as it does the launch caps
(`src/sync/coordinator.rs:824-835,941-945`).

`LaunchPlan` gains three additive fields. Each is `#[serde(default, skip_serializing_if = "Option::is_none")]` and
each carries `// cap: PEER_HANDOFF_CAP` for `scripts/check-wire-compat`:

```rust
/// An absolute directory on the receiving node: the harness profile, projected by the
/// adapter's LaunchPolicy::explicit_directory_env.
pub profile_dir: Option<String>,
/// The sender's one-line summary (summary_prompt::validate rules, 1..=200 bytes).
/// The receiving node renders the startup prompt from it (section 6).
pub handoff_prompt: Option<String>,
/// The task message, signed by the sender's node (section 1.1).
pub message: Option<SenderBlock>,
```

Rules:
- `message: Some` makes the plan a handoff plan. `handoff_prompt` and `profile_dir` are valid only in a handoff
  plan. A bare `launch` never sets any of the three, so ht-kqrt.143 and .186 are unchanged.
- The coordinator sends a plan with any of the three only to a peer that advertised `PEER_HANDOFF_CAP`. Otherwise it
  answers `LaunchAnswer::NoCap { cap: PEER_HANDOFF_CAP }` (extends `on_launch_remote`,
  `src/sync/coordinator/remote_launch.rs:153-169`). There is no fallback to the two-step path.
- `malformed` adds three checks. The body is bounded by 65 536 bytes (the `messages.body` CHECK) instead of
  `MAX_LAUNCH_TEXT_BYTES`. `handoff_prompt` must pass `summary_prompt::validate`. `profile_dir` must be at most
  `MAX_LAUNCH_TEXT_BYTES`.
- The serialized plan stays at most `MAX_PLAN_BYTES` (`plan_refusal`, unchanged). A 64 KiB plain-text body fits;
  the effective limit is content-dependent, because JSON escapes a quote or backslash to 2 bytes and a control
  character to 6.
- The initiator's CLI checks the same bounds before it sends anything. It measures the plan as the target will see
  it: the draft is serialized with placeholder values of final size in the fields the daemon fills (`sender_node`,
  `message` as a 12-byte `m<tag><8>` (1-byte prefix, 3-byte node tag, 8-byte suffix), `requested_at`, `author_role` at its longest variant, and a 128-character
  `sig`), so a plan that passes the CLI check never fails `plan_refusal` for size.

#### 1.1 `SenderBlock` and its signing bytes

New module `src/sync/handoff_block.rs` (core, neutral):

```rust
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]   // never deny_unknown_fields
pub struct SenderBlock {
    #[serde(default)] pub v: u8,                       // 1
    #[serde(default)] pub request: Uuid,               // the LaunchRequest id
    #[serde(default)] pub sender_node: Option<NodeUuid>, // signer; None only in the CLI's unsigned draft
    #[serde(default)] pub author: Option<SeatId>,      // the sender seat (a local seat of sender_node)
    #[serde(default)] pub author_role: Option<AuthorRole>,
    #[serde(default)] pub message: Option<MessageId>,  // minted by the sender's node, m<tag><8>; None in the draft
    #[serde(default)] pub body: String,
    #[serde(default)] pub priority: u8,                // 0
    #[serde(default)] pub require_ack: bool,           // true
    #[serde(default)] pub requested_at: i64,           // sender's UTC ms when it recorded the obligation
    #[serde(default)] pub ack_after_ms: Option<i64>,   // --deadline; None = ACK without deadline
    #[serde(default)] pub sig: String,                 // 128 hex chars
}
```

The sender signs what it knows: body, author seat (with role), priority, ACK policy, request id, and the message id
it minted. The recipient fills in the rest (seat, pair thread, `sent_at`) in the fact it writes. The message id is
part of the block so that the initiator can link its obligation by id (section 2.3).

Canonical signing bytes (`SenderBlock::signing_bytes`), all integers big-endian, each string a u32 length followed
by its UTF-8 bytes:

```
"HTHOFF01" (8-byte domain magic, new) || v u8 || request[16] || sender_node[16]
|| str(author) || str(author_role or "") || str(message) || str(body)
|| priority u8 || require_ack u8 || requested_at i64
|| has_deadline u8 || ack_after_ms i64 (0 when None)
```

- `sig = hex(ed25519(sender key, SHA-256(bytes)))`, checked with `verify_strict`, the same primitive as entries.
- `HTHOFF01` can never be the opening bytes of an entry (`HTJRNL01`) or a handshake (`HTPEER1-AUTH\0`), so a block
  signature can never be replayed as either, nor either one as a block.
- The three identity fields are `Option` because `NodeUuid`, `SeatId` and `MessageId` have no `Default` and
  validate on deserialize; `serde(default)` gives `None`, never a sentinel. `signing_bytes` and `verify` take a
  block whose three fields are `Some`; a block with any of them `None` fails `verify` with `BlockError::Incomplete`
  before any signature work, so a signed block on the wire or in a fact always carries all three.
- `verify(&self, pubkey: &[u8; 32]) -> Result<(), BlockError>` also enforces `v == 1`, `priority == 0`,
  `require_ack`, `ack_after_ms` in `1..=MAX_ACK_AFTER_MS` when set (`MAX_ACK_AFTER_MS` = 365 days, a new const in
  `handoff_block.rs`: the wire bound every verifier applies), and that `deadline_at()` is defined. The sender
  applies a tighter bound of its own: a remote `--deadline` is at most `OBLIGATION_TTL_MS` (6 days, §2.1, §2.3),
  so every deadline falls inside the obligation's lifetime.
- `SenderBlock::deadline_at() -> Option<i64>` is the one place that computes `requested_at + ack_after_ms`, with
  `checked_add`; an overflow is `BlockError::Deadline`. The sender (§2.3), the target (§4.1) and every authority
  check (§4.2) call it; none adds the two fields itself.

### 2. Initiator side

#### 2.1 CLI parse (`src/cli/commands.rs:3870-3946`)

The remote branch keeps its target forms and changes in these ways:
- `--summary` (validated at line 3841) is required and `--profile-dir` is accepted for `--new-tab` and
  `--new-terminal`; with `--pane` it is refused before sending with the local guarded-pane text
  `--cwd and --profile-dir require --new-tab; guarded pane launches cannot change directories` (90 bytes, an
  existing string). A missing `--summary` is refused
  before sending: `--summary is required for a handoff to another machine` (55 bytes) (Q6, provisional).
- `--profile-dir` is accepted for `--new-tab` and `--new-terminal` (a pane refuses it, above). A relative
  `--profile-dir` is refused before sending: `--profile-dir for another machine must be an absolute path there`.
- A `--cwd` or `--profile-dir` for another machine that is not UTF-8 is refused before sending, not converted
  lossily: `--cwd and --profile-dir for another machine must be UTF-8` (57 bytes).
- The initiator does not resolve or check `--profile-dir` locally, because the path is on the target. The
  target decides (section 3.1). With no `--profile-dir`, the plan carries none, and the target's default root
  applies (see Q3).
- `--harness-binary` and `--name` stay refused, with the text trimmed to
  `--harness-binary and --name are not supported for a handoff to another machine` (78 bytes).
- `--thread`, `--new-thread`, `--thread-name`, `--topic` and `--goal` are refused:
  `--thread, --new-thread, --thread-name, --topic and --goal are not supported for a handoff to another machine (it opens the 1:1 pair)`
  (132 bytes). The recipient writes the pair thread (Q2).
- `CliAction::RemoteLaunch` carries the plan with `handoff_prompt = summary` and an unsigned draft (`message`
  with `sig` empty and `sender_node`, `message`, `requested_at` unset). `then: None`: there is no follow-on
  handoff.
- `--deadline` becomes `ack_after_ms`. A `--deadline` over `OBLIGATION_TTL_MS` (6 days, §2.3) is refused before
  sending: `--deadline for a handoff to another machine is at most 6 days` (61 bytes). The cap keeps every
  deadline at or before the obligation's expiry, so the pre-link overdue always fires while the sender still holds
  the row (§2.3 "Expiry"). `--invite-deadline` is refused for a remote handoff, because a pair has no invitation.

#### 2.2 Run (`src/cli/mod.rs:3259-3326`, `src/cli/peer_launch.rs`)

- `run_selected` keeps its top-level and check-in requirements (`src/cli/mod.rs:3251-3257`).
- `run_remote` now sends the caller's claim with the request. `SyncCommand::RemoteLaunch` gains
  `#[serde(default)] claim: Option<CallerClaim>`. This is the local protocol, gated by the new local capability
  `HANDOFF_ATOMIC_REMOTE_V1 = "handoff.atomic_remote_v1"` in `src/protocol/capabilities.rs`. A CLI talking to a
  daemon without it gets `daemon predates remote handoffs; restart it` (43 bytes).
- `run_remote` then waits for `Launched{seat}` exactly as today. The follow-on `TopologyHandoff` and the
  `journaled` record hook (`src/cli/mod.rs:3265-3324`) are removed for handoff plans.
- On `Launched` the CLI settles the request file and prints the result:
  - text: `handoff seat=<seat> pane=<pane>@<tag> thread=<thread> message=<message>\n` (38 bytes plus the five
    values; `thread` is `direct_thread_id(sender, seat)`); ` message=<message>` is omitted when the target's
    answer names no message (JSON: `"message": null`);
  - JSON: `{"handoff":{"outcome":"started","seat","pane","machine","thread","message"}}`.
- `request_key` (`src/cli/peer_launch.rs:243-260`) hashes the plan, so the draft (body, summary, profile dir) is
  part of the intent. The tag is `remote launch v4` for a handoff plan; a plain launch keeps `v3`, so a pending 2.x
  launch request file is still found after the upgrade.
- A rerun of the same command reuses the id and gets the recorded outcome (unchanged). A rerun after a recorded
  refusal gets a new id (unchanged).
- When the 120 s poll ends without an answer, a handoff plan prints, instead of today's `rerun to resume` line:
  `handoff to <label> still pending: request <req8> stays queued until it starts or expires (6 days)` (84 bytes
  plus label and id). The request is not abandoned: the daemon keeps re-sending it (§2.3) until it links or its
  obligation expires. A rerun while the row is `sent` or `launched` reuses the id and reads the same state.
- **Terminal rows are answered from the ledger, never re-sent.** A poll whose id has a `linked`, `expired` or
  `failed` row (§2.3) gets its answer from the sender's ledger, with no frame to the target, so a terminal row is
  never decided afresh after the target's duplicate guard lapsed. The CLI settles the request file on each of
  these answers, so the next rerun mints a new id (as after a refusal); it never revives the row:
  - `linked`: the started result without the pane, which the ledger does not hold:
    `handoff seat=<seat> thread=<thread> message=<message>\n` (31 bytes plus the three values); JSON as on
    `Launched`, with `"pane": null`.
  - `expired`, seat unknown: `handoff <req8> to <label> expired unconfirmed` (32 bytes plus id and label).
  - `expired`, seat known (the row was `launched`, the message never linked):
    `handoff <req8> to <label> expired unlinked: seat <seat>` (36 bytes plus id, label and seat).
  - `failed`: the stored answer, printed exactly as when it first arrived (refusal frame, upgrade error, or
    failure text).
  The carrier is a local-only `RemoteLaunchAnswer` variant (Decisions, "Ledger answers"); `LaunchState` and the
  peer wire are unchanged.
- **Stale request file.** The request file is written once, when its id is minted, so its mtime is the mint
  time. A file older than `OBLIGATION_TTL_MS` is sent with `#[serde(default)] resume_only: bool` set on
  `SyncCommand::RemoteLaunch`. Any row for such an id is terminal by then (an unlinked row expires at
  `OBLIGATION_TTL_MS`), so the daemon answers it from the ledger as above. With no row (the terminal row was
  pruned, 7 days after it ended), the daemon signs nothing and answers `HandoffLedger { outcome: Unknown }`; the
  CLI then settles the file and continues as a new command under a new id, a new handoff that prints its own
  result. The old id is never signed again, so a pruned id is never revived.

#### 2.3 Daemon: sign, record the obligation, link it later

In `SyncAdmin::remote_launch` (`src/service/sync_admin.rs:827-907`), for a request whose plan carries a `message`
draft:

1. **Check the claim.** It is checked with the ordinary cooperative claim check (`seats::cooperative_claim`, A2).
   The claimed seat must be a local, nonretired seat with an open top-level binding, and must equal
   `request.requester_seat`. Otherwise `CallerUnverified`. The block names only a local seat, so N5 holds on this
   node.
2. **Sign once.** A row in `linked`, `expired` or `failed` is answered from the ledger and nothing is sent
   (§2.2); a `resume_only` request with no row is answered `Unknown` and nothing is signed. A draft whose
   `ack_after_ms` exceeds `OBLIGATION_TTL_MS` is refused with no row and nothing sent (a bug guard behind the
   CLI's 61-byte refusal). If no obligation row exists for `request.id`, the daemon mints the message id with this node's
   public-id allocator (`m<tag><8>`), sets `requested_at` (now), `sender_node` and `author_role` (the binding's
   role, `schema::open_binding_role`, read only here), and signs with `NodeIdentity::sign`
   (`src/node/identity.rs:183`). `SenderBlock::deadline_at()` gives `deadline_at` (or NULL). The signed block is
   checked with `verify` before the insert; a block that fails (an overflowing deadline cannot pass the CLI's
   bound, so this is a bug guard) is refused with no row and nothing sent.
3. **Record the obligation (one write).** `INSERT OR IGNORE INTO handoff_obligations(request, target_node, author,
   message, requested_at, deadline_at, request_bytes, state='sent', seat NULL, created_at, expires_at)`, where
   `request_bytes` is the serialized signed `LaunchRequest` (id, requester seat, plan with the signed block) and
   `expires_at = created_at + OBLIGATION_TTL_MS`, with `created_at = requested_at` (one clock read), so
   `deadline_at <= expires_at` for every row (the deadline cap of §2.1). The row is read back, and the send uses the row's
   `request_bytes`, never the freshly signed value: a repeat poll or a concurrent first send whose insert was
   ignored sends the bytes of the row that won.
4. **Send** the row's `request_bytes` as today. Answers settle the obligation row (§ Ledger states).

New node-local table (migration `0015_handoff_obligations.sql`, `STRICT`, never replicated):

```sql
CREATE TABLE handoff_obligations (
    request TEXT PRIMARY KEY CHECK(length(request) = 36),
    target_node TEXT NOT NULL CHECK(length(target_node) = 32),
    author TEXT NOT NULL REFERENCES seats(id),
    message TEXT NOT NULL UNIQUE,
    requested_at INTEGER NOT NULL,
    deadline_at INTEGER,
    request_bytes BLOB NOT NULL,          -- the signed LaunchRequest, frozen at first send
    seat TEXT,
    state TEXT NOT NULL CHECK(state IN ('sent','launched','linked','expired','failed')),
    answer TEXT,                          -- 'failed' only: the serialized RemoteLaunchAnswer that ended it
    overdue_seq INTEGER,                  -- set once by the due scan when deadline_at passes (Q4)
    thread_warned_at INTEGER,             -- set when the in-thread overdue:handoff warning is recorded
    alert_shown_at INTEGER,               -- Q4 seat-addressed alert shown (NULL = pending)
    created_at INTEGER NOT NULL,
    expires_at INTEGER NOT NULL,          -- created_at + OBLIGATION_TTL_MS
    updated_at INTEGER NOT NULL
) STRICT;
```

**Frozen bytes.** Every send of a request, first or repeat, from the CLI's poll or the daemon's re-send task,
transmits `request_bytes` unchanged. Nothing is rebuilt from live bindings or seat rows, so a role change, a
restart or a CLI death between sends cannot change the signed bytes.

**Ledger states.** `sent → launched → linked`, plus the terminal `expired` and `failed`. Every transition is one
guarded `UPDATE … WHERE request=? AND state IN (<allowed prior states>)`; a write that matches no row is a no-op,
never an error. `linked`, `expired` and `failed` never change again.

| Event | Guard (prior state) | Effect |
|---|---|---|
| `Launched{seat}` (after `wait_replicated`) | `sent` | `seat=<seat>, state='launched'` |
| `message.sent` applies here with a block whose `sender_node` is this node (materializer, same transaction) | `sent`, `launched` | `seat=<recipient>, state='linked'` |
| TTL passes (`now >= expires_at`) | `sent`, `launched` | `state='expired'` (after the row's due step, see "Expiry") |
| `NoCap`, `Refused`, or `Failed` without `staged` | `sent` | `state='failed'`, `answer=<the answer>`: nothing was authored anywhere, nothing is re-sent |
| `Failed` with `staged` | any | no change: the facts exist, the row links when the message replicates |
| `Unavailable`, timeout, unreachable, not replicated | any | no change |

So a `Launched` that settles after the message already linked the row matches no row and changes nothing, and a
late `Launched` for a row that went overdue moves it forward normally. A `NoCap`/`Refused`/plain `Failed` answer
can only follow a first decision, so the `failed` transition is guarded on `sent` too.

**The daemon owns the request.** For each row in `sent` or `launched` (overdue or not: overdue is a column, not a
state), the daemon re-sends `request_bytes` on each link-up to `target_node` and at most every 30 s. A repeat id
only ever gets the recorded answer (`peer_launcher::decide`, lines 466-486), never a second launch. So a dead CLI
loses nothing: every answer the daemon learns lands in the ledger and stays visible after the CLI is gone.

**Failure learned without the CLI.** The `failed` transition records the answer whoever fetched it. When it was
returned to a polling CLI, which printed it, the same transaction sets `alert_shown_at = now` (the author has seen
it). When the re-send task fetched it (the CLI timed out or died), it writes one `daemon.log` line, `peer list`
shows `handoff <req8> to <tag> failed: <detail>` (21 bytes plus id, tag and the answer's one-line detail) until the
row is pruned, a rerun of the command prints the stored answer (§2.2), and a row with `deadline_at` keeps
`alert_shown_at` NULL, so at its deadline the due scan raises the seat-addressed alert (Q4) as for an unanswered
request. A row without a deadline gets the log and `peer list` lines only, like an unanswered one. This covers a
target that fails before staging (worker or child failure, Herdr preflight failure, or a restart that
`settle_at_start` records as `failed: interrupted` with no handoff columns).

**Expiry.** `OBLIGATION_TTL_MS` = 6 days, a const in `store::handoff_obligations` with a compile-time assertion that
it is strictly less than `peer_requests::RETENTION_MS` (7 days). The target prunes a row only `RETENTION_MS` after
it first received the request, which is no earlier than the sender's `created_at` up to clock skew, so the one-day
margin keeps every re-send inside the target's duplicate guard and no late re-send is ever decided afresh. The
re-send task runs the expiry transition before each pass and at daemon start. Every unlinked row expires, with or
without a deadline. A remote deadline is at most `OBLIGATION_TTL_MS` (§2.1) and `created_at = requested_at`, so
`deadline_at <= expires_at`: a deadline always falls due while the row is still scanned. To close the boundary
(a row whose deadline and expiry fall in the same pass), the expiry transition first runs the row's due step
("Overdue before linking") in the same transaction when `deadline_at <= now` and that step has not run, so no
deadline-bearing row expires without its overdue. On expiry, re-sends stop, `daemon.log` gets one line, and
`peer list` shows, until the row is pruned, `handoff <req8> to <tag> expired unconfirmed` (32 bytes plus id and
tag) when the seat is unknown, or `handoff <req8> to <tag> expired unlinked: seat <seat>` (36 bytes plus id, tag
and seat) when the row was `launched`: the target confirmed the launch, but the message never linked here. A
`message.sent` that arrives after expiry still applies and still drives the ordinary receipt path (§4.3); only
the ledger row stays `expired`.

**Pruning.** `linked`, `expired` and `failed` rows are deleted 7 days after `updated_at`, at daemon start and then at most hourly
(`PRUNE_EVERY_MS`, in the re-send pass), like `peer_requests`.

**Linking.** The materializer's link is the guarded transition above, in the same transaction as the
`message.sent` apply. From then on the ordinary receipt obligation of that message is the deadline record (section
4.3).

**Overdue before linking.** Overdue is recorded in columns, never as a state, so an overdue row stays in the re-send
set and moves forward on a late `Launched` or a link. The due scan reads rows with `deadline_at <= now` and either
`state IN ('sent','launched')` or `state = 'failed' AND alert_shown_at IS NULL AND overdue_seq IS NULL` (a failure
nobody saw, see "Failure learned without the CLI"), and in one transaction per row (the row's due step):
- if `overdue_seq IS NULL`, sets `overdue_seq = schema::next_decision_seq` (the moment the row went overdue; with
  `seat IS NULL` this raises the seat-addressed alert, "Seat-addressed overdue (Q4)");
- if `seat IS NOT NULL` and `thread_warned_at IS NULL`, decides it through `record_overdue_if_pending` with a new
  `ObligationRef::Handoff(request)` and sets `thread_warned_at`. The warning is a `warn` event in the pair thread,
  key `overdue:handoff:<request>`, condition `('receipt', <seat>)`. The ACK that later clears the receipt condition
  clears it too. The payload is the receipt overdue payload with `"obligation":"handoff"` (same length as
  `"receipt"`) and `,"request":"<uuid>"`, 49 bytes more than a receipt overdue.
  - To hold that thread when the target is offline, the sender's node opens the pair itself after `Launched` and
    `wait_replicated`, through `resolve_direct_pair` (`src/store/control/direct.rs:248`). N4 allows it: the
    sender's node is home to one of the pair's seats. When the target's `thread.direct` arrives, it is the same
    `d…` id, and the existing crossed-create path skips it.
- The due batch is bounded (`HANDOFF_DUE_BATCH_LIMIT` rows that advance per pass) and pages past rows the step
  cannot move (a seat known, the pair thread not held yet), so they never starve another row's overdue; a row whose
  step, expiry or re-send errors is logged to `daemon.log` and skipped, never fatal to the pass.
- `expired` and `linked` rows are never scanned (an expiring row ran its due step first, see "Expiry"); a linked
  row's deadline is the ordinary receipt path.

With the seat unknown (no `Launched` answer yet), the overdue goes to the author's seat as the seat-addressed alert
(Q4), plus one `daemon.log` line; `peer list` shows `handoff <req8> to <tag> overdue: not confirmed by that machine`
(51 bytes plus the id and tag) while `overdue_seq IS NOT NULL AND seat IS NULL AND state='sent'`. The alert itself
outlives expiry and failure: it stays pending until it is shown, the seat becomes known, the row is pruned or the
author retires ("Seat-addressed overdue (Q4)", Clearing). When a late
`Launched{seat}` arrives, the next due scan records the in-thread warning.

### 3. Target side

#### 3.1 Validation in `decide` (refusal writes nothing)

For a handoff plan, after the existing checks (lines 487-528) and before the slot claim and insert, in this order.
The first failure answers `Refused` and is recorded in `peer_requests` only, which writes no fact.

1. **Link cap.** The link carries `PEER_HANDOFF_CAP`. Otherwise the frame is dropped, as `on_launch_request` does
   for a missing `peer.launch.v1`.
2. **Block shape and identity.**
   - `block.sender_node == from` (N6 binds the link to the node).
   - `block.author == request.requester_seat`.
   - `block.request == request.id`.
   - Sender key held: `sync_logs` has a row for `from`. Otherwise (the peer's genesis has not replicated yet,
     for example right after pairing: the link authenticates from the hello key and writes no `sync_logs` row)
     the answer is `Unavailable{reason: "sender node key not replicated here yet"}` (39 bytes), not recorded, so
     the requester asks again, exactly as for a sender seat not held yet (step 3).
   - The signature verifies under `sync_logs.pubkey` for `from`.
   - Failure detail: `handoff message does not verify against its sender node` (55 bytes).
3. **Sender seat.** The author is a seat this node holds as remote, with `home_node == from`, nonretired.
   - Not held yet: `Unavailable{reason: "sender seat not replicated here yet"}` (35 bytes). This is not recorded,
     so the requester asks again.
   - Held with another home: `Refused`, `the sender seat is not that node's seat here` (44 bytes).
4. **Profile directory.** When `profile_dir` is set, it must be absolute, `is_dir()`, and the adapter's
   `LaunchPolicy::explicit_directory_env` must accept it (`src/harness/adapter.rs:1300`, through
   `launch_registration`). The result is frozen into a `LaunchDirectoryContext`
   (`src/cli/topology_handoff.rs:118-153` is the local analogue).
   - Failure detail: `profile directory is not an existing absolute directory here` (60 bytes), or the adapter's
     own error.
   - A pane plan (`LaunchTarget::Pane`) carrying `profile_dir` is refused in `plan_refusal`, before placement and
     for a bare plan as well: `a pane launch cannot change the profile directory` (49 bytes), shown in the
     existing refused frame. Herdr's `agent.start` carries argv only, so a pane cannot take the directory's env.
5. **Harness startup input.** The registered harness must have handoff startup input
   (`prepare_startup_input`, as `HandoffLauncher::select_startup_input`), else `Refused` with the existing
   `selected adapter has no handoff startup input`.

The CLI shows each of these in its existing frame, `<label> refused the launch: <detail>`
(`src/cli/peer_launch.rs:584-590`).

When all five checks pass, `decide` renders the prompt (section 6) from the sender seat row step 3 just verified
and freezes it with the row insert. It is rendered here, not at staging, because hc's `gate_argv` needs it before
the terminal is created (§3.2); the Herdr child and `gate_argv` both read the frozen value, and staging never
renders it again.

`peer_requests` gains two columns (migration 0015): `handoff_thread TEXT` and `handoff_message TEXT`, set together
by the staging write. It also gains `handoff_prompt TEXT`, the rendered prompt, frozen in `decide` at row insert.

#### 3.2 Order: seat, then one transaction, then launch

This mirrors local handoff (`preflight`, then `stage_work`, then `launch`):

1. **Seat.** The seat is allocated by ordinary guarded resolution of the target (pane, new tab's root pane, or new
   terminal). This is the step that today's `launch` child (Herdr) or `allocate_gate_seat` (hc) already does.
2. **Stage, in one store transaction** (new `store::peer_handoff::stage`, called under the domain writer):
   - Re-check that the `peer_requests` row is `running` and that the seat is still the target's.
   - Author `thread.direct { thread: direct_thread_id(author, seat), creator: Some(author) }` if the thread is absent.
   - Author the recipient's own `membership.joined` (the sender's home node authors the sender's join when it
     applies `thread.direct`, N4, unchanged).
   - Author `message.sent` (section 4.1).
   - Set `peer_requests.handoff_thread` and `handoff_message` (`handoff_prompt` was frozen in `decide`, §3.1, and
     is not touched here). For hc, also set
     `record_gate_launch` (creation, terminal, seat), so the gate opens in the same commit.
   - Idempotent on the request id: a row that already names `handoff_message` returns it unchanged.
3. **Launch.**

`stage_work` change: the three caller-keyed `SemanticMutation`s cannot run without a caller. A new
`StagingAuthor` enum makes the caller explicit:
- `StagingAuthor::Caller { call }` is today's closure, used by `execute_steps` and `handoff_delivery`.
- `StagingAuthor::PeerRequest { request }` makes one call to the new local `Command::PeerHandoffStage { request,
  seat }`. The daemon runs step 2 for it and answers `PeerHandoffStaged { thread, message }`.
- It is served only for a `peer_requests` row of this node in `running` with a block. It has no caller claim; the
  request row is the authority (N15).
- It is gated by a local capability `PEER_HANDOFF_STAGE_V1 = "handoff.peer_stage_v1"`.

`StagedWork` gets a `thread` and a `message` from it, and no `invitation` (`invitation_attempted` stays false).

**Herdr target (no gate).** The worker stops running plain `launch --pane` for a handoff plan:
- New tab only: `internal peer-target-create` runs first, with `--directory-context=<frozen context>` for a
  handoff plan, so the tab is created with the profile env (as local `CreateTabRequest.env`); the agent started in
  its root pane inherits it. hc is unchanged (its gate argv carries `--config-root`).
- Then a new child, `internal peer-handoff-launch --request=<id> --pane=<pane>`. It has the same scrubbed
  environment, `PR_SET_PDEATHSIG` and timeout as today (`Worker::child`, lines 983-1013). It runs the
  local-handoff pipeline:
  - `NativeLauncher::preflight`: guarded resolution gives the seat, as `launch` does.
  - `stage_work` with `StagingAuthor::PeerRequest`.
  - `NativeLauncher::launch`, with argv composed through the startup input template from the frozen prompt and
    `directory_context`. Its `gate(true)` marks a possible start.
  - It prints the launch envelope, which the worker reads as today (`json_envelope_seat`).
- A child that fails before staging records `failed` with no fact.
- A child that fails after staging and before submit records `failed`. The facts stand, and the message waits on a
  seat that never started (Q5).
- `LaunchState::Failed { unconfirmed }` keeps its meaning.

**hc target (normal staged Send gate).**
- `gate_argv` (`src/service/peer_launcher.rs:675-735`) composes the harness argv with the startup input carrying
  the prompt, which is renderable before creation because it names no thread (section 6). It passes
  `config_root: Some(profile_dir)` instead of `None` (line 1046).
- After creation, the worker allocates the seat (`allocate_gate_seat`) and calls `store::peer_handoff::stage`,
  which writes the facts and the gate record in one commit.
- `peer_requests::gate_view` (`src/store/peer_requests.rs:174-205`) returns
  `send_staged = created.is_some() && (handoff_message IS NULL || the message row exists)`. So a handoff row
  releases only on its staged Send, as a local hc handoff does. A launch-only row releases on creation as today.

#### 3.3 What a peer request may author

N15 today says a request "cannot send, ACK, invite, join, …". A handoff plan may cause exactly three facts, all
written by this node in one transaction, decided here (A2):
- the pair's `thread.direct`;
- the recipient seat's own `membership.joined`;
- one `message.sent` whose author is the sender seat and whose sender block verifies.

Nothing else. In particular: no invitation, no receipt settlement, and no action attributed to the sender seat
here other than the embedded authorship it signed.

### 4. The fact and its verification on every node

#### 4.1 Shape: `message.sent` v1 plus one additive field

`MessageSentPayload` (`src/sync/facts/message.rs:49-73`) gains:

```rust
/// Set only on a handoff message authored by the recipient's home node for a remote sender (N4, N15).
#[serde(default, skip_serializing_if = "Option::is_none")]
pub handoff: Option<SenderBlock>,
```

Same kind, same `MAJOR = 1`. A minor change by the version rules (`src/sync/facts/mod.rs:9-14`).

The target authors it with these values:
- `message = block.message`, `author = block.author`, `author_role = block.author_role`, `body = block.body`;
- `priority = 0`, `require_ack = true`;
- `sent_at` = the target's decision time;
- `duration_ms = ack_after_ms.unwrap_or(0)`;
- `recipients = [{ seat, receipt: Ack, deadline_at: block.deadline_at() }]`, so the deadline is
  anchored on the sender's clock, which owns it;
- `thread = direct_thread_id(author, seat)`;
- `handoff = Some(block)`;
- every other payload field at its default: `relays_user = false`, `user_intent = None`, and any field added to
  `MessageSentPayload` later at its `serde(default)` value. The block signs no human relay or human intent claim,
  so the recipient's node may not attach one (TRUST-POLICY "Recorded message claims", "Recorded human intent").

`needs` are unchanged: thread, author, recipient.

#### 4.2 Authority rule (`MessageSent::apply`, `src/store/materialize.rs`)

When `payload.handoff` is `Some(block)`, `AuthorRule::SeatHome(author)` is replaced by
`AuthorRule::HandoffRecipientHome { block, recipient }`. It passes iff all of these hold:
- there is exactly one recipient, with `receipt: Ack`;
- the origin is that recipient's home node;
- `thread == direct_thread_id(author, recipient)`;
- the payload equals the block in `message`, `author`, `author_role`, `body`, `priority == 0`, `require_ack`,
  `duration_ms`, and `deadline_at == block.deadline_at()`;
- every payload field the block does not carry is at its default: `relays_user == false`, `user_intent == None`.
  The predicate is a whole-payload comparison: it builds the expected `MessageSentPayload` as a full struct
  literal (no `..` rest) from the block, the recipient, the thread and the payload's own `sent_at`, with every
  other field at its default value, and compares it to the payload with `==`. A field added to
  `MessageSentPayload` later fails to compile there until the predicate pins it;
- the author's replicated `home_node == block.sender_node`;
- `block.verify(pubkey)` holds, where `pubkey` is read from `sync_logs` for `block.sender_node` by a new
  `store::materialize::node_keys(conn, node)` (newest incarnation first; the block verifies against any of them, and the target's
  pre-launch sender check reads the same helper). All incarnations of a node share one key (N1).

Any failure is `FactError::Rejected`, so the entry is `failed`, visible in `sync unapplied`, and replayable.

The check reads only the fact and replicated identity, never `peer_requests`, so every node rejects the same facts:
- `needs` names the author seat. The author's `seat.created` comes from the sender's log. So any node that applies
  the message already holds that log's genesis, and with it the key. The rule never depends on arrival order.
- `require_author` still skips the check for local origin. The authoring node checks the same predicate before
  writing (section 3.2, step 2), so its own fact can never fail elsewhere.
- `scripts/check-wire-compat`: the field is `serde(default)`, and the block is a new type, which grows the baseline.

**Older (2.x) third node.** It ignores the unknown `handoff` field and applies `SeatHome(author)`. The origin is the
recipient's node, so it rejects the fact and marks it `failed`. After the upgrade, `sync unapplied replay` applies
it. This is best effort and documented (release notes and `sync unapplied` help): the 2.x node shows the message as
missing until someone replays. All other sync with 2.x nodes is unchanged.

#### 4.3 Deadline owner (N5)

- `messages` gains `deadline_node TEXT CHECK(deadline_node IS NULL OR length(deadline_node)=32)` (migration 0015).
  It is set by `materialize_message` to `block.sender_node` on every node, and is NULL otherwise.
- `obligation_authored_here` (`src/store/schema.rs:1853-1855`) reads
  `COALESCE(deadline_node, origin_node) IS NULL OR … = local`.
- So the recipient's node never fires overdue for it, and the sender's node does: before linking through
  `handoff_obligations`, and after linking through the ordinary receipt path.
- Wakes, delivery rows and catch-up stay on the recipient's home node (N5, unchanged).

### 5. Sender's node applying its own seat's message from another origin

`materialize_message` must handle `author` as a local seat with `local_origin == false`. Today a local author
implies local origin. Two places to check:
- The author's sibling delivery row and the "sent" views (`src/store/messages.rs`, `check_staged_audience` area,
  1220-1260): there is no preparation for this message here, so it materializes as a replicated message whose
  author happens to be local, with no sender binding.
- `messages.sender_binding_id` stays NULL.

There is a unit test for each (Testing).

### 6. Prompt renderer v2 (one renderer, local and remote)

New `summary_prompt::render_v2(summary: Option<&str>, from: &SenderLine) -> String` in `src/cli/handoff.rs`. Its
data comes from the store only.

One line on every host (Q1), with one renderer constant ` — from: ` (11 bytes: space, U+2014 (3 bytes), space,
`from: `):

```
<summary> — from: <seat name> (<seat id>)    when the sender seat has a name
<summary> — from: <seat id>                  when it has none
```

- `<summary>` is `--summary`, or `--goal` when it passes `validate` (`summary_prompt::select`, unchanged). With
  neither, it is the fixed line `Handoff` (7 bytes); this applies to local handoffs only, since a remote
  handoff requires `--summary` (Q6).
- The name is the replicated `seats.name` (named seats' `display_name`, `src/store/seat_directory.rs:117-141`),
  without `@tag`, since the id disambiguates. There is no thread id and no inbox hint.
- Sizes, for a 12-byte seat id:
  - fixed overhead without a name: ` — from: ` plus id = 23 bytes;
  - fixed overhead with a name: ` — from: ` + ` (` + `)` + id = 26 bytes, plus the name (at most 64);
  - largest prompt: 200 + 26 + 64 = 290 bytes;
  - v1 for comparison was at least 76 bytes plus the command and thread.
- **Remote:** the recipient's herdr-threads renders the from line from its replicated seat row in `decide`, at
  row insert (§3.1), and freezes the whole prompt in `peer_requests.handoff_prompt`.
- **Local:** new plans freeze `SenderPrompt { summary: Option<String>, from: String }` in a new plan field
  `sender_prompt`, beside `summary_prompt`. It is gated by the new local capability
  `HANDOFF_SENDER_PROMPT_V2 = "handoff.sender_prompt_v2"`. `SummaryPrompt` is `deny_unknown_fields`
  (`src/protocol/handoff.rs:302-307`), so a sibling field and a capability are the compatible path, mirroring
  `frozen_prompt`.
  - `launch_prompt`, `pane_prompt` and `render_thread_free`'s V3 call site (`src/cli/topology_handoff.rs:389`) use
    `render_v2` when `sender_prompt` is set.
  - Retained records keep `HANDOFF_SUMMARY_PROMPT_V1` and `render`/`render_thread_free`/`bootstrap_v1` byte for
    byte. `HANDOFF_SUMMARY_PROMPT_V1` stays frozen.
- **No line break (Q1).** The prompt contains no `\n` on any host, so it passes `NativeLaunchRequest::validate_argv`
  (`src/ports.rs:2147-2151`) on Herdr and hc alike. There is no host-dependent separator and no `HostKind` method
  for it.

### 7. Compatibility (3.0.0, major)

| Pair | Behaviour |
|---|---|
| 3.0 sender → 3.0 target | atomic path above |
| 3.0 sender → 2.x target (no `peer.handoff.v1`) | refused, nothing sent, obligation row `failed` (§2.3). The CLI prints `machine <label> cannot take a handoff (needs peer.handoff.v1; version <v>); upgrade it to 3.0.0 or later`: 94 bytes plus label and version, replacing the generic NoCap text for this cap only (`src/cli/peer_launch.rs:641-656`) |
| 2.x sender → 3.0 target | the 2.x two-step path still works: a plain plan launches, and the 2.x sender authors its follow-on handoff itself |
| `launch --pane P@M`, `--new-terminal --space @M` (no handoff) | unchanged, works with 2.x peers |
| 2.x third node | the handoff `message.sent` is `failed` there, and replayable after upgrade (section 4.2) |
| Handoff into an existing remote seat (`--existing --seat X@M`) | unchanged: sender-authored facts (N15 bullet 2) |

### 8. TRUST-POLICY edits (same commit as the code)

Approved in principle. The concrete wording follows.

**N4**, new bullet after "the author seat's home node for `message.sent`":
> - except a handoff `message.sent` that embeds a sender block (`handoff`, N15): it is authorized from the home node
>   of its single ACK recipient, in the 1:1 pair thread of author and recipient, when the block's signature
>   (domain `HTHOFF01`) verifies under the genesis key of the block's `sender_node`, that node is the author's
>   replicated home node, and the payload repeats the block's message id, author, role, body, priority, ACK
>   policy and deadline. Every input is in the fact or replicated identity (never `peer_requests`), so every node
>   decides it alike. A node that cannot verify (older software) marks it `failed`; it is replayable after
>   upgrade (best effort).

**N5**, after "the sender's node owns the deadline and overdue decision for an obligation it authored":
> For a handoff message (N15) the deadline and overdue decision belong to the block's `sender_node`
> (`messages.deadline_node`), not to the authoring node: the sender's node records the obligation under the
> request id when it sends the request and links it to the message when that message applies there; its clock
> anchors the deadline (`requested_at`). The recipient's home node authors that one message for the remote sender
> seat as signed (N15); no other fact is authored on a node for a remote seat.

**N15**:
- Title: "…; a cross-node handoff is the sender's facts, or one signed request."
- New bullet after *Remote launch*:
> - *Remote handoff* (`handoff --pane P@M`, `--new-tab --space S@M`, `--new-terminal --space @M`, cap
>   `peer.handoff.v1`): the request may also carry a profile directory on this node, the sender's summary and the
>   task message signed by the sender's node (a sender block). This node checks the block against the link's node
>   and the sender seat's replicated home before anything else; any failed check refuses and writes nothing. It
>   then allocates the seat, writes the pair's `thread.direct`, the recipient's join and the signed
>   `message.sent` in one transaction, and only then launches (Herdr) or opens the gate on that staged Send (hc),
>   so a launched agent always has its task. The request still cannot ACK, invite, settle, retire or act as the
>   sender seat in any other way.
- The sentence "A request names only a launch; it cannot send, ACK, …" becomes "…; besides a handoff's three facts
  above it cannot send, ACK, …".
- The peer-without-cap bullet gains: "A handoff to a peer without `peer.handoff.v1` is refused with an upgrade
  error; there is no two-step fallback."

**Needs coordinator approval** (TRUST-POLICY wording beyond N4/N5/N15):
1. Provenance table (`TRUST-POLICY.md:547`): `peer_request:<node_uuid>` says "Never on … memberships or messages".
   Proposed new value `peer_handoff:<node_uuid>`. It covers the target's audit of the three facts (the
   `peer_requests` row's `handoff_*` columns and the `daemon.log` line). It is never on bindings or receipts, and
   never replicated.
2. Accepted limit "A harness launched for a peer on a herdr-core can start before its task exists"
   (`TRUST-POLICY.md:2743-2747`): it now applies only to a bare remote `launch`. A handoff waits for its staged
   Send.
3. Accepted limit "A cross-node handoff checks eligibility once…" (`TRUST-POLICY.md:2760-2768`): the
   "`Launched` … must hold as that node's seat … a seat that never replicates leaves the handoff refused after the
   launch" clause no longer applies to remote launch handoffs.
4. H3 launch-gate paragraph (`TRUST-POLICY.md:3056`): "because the request carries no task and the sender's home
   node authors it after `Launched{seat}`" changes to say a handoff row releases on its staged Send.
5. Withdrawn by the Q4 answer: an unanswered request's overdue is the seat-addressed alert, so there is no
   `peer list`-only accepted limit.
6. New accepted limit: a 2.x node holds a handoff message `failed` until replayed after upgrade.
7. New accepted limit (redesign cycle 1, needs coordinator approval): the sender's node re-sends an unlinked
   handoff request for at most 6 days (`OBLIGATION_TTL_MS`, strictly shorter than the target's 7-day duplicate
   guard). After that the request is `expired`: no more re-sends, a `daemon.log` line and a `peer list` line; a
   target that was offline the whole time never launches it. A `message.sent` that arrives later still applies.

### 9. Errors and edge cases

| Case | Outcome |
|---|---|
| Bad signature, tampered body, wrong `sender_node` | target `Refused` (55-byte detail), nothing written. A forged fact elsewhere is `failed` on every node |
| Sender seat not yet replicated on target | `Unavailable` (not recorded), the requester asks again |
| `profile_dir` missing or not a dir there | `Refused` (60-byte detail) |
| `--pane P@M` with `--profile-dir` | refused by the CLI before sending (90-byte guarded-pane text); a pane plan carrying one is `Refused` by the target (49-byte detail), nothing written |
| Held pane, unknown pane, missing cwd, unregistered harness | as today, `Refused`, nothing written |
| Sender CLI killed after sending | target proceeds; the daemon keeps re-sending the frozen `request_bytes`; the message replicates and links; the agent has its task |
| CLI poll ends (120 s) with no answer | `handoff to <label> still pending: request <req8> stays queued until it starts or expires (6 days)`; the daemon keeps re-sending |
| Target offline for 6 days (`OBLIGATION_TTL_MS`) | the row becomes `expired` (after its due step, so a deadline, which is at most 6 days, has already raised its overdue): re-sends stop, one `daemon.log` line, `peer list` shows `expired unconfirmed` (or `expired unlinked: seat <seat>` for a `launched` row); an unshown seat-addressed alert stays pending; a rerun of the command reports it from the ledger and mints a new id next time |
| Target fails before staging (worker or child failure, Herdr preflight, `settle_at_start` with no handoff columns) after the sender CLI is gone | the re-send fetches the recorded `Failed` without `staged`; the row becomes `failed` with the answer: one `daemon.log` line, `peer list` shows `failed: <detail>`, a rerun prints the stored answer and settles the request file; with `--deadline`, the seat-addressed alert is raised at the deadline. Nothing was authored anywhere |
| Handoff arrives before the sender node's genesis replicated on the target (e.g. right after pairing) | `Unavailable` (39-byte reason, not recorded), the requester asks again; never a recorded `Refused` |
| Remote `--deadline` over 6 days | refused by the CLI before sending (61 bytes); nothing recorded |
| Rerun of a command whose request file is older than 6 days | `resume_only`: a terminal row is answered from the ledger; with the row pruned, the CLI settles the file and starts a new handoff under a new id; the old id is never signed again |
| `Launched` settles after the message already linked the row | guarded no-op: the row stays `linked` |
| Sender daemon down after sending | target proceeds; on restart, link-up re-asks; the obligation links when the message arrives |
| Target crashes after staging, before launch | row `failed: interrupted` (`settle_at_start`). The facts stand (the task is delivered to a seat that has no agent). The recorded answer is `Failed { staged: Some{thread, message} }`, because `staged` is derived from the row's `handoff_thread`/`handoff_message` (Decisions, "Failed after staging"), so the sender keeps its obligation row, links it when the message replicates and still fires overdue, and the CLI prints `; message <m> stands in thread <t>`. A rerun of the same command answers the same recorded `failed`; a new command launches anew (Q5) |
| Target offline before the deadline, message not replicated | the sender's node fires overdue (section 2.3): the seat-addressed alert while the seat is unknown, the in-thread warning once it is known; the row stays in the re-send set until it links or expires |
| Recipient ACKs, receipt replicates | the receipt condition clears, including a pre-link `overdue:handoff` warning on the same condition |
| Existing pair thread (the pane's seat already paired with the sender) | `thread.direct` is skipped and the message is added to the existing pair |
| Body over 65 536 bytes or plan over `MAX_PLAN_BYTES` | refused by the initiator CLI before anything is sent; `malformed` and `plan_refusal` back it up on the target |
| Retry with the same intent after `Refused` | new id (request file settled), as today |

## Acceptance criteria

- [ ] `handoff --new-tab L --space S@M --cwd C --kind K --summary S --profile-dir D -- task` on a Herdr target
      creates the tab with `D`'s env and starts the agent with prompt `S` plus the from line, with `D` projected by
      the adapter, and the task `message.sent` is in the target's store before the launch child submits.
- [ ] `--pane P@M` with `--profile-dir` is refused before sending; a pane plan carrying one is refused by the
      target; nothing is written.
- [ ] The same over `--new-terminal --space @M` (hc): the gate stays closed until the staged Send exists, and the
      harness argv carries the prompt and the config root.
- [ ] A refused launch (bad pane, held pane, bad profile dir, bad signature, unregistered harness, peer without the
      cap) authors no fact on any node; it leaves no `handoff_obligations` row when refused before sending, and
      otherwise a terminal `failed` row that is never re-sent and is pruned after 7 days.
- [ ] With the sender's CLI killed (and, separately, its daemon stopped) right after the request, the target still
      writes the message and launches; the message later reaches the sender's node and links the obligation.
- [ ] With `--deadline` and the target offline before the message replicates, the sender's node records the
      overdue (in-thread `overdue:handoff:<request>` when the seat is known, the seat-addressed alert otherwise);
      the target never records one. The overdue row keeps being re-sent, and a later `Launched` or link moves it
      forward.
- [ ] Every send of one request (CLI poll, daemon re-send, after a daemon restart, after the author's binding role
      changed) transmits the row's stored `request_bytes` byte for byte.
- [ ] Ledger transitions are guarded: a `Launched` that settles after the link leaves the row `linked`; `linked`,
      `expired` and `failed` rows never change; a `Failed` with `staged` keeps the row.
- [ ] An unlinked row, with or without `--deadline`, expires at `expires_at` (6 days, below the target's 7-day
      retention, asserted at compile time); re-sends stop and `peer list` shows it (`expired unconfirmed`, or
      `expired unlinked: seat <seat>` for a `launched` row).
- [ ] A remote `--deadline` over 6 days is refused by the CLI before sending; with any accepted deadline and the
      target offline throughout, the seat-addressed alert is raised before or at expiry (also when deadline and
      expiry fall in one pass), and an unshown alert stays pending after the row expires.
- [ ] A handoff that reaches the target before the sender node's genesis replicated there gets the unrecorded
      `Unavailable` (39-byte reason) and launches once the key arrives; the sender's row stays `sent`.
- [ ] With the sender's CLI gone, a target failure before staging leaves the sender's row `failed` with the
      answer: one `daemon.log` line, `peer list` shows it, a rerun prints the stored answer, and with `--deadline`
      the seat-addressed alert fires at the deadline. A failure the CLI printed raises no alert.
- [ ] A poll on a `linked`, `expired` or `failed` row is answered from the ledger with no frame sent; a request
      file older than 6 days is sent `resume_only`, and with its row pruned the old id is never signed again.
- [ ] A target crash after staging answers `Failed` with `staged` (from the row's handoff columns, also after
      `settle_at_start`), and the sender keeps and later links its obligation.
- [ ] A handoff `message.sent` with `relays_user = true` or a `user_intent` is `failed` on every node.
- [ ] A `message.sent` with a tampered sender block (body, author, deadline, wrong signer) is `failed` on every
      node that applies it and is listed in `sync unapplied`.
- [ ] A 3.0 sender to a peer withholding `peer.handoff.v1` gets the 94-byte-plus upgrade error, with nothing sent.
- [ ] Bare `launch --pane P@M` and plain local `launch` are unchanged (existing tests pass untouched).
- [ ] Local handoffs created after the upgrade use prompt v2; retained v1 records relaunch byte-identically.
- [ ] Every new model-visible string carries its byte size in the commit message (rule f): the prompt v2
      constant ` — from: ` (11 bytes; overheads 23/26 bytes for a 12-byte id, at most 290 bytes in all), the
      94-byte upgrade error, the 61-byte `--deadline` refusal, the 84-byte still-pending line, the 32-byte
      `expired unconfirmed` and 36-byte `expired unlinked: seat` lines, the 21-byte `failed:` `peer list` line, the
      31-byte ledger `linked` result line, the three target refusal details (55/44/60 bytes), the
      35- and 39-byte unavailable reasons, the 132- and 78-byte parse refusals, the 43-byte daemon-predates line, the 38-byte
      result line, and the 49-byte overdue payload delta.
- [ ] `scripts/check-wire-compat` passes: new fields `serde(default)`, `// cap: PEER_HANDOFF_CAP` on the gated
      plan fields, and the baseline grows only by `SenderBlock`.
- [ ] `scripts/check-harness-neutral` passes: profile projection and startup input go through `LaunchPolicy`; no
      harness id appears in core.
- [ ] TRUST-POLICY N4, N5 and N15 are amended in the same commit as the behaviour.

## Testing

Unit and in-process:
- `src/sync/handoff_block.rs`: signing-bytes golden vector, a round trip, rejection of every tampered field,
  cross-domain non-acceptance (an entry hash signed by the same key does not verify as a block).
- `tests/sync/facts_message.rs` (in `tests/combined.rs`):
  - apply of a handoff `message.sent` from the recipient's origin passes;
  - each mismatch (signer not home, payload differs from the block, two recipients, wrong thread, bad signature)
    is `Rejected`;
  - **older-node simulation**: apply with a registry whose `message.sent` v1 handler is the pre-3.0 one
    (`FactRegistry::builtins_plus` with a test kind replacing the handler, `src/sync/facts/mod.rs` test support) is
    `failed`, and `sync unapplied replay` under the shipped registry applies it.
- `tests/store/materialize.rs`: the `HandoffRecipientHome` rule; `deadline_node` set from the block;
  `obligation_authored_here` true only on the sender node.
- `src/service/peer_launcher.rs` tests: `decide` refusals in order, and that each leaves `peer_requests` with
  `refused` and no journal entry; a block from a node with no `sync_logs` row answers the unrecorded 39-byte
  `Unavailable` and leaves no `peer_requests` row; `handoff_prompt` is set at row insert, before any staging. `gate_view` stays closed until `handoff_message` exists.
- `src/cli/handoff.rs` tests: `render_v2` golden bytes with and without a name (one line, ` — from: `), the
  prompt passing `validate_argv`, `retained_prompts` unchanged for v1 records.
- `src/store/handoff_obligations.rs` tests: the transition table (each guarded event from each state, the no-op
  cases included, `failed` storing the answer and setting `alert_shown_at` only for a CLI-returned answer),
  first-send insert race (the losing insert sends the winner's `request_bytes`), expiry at `expires_at` with and
  without `deadline_at` (a row with `deadline_at == expires_at` sets `overdue_seq` in its expiry transaction), the
  alert predicate staying pending across expiry and `failed` and clearing on prune, the due scan raising an unseen
  `failed` row's alert at its deadline, the `OBLIGATION_TTL_MS < RETENTION_MS` assertion, the due scan
  setting `overdue_seq` once and `thread_warned_at` once a seat is known, pruning of `linked`/`expired` rows.
- `src/sync/handoff_block.rs`: `verify` rejects a block with a `None` identity field and an `ack_after_ms` over
  `MAX_ACK_AFTER_MS`; `deadline_at()` reports overflow; the restates predicate rejects `relays_user = true` and a
  `user_intent`.
- `src/store/peer_requests.rs` tests: the recorded answer of a `failed` row with handoff columns (worker path and
  `settle_at_start`) carries `staged`; without them it does not.
- CLI: the plan-size pre-check uses final-size placeholders (a quote-heavy body near the limit is refused by the
  CLI, never by `plan_refusal`); the message placeholder is pinned at 12 bytes, the length of a real
  `generate_public_id` message id, and a plan exactly at `MAX_PLAN_BYTES` with a real signed block passes both
  checks while one byte more fails both.
- CLI: a remote `--deadline` above `OBLIGATION_TTL_MS` is refused with the 61-byte line, one at exactly
  `OBLIGATION_TTL_MS` is accepted; a request file older than `OBLIGATION_TTL_MS` is sent with `resume_only`, and
  a terminal ledger row is answered from the ledger (linked, expired, failed) without a send.
- Parse tests in `src/cli/commands.rs` for the new refusals and the plan contents.

Multi-node, real daemons (`tests/multinode.rs`, harness `herdr_threads::test_support::multinode::{Cluster,
ClusterOptions}`, `src/test_support/multinode.rs`):
- Extend `tests/multinode/remote_addresses.rs`. Its `LAUNCH_STUB` (lines 283-298, through
  `HT_TEST_PEER_LAUNCH_STUB` and `HT_TEST_PEER_LAUNCH_LOG`) answers `peer-handoff-launch` like `launch`, and
  records argv, so the test asserts:
  - the prompt and profile dir reach the child;
  - `messages` on node 1 holds the task before the stub call;
  - a refused profile dir leaves `kinds(1)` unchanged.
- Kill case: the stub's `hold` file plus killing the CLI process (`cluster.text_as_env` in a scoped thread), and
  separately `cluster.stop_node(0)` / `start_node(0)`. Assert the message reaches node 0 and
  `handoff_obligations.state='linked'`.
- Overdue offline: `--deadline` of a few seconds, `HT_TEST_SYNC_HOLD_MS` short, and `cluster.partition(0, 1)` (or
  `ClusterOptions.gated` + `hold_frames_from(1)`) after `Launched`. Assert one `overdue:handoff:` warn on node 0
  and none on node 1.
- Tampered block on third nodes: `cluster.author_kind::<MessageSent>(1, &tampered)`
  (`src/test_support/multinode.rs:1458`, as `tests/multinode/config_mixed_versions.rs` does) in a 3-node cluster.
  Assert `failed` on nodes 0 and 2 (`sync unapplied`).
- Missing cap: `ClusterOptions.env` with `HT_TEST_WITHHOLD_CAPS=peer.handoff.v1` on the target (the pattern at
  `remote_addresses.rs:343-347`). Assert the upgrade error, no fact, and one `failed` obligation row holding the NoCap answer.
- hc: extend `tests/multinode/peer_launch_terminal.rs` (`test_support::fake_hc::FakeHc`). Assert the gate holds
  until staging, that the gate argv carries prompt and `--config-root`, and that a refusal creates no terminal.

Gates: `nice cargo clippy --locked --all-targets --all-features -- -D warnings`, `scripts/check-harness-neutral`,
`scripts/check-wire-compat`, `nice cargo test --locked --all-features <filters>`. Before merge:
`nice scripts/check-default-features`, the full suite, and `scripts/check-no-leaked-processes --run-id`.

## Unchanged behavior

Redesign cycle 1 (the sender obligation ledger, plus the local fixes) must not change:
- The target side: `decide`, its refusal order and texts (round 2 adds only the unrecorded `Unavailable` for a
  sender key not held yet, before the signature check, and states that the prompt freezes in `decide`, as
  decomposition already decided), staging as one transaction, the launch order, the hc staged-Send gate, and the
  duplicate-id rule (a repeat id gets only the recorded answer, never a second launch).
- The signing bytes layout (`HTHOFF01`), the signed fields and `verify_strict`; the block stays inside
  `LaunchPlan.message` and `MessageSentPayload.handoff`, both `serde(default)`.
- The authority rule's inputs: only the fact and replicated identity, never `peer_requests` or the ledger.
- Q4's seat-addressed alert and its delivery paths: digest count and token, wake probe 9 (p0, wakes at every T),
  `mod_binding_view` attention block, the `HandoffOverdue` inbox row (68 or 54 bytes plus tag, and seat when launched) behind
  `inbox.seat_alerts_v1`, and `AdvanceCursors.alerts` clearing from top-level non-readonly bindings only.
- The in-thread `overdue:handoff:<request>` warning, its receipt condition, its 49-byte payload delta, and one
  in-thread warning per handoff while its receipt condition is open.
- The deadline owner (`deadline_node`, N5) and the post-link ordinary receipt path.
- The sender's re-send cadence (each link-up and at most every 30 s), the CLI's 120 s poll window, the request
  file's location, lock and atomic write, the `request_key` hash (tag `remote launch v4` for a handoff plan, `v3` for a plain launch), and the `Launched`
  result line. The request file's settle triggers do change: besides `Launched` and a recorded refusal, the
  ledger answers of §2.2 (linked, expired, failed) settle it, and a file older than `OBLIGATION_TTL_MS` is sent
  `resume_only`.
- Q5: a target crash after staging is never resumed on the target.
- Local handoffs, bare `launch --pane P@M`, retained v1 prompts, and 2.x compatibility as in §7.

## Out of scope

- Handoff into an existing remote seat (`--existing --seat X@M`): stays sender-authored (N15 bullet 2).
- `--thread`/`--new-thread` channels for remote handoffs (refused, Q2), invitations on remote handoffs, and
  `--harness-binary`/`--name` remotely.
- Automatic replay of `failed` facts after an upgrade (stays the operator's `sync unapplied replay`).
- Wake-e2e changes: the remote path runs between nodes, and the release matrix is unchanged.

## Open questions (for the coordinator)

- **Q1 (blocking): line break vs Herdr.** The agreed prompt puts `from:` on a separate line, but every native argv
  refuses `\n` (`src/ports.rs:2147-2151`), because Herdr types the command into the pane's shell. Recommendation:
  the renderer's separator is `\n` for exec hosts (hc) and ` — ` for shell-line hosts (Herdr), chosen by
  `HostKind`. The alternative is one ` — ` everywhere, a single form.
- **Q2: thread choice.** "Pair thread" implies a remote handoff can no longer use `--thread T` or `--new-thread`,
  which today's remote path supports (`src/cli/commands.rs:3914-3936`). Recommendation: refuse them (section 2.1).
  The sender can invite the new seat afterwards.
- **Q3: "the initiator works it out" for `profile_dir`.** Recommendation: only an explicit absolute `--profile-dir`
  is sent. The initiator does not substitute its own caller config root, because that path is local. The
  alternative: send the caller's root when the user passes `--profile-dir-from-caller`, or always, and let the
  target refuse it.
- **Q4: overdue before the seat is known.** Warnings need a thread (`messages.thread_id NOT NULL`,
  `warning_conditions.thread_id NOT NULL`). Recommendation: an in-thread warning once the seat is known (the
  sender's node opens the pair itself, N4), otherwise `peer list` and `daemon.log` only, as a new accepted limit.
- **Q5: crash between staging and launch.** The facts exist and the agent never starts. A message to a seat with no
  agent is a normal state (it waits), and the sender's overdue fires. Recommendation: accept it and document it
  under N15. The alternative is a target-side relaunch, which conflicts with "never resumed".
- **Q6: prompt without a summary.** Provisional decision: a remote handoff requires `--summary` (§2.1); the `Handoff` line is for local v2 prompts only. The agreed format starts with `<summary>`. Recommendation: the fixed line
  `Handoff` (7 bytes). Alternatives: require `--summary` for remote handoffs, or send the from line alone.
- **Q7: dropping the inbox fallback.** v2 drops v1's `if not shown, run herdr-threads inbox` hint. A hookless
  harness (managed-launch Codex, `HarnessAdapter::binding_turn_delivery`) relies on the p0 wake and the pending
  hint line instead. That is acceptable under the delivery contract, but noted.
- **Q8: sender block contents.** The block includes the sender-minted message id and `author_role`, beyond the
  listed body, seat, priority, ACK policy and request id. Both are needed: the id for linking, the role because
  `message.sent` carries it as a send-time claim.

## Coordinator answers (2026-11-04, relayed from the user, thread d5pFwv0Z4Ps #6)

These override the provisional answers wherever they differ.

- **Q1:** one line on every host, no newline variant: `<summary> — from: <name> (<id>)`, or
  `<summary> — from: <id>` when the seat has no name. One renderer constant ` — from: ` (11 bytes: space, U+2014
  (3 bytes), space, `from: `). Overhead for a 12-byte id: 23 bytes without a name; 26 bytes plus the name with one
  (` (` + `)`). Largest prompt: 200 + 26 + 64 = 290 bytes. No `HostKind` separator method.
- **Q2:** `--thread`, `--new-thread` and `--goal` (and `--thread-name`, `--topic`) are refused for a remote handoff.
- **Q3:** only an explicit absolute `--profile-dir` is sent.
- **Q4:** when the target never answers, past the deadline the sender's node raises an overdue warning to the
  sender's own seat naming the machine and request: a seat-addressed warning, no thread (see "Seat-addressed overdue
  (Q4)"). Once the seat is known it goes to the pair thread as usual. This replaces the `peer list`/`daemon.log`-only
  path; no accepted limit for it.
- **Q5:** accepted limit: a target crash after the message transaction leaves the task with no agent, never resumed.
- **Q6:** `--summary` is required when a remote handoff carries a message.
- **TRUST-POLICY:** approved, beyond N4/N5/N15: the provenance value `peer_handoff:<node_uuid>`; rewording the
  accepted limits (~2743-2747, ~2760-2768) and the H3 launch-gate paragraph (~3056) that assume the task arrives
  after launch; the new accepted limits "a 2.x node holds the handoff message `failed` until replayed" and "a target
  crash after the message transaction leaves the task with no agent, never resumed"; the N5 exception (the target
  authors one message for a remote sender seat).

### Seat-addressed overdue (Q4)

When the target never answers (seat unknown), the overdue goes to the author's own seat with no thread. Once the
seat is known, the in-thread path of §2.3 takes over. There is no existing thread-less warning; the closest precedent
is the seat-level `InboxBatchV2Item::FirstContact` (`src/protocol/results.rs:1486`, `src/store/queries/inbox_v2/mod.rs:243-270`).

**Data model.** No new table. `handoff_obligations` (0015, §2.3) carries `overdue_seq INTEGER` (the node decision
that made the row overdue) and `alert_shown_at INTEGER` (NULL = pending), with the partial index
`handoff_obligations_alert(author, overdue_seq) WHERE overdue_seq IS NOT NULL AND thread_warned_at IS NULL
AND alert_shown_at IS NULL AND state!='linked'`. The predicate reads `state` only to exclude `linked` (after
linking the ordinary receipt overdue takes over); it reads no seat, so a `launched` row whose pair thread this
node does not hold keeps its alert, and an unshown alert survives the row's expiry or failure (Clearing). The pair
thread itself is not testable in SQL (its id is a hash), and `thread_warned_at` is exactly "the in-thread warning was
written". Overdue is these columns, never a ledger state. The pending alert is exactly that
predicate joined to a non-retired local author seat. Node-local, never journaled.
Its publication key is `(overdue_seq, 0)`, a real `local_node` decision, so it orders with every other warning key.

**Write path.** The §2.3 due step, for a row with `deadline_at <= now` and `overdue_seq IS NULL`
in `sent` or `launched` (a seat known without the pair thread held is the same alert), in an unseen `failed` (§2.3 "Failure learned without the CLI"), or in `sent` at its expiry transition,
runs one transaction: `overdue_seq = schema::next_decision_seq`,
`DeliveryTail::attention_unless_retired(author, REASON_NEW_ATTENTION)` (`src/store/delivery_tail.rs:94`), and the
`daemon.log` line. The due step changes no state, so a `sent` row stays in the re-send set. Every later exit from the
predicate (shown, the in-thread warning written, linking, a pruned row, retirement) does the same `wake_work` bump
for the author, so fingerprints move; a write that leaves the predicate unchanged (a late `Launched{seat}` alone)
bumps nothing. The ledger's guarded transitions are the only writers of those exits.

**Delivery paths touched.**
- Counts: `attention::digest_for` (`src/store/attention.rs:422-500`) adds the alert count to `warnings.count`
  (`has_more = true`, alerts are not listed: an `AttentionRef` needs a thread); `token.warning` and
  `frontier.actionable_warning` take the max with the alert key. Same for `queries::pending_warning_count`
  (`src/store/queries.rs:5256`). The hookless pending hint follows from the digest.
- Wake (p0, delivery contract rule 5): a ninth `WAKE_PROBES` entry on `handoff_obligations_alert`
  (`attention.rs:1521`); `wake_seat_attention` (`attention.rs:1740`) merges the alert key into `latest_warning_seq`
  and `frontier.actionable_warning`. It wakes at every T; the closed-loop wake retypes only on frontier advance.
- Hook and channel: `seats::mod_binding_view` (`src/store/seats/cooperative.rs:1518`) adds alerts to
  `other_pending`, so the hook pull and mod presentation emit the existing attention block. No new hook, channel,
  `frame.rs` or `register.js` text.
- Text inbox: new `InboxBatchV2Item::HandoffOverdue { request, node_tag, seat }` (`seat` `#[serde(default)]`, set for a launched row), carried only on a first page, ahead of
  the merge, and only when `InboxQuery.seat_alerts` is set; the CLI sets it only if the daemon advertises
  `inbox.seat_alerts_v1` (`InboxQuery` is `deny_unknown_fields`). Rendered in `output_compact.rs:76`;
  `assemble.rs:101` maps it to `Assembled::Attention`; `hook_inbox.rs:156` treats it as whole.

**Framing (model-visible, inbox only).** One row, no `read:` line:
`warning overdue_handoff <req8> to <tag>: not confirmed by that machine\n` (68 bytes plus the node tag) for a row
the target has not confirmed, and `warning overdue_handoff <req8> to <tag>: unlinked: seat <seat>\n` (54 bytes plus
the tag and seat) for a row launched there but not linked yet, matching `peer list`'s `overdue unlinked`.

**Clearing.** (a) Shown: `AdvanceCursors` gains `#[serde(default)] alerts: Vec<String>` (request ids, sent only under
the capability); the daemon sets `alert_shown_at` for rows authored by the claim's seat, only from a top-level,
non-readonly binding (a subagent's display, `--machine` or `--json` settle nothing). (b) The in-thread warning is
written (seat known and the pair thread held here): a late `Launched{seat}`, which the continuing re-sends fetch,
moves the row `sent → launched` with its seat (the guarded transition of §2.3), and the next due scan records the
in-thread `overdue:handoff:<request>` (`thread_warned_at`) once the pair thread is held; an unshown alert then just
disappears. A late `Launched` alone does not clear it: a background re-send settles without waiting for replication,
so the seat can be known while the pair thread is not, and the target may be offline. The "one warning per handoff"
dedupe keys only on the in-thread event. (c) Linked (which sets the seat) or pruned. Expiry and a `failed` answer do
not clear an unshown alert: the predicate reads `state` only for `linked`, so the alert stays pending until it is
shown, the in-thread warning is written, the row is pruned (7 days after it ended) or (d) the author retires.
`peer list` keeps showing `overdue` while the row is `sent` with `overdue_seq` set, `overdue unlinked: seat <seat>`
(36 bytes plus the id, tag and seat) while it is `launched`, overdue and not yet warned in-thread, `expired
unconfirmed` once it expires, and `failed: <detail>` once it fails.

**Accepted consequences.** Hooks and channels deliver only the generic attention block, so the model reads the alert
with one `inbox` call, as for every actionable warning today. An old CLI against a new daemon gets the count and the
wake but cannot show or clear the alert; it clears on link, launch or upgrade.

**Tests.** Digest count, `has_more`, token key; `seat_has_pending_rows` through probe 9;
`tests/store/wake_discovery_cost.rs` for the new index; the inbox item on a flagged first page only; byte-exact row;
a launched-overdue row renders the `unlinked: seat` line; `AdvanceCursors` clears from top-level only; late `Launched` (fetched by a re-send after the row went overdue)
swaps to exactly one in-thread warning; linking or the in-thread warning removes the alert (a `Launched` for a seat
whose pair thread is not held keeps it, and `peer list` shows the `launched` row), while expiry and a re-send-fetched `Refused` keep an unshown alert pending until shown or pruned; a `Refused` returned to a polling CLI marks it shown; a row whose deadline and expiry fall in one pass still raises its alert; an idle seat at T = 0, -1, 3 wakes once and is not retyped after shown; an old CLI gets
count and attention only.

## Decisions added during decomposition (coverage rounds 1-2)

These refine sections 2-6; where they differ, they win.

- **Failed after staging.** `LaunchState::Failed` gains one `#[serde(default)]` field `staged: Option<{thread, message}>`, set when the
  target failed after its staging transaction. It is never stored as its own flag: the one function that builds
  the recorded answer of a `failed` `peer_requests` row derives `staged = Some{thread, message}` exactly when the
  row's `handoff_thread` and `handoff_message` are set (staging sets both in its one transaction, so they are set
  together or not at all). That function serves the worker's failure, `settle_at_start`'s `interrupted` rows and
  every repeat-id answer alike, so a target crash after staging answers `Failed` with `staged` too, whatever
  `pane` holds and whatever `unconfirmed` says. The sender keeps its obligation row for such an answer (no state
  change, §2.3), so it links when the message replicates and overdue still fires. The CLI prints the failure plus
  `; message <m> stands in thread <t>` (31 bytes plus ids). Without `staged`, Failed moves the row to the terminal
  `failed` with its answer, as in §2.3.
- **Duplicate request ids.** An existing `peer_requests` row answers its recorded outcome; the target never
  re-decides, re-validates or re-launches. `Unavailable` is unrecorded, so a retry decides afresh; the sender keeps
  the row `sent` on Unavailable.
- **Overdue across linking.** A pre-link `overdue:handoff:<request>` warning uses the receipt condition, so the
  recipient's ACK clears it; once linked, the ordinary receipt overdue for that message is not recorded again while
  the pre-link warning's receipt condition is still open (it stands for the linked message); a cleared one no longer
  stands for it, and the linked message gets its own overdue. The seat-addressed alert of Q4 is separate.
- **Existing pair.** Staging resolves the pair for (author, seat) through the canonical direct-pair lookup and reuses
  an existing pair whatever its id; `thread.direct` is authored only when none exists. Crossed creates are
  idempotent on the id (§2.3).
- **Profile directory and startup input.** `decide` validates the profile directory through
  `LaunchPolicy::explicit_directory_env` and the startup input through the existing adapter API; the Herdr child and
  the hc gate derive the same projection from the stored plan through the same calls (deterministic).
- **Local capabilities.** `HANDOFF_ATOMIC_REMOTE_V1` (advertised with the sender requester),
  `PEER_HANDOFF_STAGE_V1` (with staging; the Herdr child checks it and fails with no fact without it) and
  `HANDOFF_SENDER_PROMPT_V2` (with prompt v2; the local CLI falls back to the v1 render against an older daemon).
- **Ledger answers.** A poll answered from the sender's ledger (§2.2: a `linked`, `expired` or `failed` row, or a
  `resume_only` id with no row) is carried by one new local-only variant,
  `RemoteLaunchAnswer::HandoffLedger { outcome }` in `src/protocol/sync_admin.rs`, with
  `outcome: Linked { seat, thread, message } | Expired { seat: Option<String> } | Unknown` (each field
  `serde(default)`); a `failed` row answers with its stored `RemoteLaunchAnswer` verbatim, so no variant is needed
  for it. It is the local protocol under `HANDOFF_ATOMIC_REMOTE_V1` (only a CLI that sends a handoff draft gets
  it), never a `LaunchState` variant and never on the peer wire, so `scripts/check-wire-compat` is unaffected.
  The requester owns building it (PeerHandoffRequester); the CLI owns printing it (§2.2 lines) and settling the
  request file.
- **Remote `--summary`.** Required for a remote handoff (Q6, provisional): `--summary is required for a handoff to
  another machine` (55 bytes).

## Key abstractions

**SenderBlock** — new struct (module `src/sync/handoff_block.rs`, core), the sender node's signed statement of one handoff message.
- Owns: the canonical `HTHOFF01` signing bytes, signing (given a `NodeIdentity`), `verify` (presence of `sender_node`/`author`/`message`, signature, and the v/priority/ACK invariants with `ack_after_ms <= MAX_ACK_AFTER_MS`), `deadline_at()` (the one checked `requested_at + ack_after_ms`), and one consistency predicate that says whether a `message.sent` payload restates the block: a full-struct comparison against the expected payload (author, role, message id, body, priority, ACK, duration, recipient deadline, pair thread, single Ack recipient, and every other field, `relays_user`/`user_intent` included, at its default).
- Hides: the byte layout, the domain separation and what "restates the block" means.
- Must not: read the store, know about `peer_requests` or obligations, or pick a key. Callers pass in the pubkey.
- Relationships: a plain value type. The wire (`LaunchPlan.message`), the fact (`MessageSentPayload.handoff`), the sender daemon (sign) and the target/materializer (verify plus the predicate) all depend on it; it depends on nothing above `sync`. The target's check before writing (§4.2) and `HandoffRecipientHome` call the same predicate; there must not be two copies.
- Lifecycle: a value, signed once at first send and frozen inside the obligation row's `request_bytes`; it is never rebuilt.
- Errors: a `BlockError` callers do not branch on. The target maps every variant to the one 55-byte refusal, the materializer to `FactError::Rejected`; the variant goes only to the log.

**HandoffObligations** — new module `store::handoff_obligations`, the sender node's ledger of signed requests (table `handoff_obligations`).
- Owns: the row with its frozen `request_bytes` (the signed `LaunchRequest`, written once by the first insert; every send reads it), the minted message id, `requested_at`/`deadline_at`/`expires_at`, the closed state enum `Sent → Launched → Linked` plus terminal `Expired` (from `Sent|Launched` at `expires_at`, after the row's due step) and terminal `Failed` (from `Sent` on NoCap/Refused/plain Failed, storing the answer, and `alert_shown_at` when a polling CLI received it), each transition one guarded `UPDATE … WHERE state IN (…)` that is a no-op when the guard misses, the overdue columns (`overdue_seq`, `thread_warned_at`, `alert_shown_at`; overdue is never a state), `OBLIGATION_TTL_MS` (6 days, compile-time `< peer_requests::RETENTION_MS`), the due query over `Sent|Launched` rows and unseen `Failed` rows, the state-free alert predicate (`overdue_seq` set, `seat` NULL, unshown), the re-send set (`Sent|Launched`, overdue or not), the terminal-row lookup the requester answers polls from, and pruning of `Linked`/`Expired`/`Failed` rows 7 days after `updated_at`.
- Hides: the transition rules, the guards and their SQL.
- Must not: sign, send frames, or write warnings itself (overdue goes through `record_overdue_if_pending`). It is the only writer of the table: the materializer's link step calls into it.
- Relationships: used by `PeerHandoffRequester`, by the materializer (link in the same tx) and by `scheduler::deadlines` (due scan).
- Lifecycle: durable, node-local, never replicated; rows outlive the CLI and the daemon.
- Errors: store errors only; a repeat insert is not an error.

**PeerHandoffRequester** — new module `service::peer_handoff` (sender daemon), the one place that turns a draft into a signed, recorded, sent request and settles the answer.
- Owns: the claim check (equal to `requester_seat`, local, open top-level binding), filling and signing the block once (only when no row exists), recording the obligation with the signed `request_bytes`, sending the row's `request_bytes` through the existing `launch_remote` path (never a freshly built request), settling answers into the ledger through its guarded transitions, opening the pair with `resolve_direct_pair` after `Launched` + `wait_replicated`, the re-send loop over the ledger's re-send set (on link-up to `target_node` and at most every 30 s), running the expiry transition before each pass and at start, the bug-guard refusal of a draft whose `ack_after_ms` exceeds `OBLIGATION_TTL_MS`, and answering a CLI poll for a terminal row from the ledger without a send (`RemoteLaunchAnswer::HandoffLedger` for `linked`/`expired`, the stored answer for `failed`; `HandoffLedger { outcome: Unknown }` for a `resume_only` id with no row, which it never signs).
- Hides: that the CLI's first send and the daemon's later re-sends are the same operation.
- Must not: keep a second copy of sign/settle in `SyncAdmin::remote_launch` (it delegates when the plan carries a draft); fall back to the two-step path.
- Relationships: composes `HandoffObligations`, `SenderBlock`, `NodeIdentity` and the coordinator's `launch_remote`; called by `SyncAdmin::remote_launch` and one daemon task.
- Lifecycle: stateless functions plus one daemon-owned task started and stopped with the sync runtime; all state is in the ledger.
- Errors: `CallerUnverified` (claim); `RemoteLaunchAnswer` variants pass through. The CLI tells apart only NoCap for `PEER_HANDOFF_CAP` (upgrade text) and the missing local capability.

**PeerHandoffStage** — new module `store::peer_handoff`, the target's single staging transaction (§3.2 step 2).
- Owns: re-checking the row is `running` and the seat is still the target's, calling the `SenderBlock` predicate, authoring `thread.direct` (if absent), the recipient's join and `message.sent` (with `deadline_node`), setting `peer_requests.handoff_thread/handoff_message`, the hc `record_gate_launch` in the same commit, and idempotence on the request id. It does not render or write `handoff_prompt`: `decide` froze it at row insert (§3.1).
- Hides: the three-fact atomicity and that the gate opens in the same commit.
- Must not: author anything else (no invitation, no receipt, nothing else attributed to the sender), or read a caller claim; the `peer_requests` row is the authority.
- Relationships: called from two entries only: `Command::PeerHandoffStage` (the Herdr child via `StagingAuthor::PeerRequest`) and the hc worker after `allocate_gate_seat`.
- Lifecycle: one call per request, under the domain writer.
- Errors: row not running / seat moved (the child records `failed`, no fact); store errors.

**StagingAuthor** — new enum in `cli::handoff`, choosing who authors staged work: `Caller { call }` or `PeerRequest { request }`.
- Owns: the choice of author source; matched once at `stage_work` entry.
- Must not: be threaded through the create/invite/send phases.

**AuthorRule** — extended: `HandoffRecipientHome { block, recipient }`. `require_author` checks origin = recipient's home, author's replicated home = `block.sender_node`, and `block.verify(key)` for a key of `node_keys(tx, sender_node)`. Payload equality comes from the `SenderBlock` predicate, called in `MessageSent::apply`, not re-implemented in `materialize.rs`.

**LaunchPlan** — extended: `profile_dir`, `handoff_prompt`, `message` (each `// cap: PEER_HANDOFF_CAP`), plus two queries every branch point uses: `handoff()` (the block, or none) and `required_cap()` (launch / terminal / handoff). `on_launch_remote`, the receive-side link check, `malformed`, `decide` and the worker ask the plan; none repeats `message.is_some()` and target matches. Today's terminal-cap `if/else` in `on_launch_remote` folds into `required_cap()`.

**PeerLaunchService / Worker** — extended: `decide` adds a `handoff_refusal` step (mirrors `plan_refusal`): link cap, block identity, sender key held (unrecorded Unavailable when `sync_logs` has no row for `from`), signature, sender seat (Unavailable vs Refused), profile dir via `LaunchPolicy::explicit_directory_env`, startup input. The worker matches `plan.handoff()` in exactly two places: which child runs on Herdr (`peer-handoff-launch` vs `launch`), and what follows gate-seat allocation on hc (`peer_handoff::stage` vs `record_gate_launch`). `gate_argv` takes the frozen prompt and config root.

**peer_requests** — extended: `handoff_thread/handoff_message/handoff_prompt` columns; `gate_view.send_staged` becomes "created and (no handoff message, or its message row exists)".

**ObligationRef** — extended: `Handoff(request)`, matched in `obligation_authored_here` and `record_overdue_inner`. `Receipt`'s authored-here SQL reads `COALESCE(deadline_node, origin_node)`.

**summary_prompt** — extended: `render_v2` over a `SenderLine` (name?, id), one line on every host with the constant ` — from: ` (Q1); no host-dependent separator, no `HostKind` method. The local plan gains `sender_prompt` (behind `HANDOFF_SENDER_PROMPT_V2`). v1/V3 stay byte-frozen.

**MessageSentPayload / materialize_message** — extended: optional `handoff` field, `deadline_node`, the obligation link in the same transaction, and a local author with non-local origin (no sender binding, no preparation).

Variation: the launch-plan kind is closed (launch, terminal, handoff) → `LaunchPlan::handoff()`/`required_cap()`, a closed `match` only in the worker's two branch points. Obligation states → a closed enum in `HandoffObligations`, monotonic, with guarded transitions. The prompt is one form on every host (Q1), so nothing varies by host. Harness differences (profile projection, startup input) stay behind `LaunchPolicy`; core gets no new branches. No new trait: nothing here has an open-ended set of kinds.

Walk-through: `handoff --pane P@M --summary S -- task` → the CLI builds a plan with `handoff_prompt` and an unsigned draft; `run_remote` sends it with its claim → `SyncAdmin::remote_launch` delegates to `PeerHandoffRequester` (claim check, sign once, ledger row `Sent` with frozen `request_bytes`, `launch_remote` of those bytes; the coordinator checks `plan.required_cap()`) → target `decide`: `plan_refusal`, placement, hold, `handoff_refusal`, freeze the prompt (`render_v2`, one line), insert the row, start the worker → the worker runs `peer-handoff-launch`: `preflight` → `stage_work(StagingAuthor::PeerRequest)` → `Command::PeerHandoffStage` → `store::peer_handoff::stage` → `NativeLauncher::launch` → `launched` → the requester sees `Launched{seat}`, waits for replication, moves the ledger row `Sent → Launched` (guarded; a no-op if it already linked), opens the pair → `message.sent` replicates back, `HandoffRecipientHome` passes, the materializer links the ledger row; the deadline follows the ordinary receipt path (`deadline_node` = sender). A likely future change, an atomic handoff into an existing remote seat, touches one plan variant in `required_cap()`/`handoff()` and one arm in the worker's match; `SenderBlock`, the ledger, the requester and the authority rule are unchanged. A new host kind touches nothing here.

Per spec §3.2: `StagingAuthor` is a mode flag on `stage_work`; `PeerRequest` replaces all three phases with one call, so `stage_work` matches on it once at entry — the deliberate exception that keeps `StagedWork` and the `execute_steps` pipeline shared.
Per spec §3.1 vs §3.2 (hc): the prompt is rendered and frozen once in `decide`, at row insert (the sender seat row is already verified there), because hc's `gate_argv` needs it before creation; the Herdr child and `gate_argv` both read `peer_requests.handoff_prompt`; staging never renders it again.
Per Q6 (provisional: require `--summary` remotely): a remote `handoff_prompt` is always `Some`; the `Handoff` fallback line stays only for local v2 prompts without a summary.

Digest:
- SenderBlock: owns signing bytes, sign/verify and the single "payload restates block" predicate; new block fields and invariants go here; callers never compare payload fields to a block themselves or pick the domain magic.
- HandoffObligations: owns the sender's request ledger, its frozen request bytes, its guarded monotonic transitions, the overdue columns and expiry; new states and due rules go here; callers never write `handoff_obligations` SQL directly (the materializer's link calls it).
- PeerHandoffRequester: owns claim check → record → sign → send → settle, plus re-sends; new sender-side request behaviour goes here; `SyncAdmin::remote_launch` and the re-send task never duplicate signing or settlement.
- PeerHandoffStage: owns the target's one-transaction authoring of the three facts and the gate record; new staged artifacts go here (only with a TRUST-POLICY N15 change); callers never author handoff facts piecemeal or with a caller claim.
- StagingAuthor: chooses who authors staged work (caller vs peer request); a new author source is a new variant matched once at `stage_work` entry; phases never branch on it.

## Post-Implementation Notes
- ht-8lem.49: `--profile-dir` is applied on Herdr targets: a remote new tab is created with the frozen profile env (`internal peer-target-create --directory-context`), and a remote pane with `--profile-dir` is refused by the CLI (90 bytes, existing) and by the target (49 bytes).
- ht-8lem.50: the seat-addressed alert stays until the in-thread warning is written (a `launched` row without the pair thread keeps it, and `peer list` shows `overdue unlinked: seat <seat>`); the due pass pages past rows it cannot advance; a row error in due, expiry or re-send is logged and skipped.
