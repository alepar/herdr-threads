# Delivery policy: neutral delivery core, Claude push-channel re-fit, hook-pull requests and `wake_on`

Parent: [root spec](2026-10-09-agy-harness-adapter-design.md) · Bead: ht-poh.3

## Goal

Delivery is an adapter policy instead of a Claude special case:
- `src/delivery/` holds the v0.5.0 delivery mechanics, without naming any harness.
- Claude's mod runs through `DeliveryPolicy` (`PushChannel`). Its wire and behaviour stay byte-identical, and the existing v0.5.0 tests pass unmodified.
- A `HookPull` harness (agy, ht-poh.5) can pull framed pages with `DeliveryPull` and settle them with `DeliveryConfirm`, decided against A2.
- A seat can opt in at runtime to waking on every message with `herdr-threads wake-on all`, which the herdr-threads SKILL explains.

## Problem description

v0.5.0 mod delivery is hard-wired to Claude in nine places:
- `service/dispatch.rs:531`, `:557`, `:634`, `:653`, `:670`;
- `store/receipts.rs:144-150`;
- `cli/watch.rs:986`;
- `cli/commands.rs:1097` (`value_parser = ["claude"]`) and `:2930`.

The delivery mechanics live inside the `watch` CLI child (`cli/watch.rs:522-813`): chunk assembly, the 8 KiB truncation with marker, 32-item/64 KiB pages, id de-duplication and header-label flattening. The D4 framing exists only in the mod's JavaScript (`integrations/claude/mod/hooks/register.js:10-58`).

A pull-based harness needs the same mechanics on the daemon side, plus an offered ledger and a confirm path. The root spec settles the shape (§2) but leaves four things open:
- where the ledger and `wake_on` live;
- whether the ledger survives a restart;
- how "channel live" is decided for fallback;
- how `wake_on: all` reaches a seat whose live mod appends lazy rows without a turn.

## Main challenges

**Byte-identity under a rename.** The existing tests import `WatchRefusalReason::NotClaude` by name (`tests/cli/watch.rs:1017-1026` through `use WatchRefusalReason::*`, and `tests/daemon/watch_connection.rs:234`). The tests also call `cli::watch::{drain, EmitState, record_truncated, hint_path, run_ack_lines, run_session, parent_alive, precheck}` through `use super::*`. So the neutral names have to be added without removing the frozen ones, and the extraction has to leave a facade.

**Pull without a store write.** The pull must stay a one-round-trip read on the 1.5 s hook budget, yet record what was offered. The confirm must settle only what this generation was offered.

**Re-offer storms.** agy's `PreInvocation` fires before every model call. If the transcript never shows an item (unreadable transcript, aborted injection), a naive "offer until confirmed" re-injects up to 64 KiB on every call.

**`wake_on: all` meets mod behaviour.** The Claude mod appends lazy rows without a turn (D5), so `wake_on: all` would silently not wake a live-mod seat unless the mod's input changes. The mod code itself must not change.

**The SKILL file is capped.** `tests/cli/skill.rs:148` requires fewer than 200 lines, and the file is at 197.

## Key decisions made

**Trait and registry.**
- `DeliveryPolicy` lives in `src/harness/adapter.rs` and is reached through `HarnessAdapter::delivery_policy()` (default `None`) and a registry lookup by harness id.
- The registry validates the pairings: `PushChannel` ↔ `cooperative_mod_delivery`, `HookPull` ↔ `cooperative_hook_delivery` + `Native`.

**`src/delivery/` is a move.** Code moves out of `cli/watch.rs` behind a facade that keeps every name the tests use. It adds a Rust port of the JS `frame()`, pinned to the JS by shared golden fixtures.

**Claude checks become policy lookups.**
- Core code uses the neutral constant `NO_PUSH_CHANNEL`, which aliases the frozen wire variant `NotClaude` (`not_claude`). That variant stays only in `protocol/watch.rs` (allowlisted).
- `ack_mod_delivered` becomes `ack_policy_delivered(…, mechanism, provenance)`.

**`DeliveryPull` / `DeliveryConfirm`.**
- New `src/protocol/delivery.rs`, capability `delivery.pull_v1`, `PROTOCOL_VERSION` stays 6 (the `MOD_WATCH` precedent).
- The offered ledger is **process-local**, in the daemon. A pull stays a read with no store write, which matches the `ModChannels` precedent. A restart re-offers (at-least-once).
- A generation mismatch discards the seat's ledger entry lazily.
- Re-offers are bounded: 60 s spacing and at most 3 offers per id per generation. After that the item is parked and stays pending for `inbox`.

**Routing.**
- "Channel live" stays exactly today's predicate: any registry entry in Live, ReconnectGrace or RebindGrace, and not stalled.
- The policy's `idle_wake` is recorded on the channel entry. Native wake is suppressed only for `Channel`.
- Digest omission for `HookPull` applies at tool-boundary check-ins. The lifecycle check-in keeps the digest, because agy's SessionStart context (ht-poh.4) carries non-body attention.

**`wake_on`.**
- Stored as `seats.wake_on`, plus `wake_work.last_lazy_ordinal`, in the next free migration.
- Set with `herdr-threads wake-on [nudge|all]` (an accountable cooperative top-level mutation).
- Read by `store::attention::wake_seat_attention`, so a pending lazy row becomes a wake source with its own frontier.
- For push channels, `InboxBatchV2` marks such lazy rows `wakes: true`, following the `informational` precedent. `watch` then emits them as `message` lines (ack not required), so the unchanged mod submits a turn when idle.

**SKILL.** The `wake_on` guidance is one bold-led bullet, which keeps the file at 198 lines.

## Decision points

### DP1. `DeliveryPolicy` trait and the adapter hook

**Chosen** (in `src/harness/adapter.rs`; per bead ht-poh.3, this file gains only these items):
```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeliveryMechanism { PushChannel, HookPull }
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IdleWake { Native, Channel }
pub trait DeliveryPolicy: Send + Sync + 'static {
    fn mechanism(&self) -> DeliveryMechanism;
    fn provenance(&self) -> &'static str; // an A3 constant
    fn idle_wake(&self) -> IdleWake;
}
// in HarnessAdapter, beside composer_policy (adapter.rs:177):
fn delivery_policy(&self) -> Option<&dyn DeliveryPolicy> { None } // None: hooks + Native wake
```

Registry changes (`src/harness/registry.rs`):
- `ErasedAdapter` (`:186`, beside `:269-270`) forwards the method.
- `Registration::delivery_policy()` mirrors `composer_policy()` (`:989`).
- New `Registry::delivery_policy(&self, harness_id: &str) -> Option<&'static dyn DeliveryPolicy>`, plus `Registry::push_channel_ids() -> Vec<&'static str>` for the clap parser. `"human"` and unknown ids give `None`.
- `Registry::new` refuses a policy whose `(mechanism, provenance, idle_wake)` is not one of:
  - `(PushChannel, cooperative_mod_delivery, Channel | Native)`;
  - `(HookPull, cooperative_hook_delivery, Native)`.

  The error is a new `RegistryError::InvalidDeliveryPolicy`. A provenance string is honest only if A3 defines it, and a pull harness has no channel that could wake it.

Core always reaches a policy as `builtins().delivery_policy(&binding.harness)`, keyed by the canonical binding's harness id and never by a client hint.

**Considered:**
- A `DeliveryPolicy` with an associated type, or generic over `Admission`: not object-safe through `ErasedAdapter`.
- A bool `suppress_wake()` instead of `idle_wake()`: rejected by the root decision log, item 9.

### DP2. `src/delivery/` module API, extracted byte-identically

**Chosen.** `src/lib.rs` gains `pub mod delivery;` with these files:
- **`delivery/assemble.rs`.** `Assembler { emitted: HashSet<String>, partial: HashMap<String, String>, command_prefix: Vec<String>, truncation_hint: Option<PathBuf> }` with `fn push(&mut self, item: InboxBatchV2Item) -> Result<Assembled, AssembleError>`.
  - `Assembled` is one of: `Item(DeliveryItem)`, `Pending` (chunk buffered), `Attention`, `InformationalNotice`, `Duplicate`.
  - It contains `emit_chunk` (`cli/watch.rs:692-760`), `watch_label` → `label()` (`:763-791`) and `record_truncated` / `read_truncated_hint` (`:793-820`), moved verbatim.
  - `DeliveryItem { kind: Message | Lazy, id: MessageId, message: WatchMessage }`. It reuses the frozen `protocol::watch::WatchMessage`, so `watch` lines are unchanged.
- **`delivery/page.rs`.** `trait PageSource { fn page(&mut self, cursor: Option<String>) -> Result<InboxBatchV2Page, PageError>; }` and `fn drain_pages(source, assembler, max_pages, sink: impl FnMut(DrainEvent) -> Result<Stop, E>)`.
  - `DrainEvent` is `Item | Attention | AttentionCleared | Exhausted`.
  - The loop, `MAX_DRAIN_PAGES`, the informational-warning skip and the `attention_cleared` retraction move from `cli/watch.rs:522-672` unchanged.
  - Page constants re-export `protocol::watch::{WATCH_BODY_LIMIT_BYTES, WATCH_PAGE_MAX_ITEMS, WATCH_PAGE_MAX_BYTES}` as `BODY_LIMIT_BYTES`, `PAGE_MAX_ITEMS` and `PAGE_MAX_BYTES`. The originals stay where `tests/protocol/watch.rs` reads them.
- **`delivery/frame.rs`.** A Rust port of `register.js:10-58`:
  - `pub const HEADER: &str`, the exact JS string;
  - `one_line`, `indent` (the JS `LINE_BREAK` set `\r\n|[\n\r\u{b}\u{c}\u{85}\u{2028}\u{2029}]`), `markers`, `label` (the JSON-quoted name);
  - `pub fn frame(items: &[FrameItem]) -> String` with `FrameItem { kind, id, thread, thread_name, sender, sender_name, author_role, relays_user, user_intent, body }`, where `kind = "attention"` uses the short header form.

  Golden fixtures `tests/fixtures/delivery/frame-*.{json,txt}` are generated once from the JS `frame()`. They cover:
  - an empty item list;
  - a multi-line body with `\r\n`, `\u2028` and an empty line;
  - control characters in the thread and sender;
  - a name equal to its id;
  - every marker;
  - attention.

  They are asserted by a Rust test and by a **new** mod test, `integrations/claude/mod/tests/frame_golden.test.ts`. Existing mod tests are untouched.
- **`delivery/mod.rs`.** Re-exports, plus (from L3 on) `settle` and (from L4 on) `ledger` and `routing`.
- **`cli/watch.rs` becomes a facade.** `drain(client, clock, seat, frame_version, state, out)` keeps its exact signature and wraps `drain_pages` with a `ClientPageSource` and a line-writing sink. `EmitState`, `EmitState::new`, `with_command_prefix`, `record_truncated` and `hint_path` remain reachable under the same names, by re-export or a thin wrapper, so `tests/cli/watch.rs` (`use super::*`) compiles unchanged.

**Considered:**
- Framing in the agy hook client: the daemon would then hand out unframed bodies. Framing in two places means two implementations to keep in parity with the JS.
- Moving the JS to call the Rust frame: it would change mod behaviour.
- Leaving the drain code in `cli/watch.rs` and copying it for pull: two copies of D4.

### DP3. Every Claude check becomes a registry policy lookup, with `not_claude` preserved

**Chosen:**

| Site | Becomes |
|---|---|
| `protocol/watch.rs:257` `WatchRefusalReason::NotClaude`, `:312` `WatchStatusReason::NotClaude` | Variants kept (frozen wire `not_claude`; allowlisted per ht-poh.1). Add `pub const NO_PUSH_CHANNEL: Self = Self::NotClaude;` on both enums. Core code names only `NO_PUSH_CHANNEL`. |
| `service/dispatch.rs:634` (claim not Claude or not top-level) | `claim.role != TopLevel \|\| !declares(claim.harness, PushChannel)` → `Err(Refusal::NO_PUSH_CHANNEL)` |
| `dispatch.rs:653` (`binding.harness != "claude"`) | `!declares(&binding.harness, PushChannel)` → `NO_PUSH_CHANNEL` |
| `dispatch.rs:670` `harness: Harness::Claude` | `harness: claim.harness`. The registration also gains `idle_wake: policy.idle_wake()` (DP7). |
| `dispatch.rs:531` (binding filter `== "claude"`) | `declares(&binding.harness, PushChannel)` |
| `dispatch.rs:557` (resume claim `Harness::Claude`) | `Harness::Agent(builtins().agent(&binding.harness)?)` |
| `store/receipts.rs:144-150` | Inside the new `ack_policy_delivered(ctx, conn, budget, AckPolicyDelivered{ mechanism, provenance, via: &'static str, messages, operation, claim, digest }, permit)`: role TopLevel and `declares(claim.harness, mechanism)`, else `InvalidRequest "delivery ACK requires a top-level claim of a harness with this delivery policy"`. `ack_mod_delivered` keeps its signature and payload (`ack_mod_delivered_payload`, `:52`), observation JSON (`action_provenance`, `via`) and per-id results, and calls it with `(PushChannel, policy.provenance())`. For Claude that is `cooperative_mod_delivery`, which is byte-identical. `plan_mod_ack` (`:68-117`) becomes the pure `delivery::settle::plan(SettleFacts) -> SettlePlan` plus the store's SQL fact reads, with identical decisions (truncation decided from the stored body length). |
| `cli/watch.rs:933-991` `CallerFailure::NotClaude`, `:986` | `CallerFailure::NoPushChannel` (private), `!declares(current.harness, PushChannel)`. It maps to `WatchStatusReason::NO_PUSH_CHANNEL`. |
| `cli/commands.rs:1097` `value_parser = ["claude"]`, `:2930` | `PossibleValuesParser::new(builtins().push_channel_ids())` and `Harness::Agent(builtins().agent(&value)?)`. The help text and refusal text name `<id>` ("watch needs --harness ID and --session ID…"). With only Claude declaring a push channel, the possible-values list is the same `[claude]`. |
| Doc comments naming Claude in `protocol/watch.rs`, `service/mod_channels.rs:1`, `protocol/attention.rs:178-180` | Neutral wording ("a push-channel harness's mod"). |

The Claude adapter (`src/harness/claude.rs` / `src/harness/claude/`) declares `ClaudeDelivery: (PushChannel, COOPERATIVE_MOD_DELIVERY_PROVENANCE, Channel)`. The mod assets and their setup stay in the Claude adapter, unmoved.

**Considered:**
- Renaming the Rust variant to `NoPushChannel` with `#[serde(rename = "not_claude")]`: the wire stays the same, but it breaks the existing tests' glob imports, which violates "existing tests unchanged".
- Leaving `harness: Harness::Claude` in the resume claim: it is wrong for any other push harness.

### DP4. `DeliveryPull` / `DeliveryConfirm` wire

**Chosen.**
- A new module, `src/protocol/delivery.rs`.
- Capability `pub const DELIVERY_PULL: &str = "delivery.pull_v1";` in `protocol/capabilities.rs`. It is added to `ADVERTISED` (`:50`) only when the handlers land (L4), with a probe arm in `every_advertised_capability_has_a_handler`.
- `PROTOCOL_VERSION` stays 6 (`wire.rs:34`), as `MOD_WATCH` did.
- Both are ordinary one-shot `Command`s.
- An older daemon fails to decode them. The hook treats any error as "print nothing", so it never probes capabilities on the hot path.

```rust
#[serde(deny_unknown_fields)]
pub struct DeliveryPull {
    pub claim: CallerClaim,
    /// `attention_version` of the last Page/Unchanged reply this conversation saw.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub seen_attention_version: Option<u64>,
    /// argv the truncation marker's commands start with (`cli::hook::cli_prefix`); empty = bare program.
    /// ≤ 8 args, ≤ 4096 bytes total, validated.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub marker_prefix: Vec<String>,
}
// CommandResult::DeliveryPulled(DeliveryPullReply)
#[serde(tag = "kind", content = "args", rename_all = "snake_case")]
pub enum DeliveryPullReply {
    Unchanged { attention_version: u64 },
    Page(DeliveryPage),
    Refused { reason: DeliveryRefusalReason },
}
pub struct DeliveryPage {
    pub attention_version: u64,
    pub text: String,              // delivery::frame output: HEADER first, never starts with '/'
    pub offered: Vec<MessageId>,   // non-truncated message/lazy ids now in the ledger
    pub truncated: Vec<MessageId>, // shown cut; never confirmable
    pub more: bool,                // stopped at a page bound
}
#[serde(rename_all = "snake_case")]
pub enum DeliveryRefusalReason { NoBinding, SessionMismatch, Held, Unresolved, NoHookPull, Busy }

#[serde(deny_unknown_fields)]
pub struct DeliveryConfirm {
    pub claim: CallerClaim,
    pub ids: Vec<MessageId>,        // 1..=MAX_BATCH_ITEMS (100), no duplicates (validate())
    pub operation: OperationId,     // accountable transaction / idempotent replay
}
// CommandResult::DeliveryConfirmed(DeliveryConfirmReport { results: Vec<DeliveryConfirmItem> })
pub struct DeliveryConfirmItem { pub id: MessageId, pub result: DeliveryConfirmOutcome,
    #[serde(default, skip_serializing_if = "Option::is_none")] pub reason: Option<DeliveryConfirmReason> }
#[serde(rename_all = "snake_case")]
pub enum DeliveryConfirmOutcome { Settled, AlreadySettled, NotOffered, RefusedTerminal, StaleGeneration, Retryable }
#[serde(rename_all = "snake_case")]
pub enum DeliveryConfirmReason { Unknown, NotAddressed, Truncated, NoHookPull, Busy }
```

**`attention_version` semantics.** It is an opaque u64: FNV-1a 64 over the fields of the seat's `ModFingerprint` (`ports.rs:2490`, produced by `store::seats::mod_seat_view`, `seats.rs:4568`). The fingerprint already moves with every pending source (receipts, lazy rows, waking warnings, invitations, binding generation). The `wake_work.attention_version` counter alone does not move on settlement, so after a bounded page it would hide the rest.

**`Command` plumbing** (`protocol/commands.rs`):
- `Command::DeliveryPull` is on the read dispatch path (like `AttentionDigest`, `dispatch.rs:786`).
- `Command::DeliveryConfirm` plus `PermitMutation::DeliveryConfirm` go on the cooperative mutation path.
- `claim()` / `validate()` arms.

**Considered:**
- Reusing `ModAckOutcome`: it has no `not_offered`, and it is part of the frozen mod contract.
- A streaming connection like `watch`: agy hooks are one-shot processes.
- The daemon returning structured items for the hook to frame: framing would live in two places, and the D4 text would be built client-side.
- Omitting `marker_prefix`: the daemon cannot know the pane's selectors, and the truncation marker must print commands the agent's shell can run (v0.5.0 note ht-j16.20/.24).

### DP5. Offered ledger: storage, restart and rotation

**Chosen: process-local.**
- `delivery/ledger.rs` holds a pure data structure, `OfferedLedger { seats: HashMap<SeatId, SeatOffers> }`:
  - `SeatOffers { generation, native_session, more_pending: bool, offers: BTreeMap<MessageId, Offer { first_at, last_at, count: u8, truncated: bool }> }`.
  - Bounds: `MAX_LEDGER_SEATS = 4096`, evicting the least recently touched seat; `MAX_OFFERS_PER_SEAT = 256`; a full table records no new id and evicts none (new ids wait for `inbox`; ht-poh.59).
- `src/service/delivery_pull.rs` owns it behind a `Mutex`. The lock is never held across a store read.

**Pull** (in the daemon, using the DP2 `Assembler` with a store-backed `PageSource` over `InboxBatchV2`):
1. Run the A2 decision (DP6).
2. Read the fingerprint and compute `attention_version`.
3. **Short-circuit to `Unchanged`** when `seen == attention_version`, `!more_pending`, and no ledger offer is due.
4. Otherwise assemble oldest-first. An id with a ledger offer is skipped unless it is **due**: `now - last_at >= REOFFER_AFTER_MS (60_000)` and `count < MAX_OFFERS_PER_GENERATION (3)`.
5. Include one `attention` block when `seen != attention_version` and a waking invitation or warning is pending.
6. Stop at 32 blocks or 64 KiB of framed text, always including at least one block.
7. Record every included message or lazy id (count+1; truncated ids flagged), and drop ledger entries for ids no longer pending.
8. An empty page is `Unchanged`.

**Rotation.** Any pull or confirm whose canonical binding generation differs from `SeatOffers.generation` discards the entry before proceeding. That is the root's "a rotated generation discards its offered set", decided against the canonical view without coupling to the check-in code.

**Restart.** A restart loses the ledger. A later confirm returns `not_offered`, the items stay pending, and the next pull offers them again: at-least-once. This is the same accepted window as the mod after a daemon restart (v0.5.0 Post-Implementation Notes).

**Considered:**
- A persistent `delivery_offers` table: the pull would become an accountable write through the fair writer on a 1.5 s hook path. It would also need a migration and schema-list churn. Its only gain is no duplicate after a restart.
- Re-offering until confirmed with no bound: re-injects up to 64 KiB before every model call when the transcript is unreadable.
- Never re-offering: an injection agy silently dropped would lose the item until `inbox`.

### DP6. A2 decision rules for pull and confirm

**Shared read.** One store read transaction through `mod_seat_view`. The pair is refused in this order:

| Condition | Refusal |
|---|---|
| `claim.role != TopLevel` or `claim.instance != self.instance` | `session_mismatch` |
| seat missing, retired, or `!continuity_resolved` | `unresolved` |
| target held | `held` |
| no open binding, or provenance ≠ `cooperative_top_level` | `no_binding` |
| `builtins().delivery_policy(&binding.harness)` mechanism ≠ `HookPull` | `no_hook_pull` |
| `binding.native_session != claim.native_session` | `session_mismatch` |
| store busy | `busy` |

The claim's generation is a hint. The ledger is keyed by the canonical generation.

**Confirm.**
- Every id gets a result.
- Ids not in `SeatOffers` for the canonical generation get `not_offered`. When the claim's generation is older, they get `stale_generation`.
- The remaining ids go to `ack_policy_delivered(mechanism = HookPull, provenance = cooperative_hook_delivery, via = "transcript", digest = delivery_confirm_payload(..))`, which decides per id against A2 again:
  - ordinary pending receipts settle exactly as `AckDisplayed`;
  - lazy rows complete as displayed;
  - truncation (from the stored body length) gives `refused_terminal/truncated`.
- Settled and already-settled ids leave the ledger.

### DP7. Routing: digest suppression, idle wake and "channel live"

**"Channel live"** stays today's per-seat predicate: any `ModChannelRegistry` entry in Live, ReconnectGrace or RebindGrace, with `!stalled` (`scheduler/mod.rs:610`, and the v0.5.0 note "Liveness is per seat").

**Idle wake.**
- `ports::ModChannelRegistration` gains `idle_wake: IdleWake`, taken from the policy at registration (DP3).
- `ModChannels` gains `fn wake_suppressed(&self, seat: &SeatId) -> bool { self.seat_live(seat) }`. The registry overrides it with "a live entry whose `idle_wake == Channel`".
- The scheduler's `mod_suppressed` (`:610`; callers `:290`, `:703`, `:936`) calls `wake_suppressed` instead of `seat_live`. For Claude (`Channel`) this is identical.

**Per mechanism:**
- **`PushChannel` + `Channel`:** the dispatcher skips the seat while the channel is live, and falls back to Native once it is not (the existing grace expiry and kick).
- **`HookPull` + `Native`:** no channel exists, so the existing dispatcher wakes as today. The woken turn's `PreInvocation` pull delivers.

**Digest suppression.** New `delivery/routing.rs`:
```rust
pub enum Boundary { Lifecycle, Tool }
pub fn omit_digest(policy: Option<&dyn DeliveryPolicy>, channel_live: bool, b: Boundary) -> bool {
    match policy.map(|p| p.mechanism()) {
        Some(DeliveryMechanism::HookPull) => matches!(b, Boundary::Tool),
        _ => channel_live, // PushChannel and None: exactly today's mod_channel_live rule
    }
}
```
- It is used at `cli/hook.rs:2468-2491` (Lifecycle) and `harness/bridge.rs:1206-1213` / `:1267` (Tool), with the policy looked up from the event's harness.
- This is presentation, not authority, so a compiled registry lookup is enough.
- The wire flag `AttentionDigest.mod_channel_live` (`protocol/attention.rs:181`) is unchanged.
- `HookPull` keeps the lifecycle digest because the agy SessionStart context (root §3, ht-poh.4) shows non-body attention there. `PreInvocation` pulls have no digest path.

**Considered:**
- Having the daemon set `mod_channel_live = true` for pull seats: an extra binding read on every digest call, and it overloads a frozen field's meaning.
- Suppressing the lifecycle digest too: it conflicts with root §3's SessionStart context.

### DP8. `wake_on`: storage, command, core wake decision, push-channel promotion

**Storage.** The next free migration (`0029` on current main; use whatever is free at merge) adds:
```sql
ALTER TABLE seats ADD COLUMN wake_on TEXT NOT NULL DEFAULT 'nudge' CHECK(wake_on IN ('nudge','all'));
ALTER TABLE wake_work ADD COLUMN last_lazy_ordinal INTEGER CHECK(last_lazy_ordinal IS NULL OR last_lazy_ordinal>0);
```
- `store/schema.rs` expected lists (`:2348`, `:2512`) are updated.
- The preference is per seat. It survives `/clear`, resume and rebinds, and a fresh seat starts at `nudge`.

**Wire** (`protocol/commands.rs`, beside `DeliveryMode` at `:752`):
- `enum WakeOn { Nudge, All }` (snake_case);
- `Command::SetWakeOn(SetWakeOn{ value, operation, claim })` as a `PermitMutation`;
- `Command::WakeOn(WakeOnQuery{ seat })` as a read;
- `CommandResult::WakeOn(WakeOnState{ seat, wake_on, changed })`;
- capability `seat.wake_on_v1`; version stays 6.

**Store.** `seats::set_wake_on` runs in an accountable cooperative transaction (scope `seat:<id>`). It requires the current binding with a top-level claim and refuses declared subagents. When the value changes it calls `wake::note_new_attention` (`wake.rs:28`), so pending lazy rows are re-judged.

**CLI.** `src/cli/wake_on.rs` provides `herdr-threads wake-on [nudge|all]`:
- with a value: sets it and prints `wake_on: all (was nudge)`;
- without one: prints the current value;
- `--json` follows existing output conventions.

The pane resolves to the caller through `derive_caller`, like other cooperative mutations.

**Core wake decision.**
- `store::attention::wake_seat_attention` (`attention.rs:1068`) reads `seats.wake_on`. For `all` it reads the seat's pending-lazy summary. That query is extracted from `seats.rs:4627-4631` into `lazy_delivery::pending_summary(db, seat) -> (count, max_ordinal)`, shared with `mod_seat_view`.
- The summary goes into `EffectiveSeatAttention` and `LogicalAttentionFrontier` (`ports.rs:2175`) as `lazy: Option<i64>` (max ordinal). `advanced_beyond` (`:2181`) includes it.
- `wake::load_candidate` (`wake.rs:43`) reads `last_lazy_ordinal` into `last_reserved_frontier`, and `reserve` (`:448`) writes it with the other `last_*` frontier columns.
- The `WakeAttentionWitness` re-check covers it.
- `reason_labels` adds `lazy`.
- The batch window treats the oldest pending lazy row like an ordinary receipt.
- Lazy publication (`store/materialization.rs:439` `bump` on the lazy recipient path) sets the reason bit (the `note_new_attention` write) for recipients whose `wake_on='all'`.
- Pokes are unaffected: lazy rows have no deadline.
- Contract rule 4 (idle only) is unchanged: the dispatcher's idle and composer checks still gate.

**Push-channel promotion.**
- `InboxBatchV2Item::LazyMessage` (`results.rs:1115`) gains `#[serde(default, skip_serializing_if = "is_false")] wakes: bool`, set by the store's `InboxBatchV2` query when the reading seat's `wake_on='all'`. This follows the precedent of `informational` (`:1140-1145`, ht-j16.34): absent on the wire when false, so default seats are byte-identical.
- `delivery::page` emits a `wakes` lazy row as a `WatchItem::Message` with `ack_required: false`.
- The unchanged mod then context-attaches it while busy, or submits it as a turn when idle, and acks via `context` or `submit`. `plan` already completes lazy-mode ids regardless of `via` (`receipts.rs:89-103`).
- Pull pages are unaffected: they deliver every pending item anyway.

**SKILL.** One line in `integrations/skill/SKILL.md`, "Communication: scope and attention", which keeps the file under the 200-line test:
> - **Waking on every message.** By default only nudges (`send --nudge`, ACK requests, invitations, warnings) wake you when idle; lazy messages wait for your next turn. A coordinator or moderator that must react to every message but does not loop on its own may run `herdr-threads wake-on all` (`wake-on nudge` restores the default; bare `wake-on` shows it). Every wake costs a turn, so ordinary workers keep the default.

**Considered:**
- `wake_on` on the binding: lost on every `/clear`, while the role persists across sessions.
- A client-local preference file: it would be a hint deciding a daemon action, which violates A2.
- Changing the mod to submit lazy rows when a flag is set: that changes mod code and behaviour.
- Leaving `wake_on` unhonoured for a live mod: it silently breaks R11 for the most common coordinator seat.
- A separate `bump` table: the existing `wake_work` reason bit already drives re-derivation.

### DP9. TRUST-POLICY edits (each in the leaf that adds the code)

**L3:**
- A3 `cooperative_mod_channel` / `cooperative_mod_delivery` (`TRUST-POLICY.md:142-143`): "harness `claude`" and "top-level Claude session" become "a harness whose adapter declares a push delivery channel (today Claude)". Semantics are unchanged.
- A4 "Mod channel" (`:222-230`): the channel replaces native wake only when that adapter's policy says the harness wakes itself (`idle_wake: Channel`); "the Claude check-in results" becomes "that harness's check-in results".
- A5 rows (`:281-282`): the same rewording.
- A8 (`:402`, `:413-416`): "the bundled Claude mod" becomes "a push-channel mod".

**L4:**
- A3 new row `cooperative_hook_delivery` (receipt action observation; lazy completion claim), with the root §4 text. It adds that the daemon recorded the id as offered to that binding generation by a `DeliveryPull`, and that the hook reported it found the id's frame header in the session's transcript.
- A4 bullet "Hook-pull delivery": no channel and no wake change; tool-boundary check-ins omit the digest; the lifecycle check-in keeps it.
- A5 row: `DeliveryPull` / `DeliveryConfirm` by the current top-level binding of a hook-pull harness with a matching native session; settlement only for ids offered to that canonical generation; the daemon re-decides each id.
- A8: hook pull as a third completion source.
- Accepted limits:
  - the hook claim is cooperative (any same-user process can send `DeliveryConfirm` for offered ids);
  - the ledger is process-local (a restart re-offers);
  - re-offers are bounded at 3 per generation 60 s apart, after which the item waits for `inbox`;
  - a resume rotates the generation and re-offers items the resumed transcript already holds;
  - a native wake marker names `inbox`, so a woken pull session may display bodies twice (never settle twice).

**L5:**
- A5 row "set `wake_on`": the seat's current top-level binding (cooperative; changes wake eligibility only, never receipts or authority).
- Accepted limit: any same-user process can flip it.

**L6:**
- A8 line 413-ish "Lazy delivery creates no receipt, deadline, ACK evidence, attention, wake, poke…" gains "…except that a seat with `wake_on: all` is woken (wake eligibility only; still no receipt, digest attention or poke)".

**L7:**
- A8: a push-channel mod may complete a `wake_on: all` lazy row delivered via `context` or `submit`, with the claim "entered the model's context through the mod".

The root §4 startup-enrollment text for agy is ht-poh.4's. The "transcript the hook cannot read" limit is ht-poh.5's.

### DP10. Byte-identity guard for the Claude mod

**Mechanical gate.** For every leaf, `git diff <base> --` over the following files is empty:
- `integrations/claude/mod/hooks/`;
- `integrations/claude/mod/tests/delivery.test.ts` and `stress.test.ts`;
- `tests/protocol/watch.rs`, `tests/cli/watch.rs`, `tests/daemon/watch_connection.rs`;
- `tests/service/{mod_channels,mod_ack_dispatch,mod_digest_flag}.rs`, `tests/store/mod_ack.rs`;
- `tests/cli/hook.rs`, `tests/harness/bridge.rs`, `tests/harness/cooperative.rs`, `tests/protocol/attention.rs`.

**Tests that must pass unchanged:** those suites (`nice cargo test --locked --all-features watch mod_ack mod_channel mod_digest hook bridge cooperative attention`) and `claude plugin test` on the mod (skipped with a recorded reason when no `claude` binary is available).

**Permitted additions to existing tests:**
- `tests/protocol/capabilities.rs` may gain probe arms for the new capabilities only.
- The gated live stress `tests/native/claude_mod/stress.py` is re-run once at the epic's end when a signed-in profile exists.

**New guards:**
- the Rust and TS frame golden parity;
- a test that the Claude `watch --harness` possible-values list is still `["claude"]`;
- a wire snapshot that `WatchRefusalReason::NO_PUSH_CHANNEL` serializes as `"not_claude"`.

### DP11. Dependencies and accounting

- **ht-poh.1** (check script plus allowlist with frozen `not_claude`) gates L1 and every leaf that removes `to-migrate` entries.
- **ht-poh.7** (contract audit with inline fixes) gates the refactor leaves L2, L3 and L4, and the wake-decision leaf L6.
- **ht-poh.5** consumes L1 only: the trait plus the wire.
- Every commit carries `Outside-adapter: <list>`:
  - L1 and L4: trait extensions and new core the agy adapter needs;
  - L2: a reworked implementation (extraction);
  - L3: the Claude re-fit (reworked implementation plus callers);
  - L5, L6 and L7: an R11 core feature, not attributable to the agy adapter.

### DP12. Admission evidence for every push path (2026-10-23, ht-admission-ack)

A delivery claim settles a receipt only on evidence the host gives that the item's frame entered the session (a stored row, a model request that carries it), never on emission alone: a call that returned, a 2xx, a queue acceptance. Where the host returns or exposes an id for that evidence, the adapter reports it with the ACK and the receipt's ACK observation records it. An adapter whose host offers no observable admission point declares no push channel for that path.

- Claude mod (shipped): `$.session.append` resolves `{ message, uuid }` for the stored row, or `{ deny }`. The mod passes the uuid as `watch ack --via append --row <uuid>` (sent only to a daemon advertising `mod.ack_row_v1`), and the observation records `row`. An append result without a uuid still settles, without `row`: the row was stored (no `deny`), and refusing it would re-append a row that is already in the transcript. `context` and `submit` have no row to report.
- Pi (future adapter): the extension's `sendMessage` returns `void`, so its call cannot be the predicate. The adapter puts our ids in the custom message's `details`, which Pi drops when it converts the message for the model, so the ids cost no tokens, and it ACKs only after it sees those ids in a `context` event (the message is in a provider request) or at least in `message_start`. Until then it does not ACK; an aborted run, or no proof, re-offers through the existing ledger.
- OpenCode (future adapter): 1.x ACKs when the frame header is seen in the `experimental.chat.messages.transform` input (the messages of the next model step); 2.x ACKs on the durable `session.inbox.delivered` event for a message id the plugin chose. A `promptAsync` 2xx is a hand-off, not admission.
- Each new predicate gets its TRUST-POLICY A3 wording in the commit that adds the adapter (AGENTS.md "Trust policy").

Sources: `docs/research/2026-10-23-integration-alternatives/` (README section 4 idea 12; `comparison/claude.md` D15, `comparison/pi.md`, `comparison/opencode.md`), with the host shapes checked against Claude Code 2.1.309 plugin types (`SessionAppendResult`), pi-mono `packages/coding-agent/src/core/extensions/types.ts` and `core/messages.ts`, and OpenCode `packages/plugin/src/index.ts` at v1.19.4 and `packages/schema/src/session-event.ts` on dev.

## Design roast round 1 amendments (2026-10-10)

- **`DeliveryConfirm` from any top-level hook.** It may be sent from any top-level hook of the binding: the agy adapter sends it at `Stop` as well as at `PreInvocation`. This needs no wire change.
- **Wake eligibility.** Items confirmed at Stop are settled, so they are no longer pending and cannot trigger an idle wake.

## Acceptance

- `HarnessAdapter::delivery_policy()` exists with default `None`.
  - Claude declares `(PushChannel, cooperative_mod_delivery, Channel)`. Codex and Hermes declare `None`.
  - `Registry::new` rejects an invalid pairing (unit test).
- `scripts/check-harness-neutral` passes with every `to-migrate` entry gone for:
  - `service/dispatch.rs`, `service/mod_channels.rs`, `cli/watch.rs`, `store/receipts.rs`;
  - the `cli/commands.rs` watch args, `protocol/watch.rs` doc text, `protocol/attention.rs`.

  The only remaining entries in these files are the frozen `NotClaude`/`not_claude` wire entries.
- All DP10 guards hold: the files show no diff, and the existing suites and mod tests pass.
- `src/delivery/` exists. `cli/watch.rs` holds no assembly, truncation, label or paging logic. The Rust `frame()` matches the JS golden fixtures byte for byte.
- With a test-support `HookPull` adapter (`synthetic_fourth` declaring `(HookPull, cooperative_hook_delivery, Native)`), daemon tests show:
  - pull returns a framed page whose `offered` ids settle on confirm under `cooperative_hook_delivery`, ordinary and lazy alike;
  - a second pull with the returned version gives `Unchanged`;
  - `more: true` pages continue;
  - an unconfirmed id is re-offered after 60 s, at most 3 times;
  - a rotated generation discards offers (`not_offered` / `stale_generation`);
  - truncated ids are refused;
  - a Claude or Codex binding is refused as `no_hook_pull`, and a mismatched session as `session_mismatch`;
  - `delivery.pull_v1` is advertised with a probe arm.
- Routing:
  - tool-boundary check-ins omit the digest for `HookPull` seats; lifecycle check-ins keep it;
  - `wake_suppressed` is false for a live channel whose policy says `Native`, and identical to today for Claude.
- `herdr-threads wake-on all|nudge|<none>` sets and reads the seat value; subagent claims are refused.
- A seat with `wake_on: all` and only a pending lazy row:
  - is reserved for a native wake when idle;
  - is not re-reserved for the same lazy frontier;
  - is not woken at `nudge`.
- A live-mod seat with `wake_on: all` sees that lazy row as a `message` line with `ack_required: false`. A default seat's `watch` output is byte-identical.
- SKILL contains the `wake-on` bullet and stays under 200 lines.
- The TRUST-POLICY edits in DP9 land in the same commits as their code.
- Each leaf passes `nice cargo clippy --locked --all-targets --all-features -- -D warnings`, `nice scripts/check-default-features` and its focused tests. No process is left running (`scripts/check-no-leaked-processes`).

## Post-Implementation Notes

> *As this design is implemented and iterated on — bug fixes, adjustments, anything that diverged from the assumptions above — append a dated note here, whether or not a formal debugging skill was used.*
