# Unified seat selector — design

Run: [2026-10-15-unified-seat-selector](run.md) · branch `super-auto/unified-seat-selector`

## Goal

Every CLI argument that names a seat accepts an exact seat ID or a name: a pane, tab or space
label, a live agent name, or a `/`-joined path of them. The CLI resolves the name to exactly
one seat, or fails and lists every candidate (a safety cap of 32, with an omitted count, guards
only pathological topologies). Nearer panes win ties: the caller's tab, then
its workspace, then anywhere. When a name was resolved, one stderr line shows which space, tab,
pane and seat it picked.

## Problem description

Threads already resolve names (exact ID, then joined, active and all-history name tiers;
`src/store/queries.rs` `resolve_thread`, frozen before dispatch in `src/cli/threads.rs`). Seats
do not. Every `--seat`/`SEAT` argument goes through `id(s, SeatId::parse)` in
`src/cli/commands.rs` and accepts only an exact, opaque seat ID. The only name path is a
separate flag family (`--space/--tab/--pane`, `send --require-ack-pane`; `src/cli/panes.rs`),
which searches only the caller's own tab unless parents are given, and which does not exist at
all for 1:1 `send --seat`, `seat inspect`, `seat retire`, `seat rebind SEAT` or `--replace`. A
person who sees `project/alice/sAb12Cd34` in `read` output cannot use it as a seat argument.
They must look up an opaque ID first.

## Main challenges

- **What is an "exact ID".** Seat IDs are opaque strings (`id!` macro, `src/protocol/ids.rs`),
  so whether a string is a seat ID cannot be decided syntactically. It has to be checked against
  the daemon.
- **Where names live.** Labels and agent names are live Herdr topology, read client-side today
  (`host.topology`). The seat mapping lives in the daemon.
- **Ambiguity.** `tryout/alice` can mean space/tab, tab/pane or space/pane, and a label may
  itself contain `/`. The user explicitly wants every one of those readings accepted.
- **Two command classes.** Reads must never allocate. Mutations that today accept a pane
  (`invite --pane`, `--require-ack-pane`) allocate through ordinary guarded resolution.
- **Trust policy.** Labels are locators, never identity (TRUST-POLICY.md C1: labels "never move a
  seat, allocate one, or end a binding"). Resolution must freeze one canonical seat ID before any
  journal intent, and a retry must never resolve the name again.

## Key decisions made

The resolution runs **client-side**, in a new pure module `src/cli/seat_selector.rs`. It takes
the selector string, any `--space/--tab` narrowing, one Herdr topology snapshot and the caller's
live pane, and returns one pane, or `NotFound`/`Conflict` with candidates. Around it sits a thin
runtime stage in `src/cli/mod.rs`, which:
1. checks whether the whole string is an existing seat ID (`SeatInspect`);
2. handles the read-output nickname form;
3. reads the topology only when a name actually needs it;
4. maps the chosen pane to a seat, by lookup for reads and operator commands and by guarded
   allocation for mutations that already allocate for panes;
5. freezes the seat ID into the parsed action;
6. prints the `resolved:` line.

No wire or protocol change is needed: `SeatInspect`, `Seats` (via `collect_pane_seats`) and
`ResolveSeat` already exist.

## Decision points

### D1. Resolution location: client-side pure resolver + runtime stage

**Chosen.** A pure resolver over `HostTopology` in `src/cli/seat_selector.rs`, plus a runtime
stage in `src/cli/mod.rs` that runs after thread-selector resolution and pane-target freezing,
and before any journal intent or dispatch. This mirrors `panes::resolve_cli_targets`, which
already reads topology client-side. The pure function is unit-testable against hand-built
topologies, as `tests/cli/panes_hint.rs` does.

**Considered.** A daemon-side `ResolveSeatSelector` command, mirroring `ResolveThread`. Rejected:
the daemon does not hold live labels or agent names, so it would need a host read the CLI
already performs, plus a wire version bump and capability gating for no gain.

### D2. Parse order

For each seat argument string `S`:

1. **Exact seat ID.** If `S` is a valid opaque ID (`SeatId::parse` succeeds) and `SeatInspect`
   finds it in this instance, retired seats included, use it. No topology read happens, so an
   exact ID keeps working with Herdr down (TRUST-POLICY C4 for reads). Without a daemon, the
   command fails as it does today.
2. **Nickname form `PREFIX/SEAT_ID`.** If `S` contains `/` and its last segment is an existing
   seat ID (same `SeatInspect` check), the seat ID wins. The prefix must be checked against that
   seat's live location:
   - The prefix is run through the resolver (D3) with the tier filter off (all tiers pooled).
   - It passes when the seat's mapped target (`mapping.target`) is among the matched panes.
   - If it is not, the result is `Conflict`. The message names the seat's actual live path and
     suggests the bare seat ID.
   - If the seat has no live mapped target (unresolved, retired or unbound), the result is
     `Conflict` "cannot verify location of SEAT".
   - If the host is unreachable, the result is `HostUnavailable`.

   This is what lets a `read` nick (`SPACE/PANE/SEAT_ID`) be passed as a seat argument: the
   trailing seat ID always decides. The prefix verifies only when the `read` renderer showed the
   labels unaltered. A nick whose labels the IRC renderer clipped (32 characters) or escaped
   fails verification with `Conflict`, which names the live path and suggests the bare seat ID.
   `read`/`follow` rendering itself is unchanged.
3. **Direct pane ID.** If `S` looks like a pane ID (`panes::looks_like_pane_id`) and no narrowing
   flags are given, it is used as the target without a topology read, as `PaneSelector::direct_id`
   is today.
4. **Names.** Read the topology and run the resolver (D3). A failed host read is
   `HostUnavailable` (exit and code as today for pane selectors). A name is never passed through
   as an ID.

**Considered.** Treating any `SeatId::parse`-valid string as an ID without a daemon check, as
today. Rejected: that makes every name syntactically an ID. Checking the daemon first, rather
than the topology first, keeps old scripts that pass IDs working byte-for-byte, with no Herdr
dependency.

**Narrowing with an exact ID or nickname.** When `--space`/`--tab` accompany an exact seat ID
(step 1) or a nickname (step 2), the seat's live mapped target must lie inside the narrowed
space/tab. That check reads the topology, which is fine because the caller asked for it.
- Outside the narrowed space/tab: `Conflict`, naming the seat's live path.
- No live target: `Conflict` "cannot verify location of SEAT".
- Host unreachable: `HostUnavailable`.

Without narrowing, an exact ID still reads nothing from Herdr.

**Which forms read Herdr.** The no-read guarantee covers exact IDs without narrowing only. A
nickname always reads the topology to verify its prefix:
- host unreachable → `HostUnavailable`;
- the seat has no live mapped target → `Conflict`;
- the seat's target is outside the prefix → `Conflict`.

**ID-shaped strings.** Only an *existing* seat ID takes step 1. A string that merely looks like
a seat ID but names no seat falls through to name resolution, and normally ends in `not_found`
after a topology read. A pane, tab or space label that equals an existing seat ID cannot be
reached by name; use its pane ID (a longer path ending in such a label takes the nickname form).
The docs state this accepted limit.

**Unverifiable nickname.** A nickname whose seat has no live mapped target (unresolved, retired
or unbound) is `Conflict` "cannot verify location of SEAT; pass the bare seat ID", as in step 2.
It is never `not_found`, because the seat exists.

### D3. Interpretations, matching and tiers (pure resolver)

The resolver collects candidates from every interpretation, then tiers them.

**Whole string as one segment.** `S` itself, unsplit, is always tried as a one-segment selector,
so a label containing `/` still matches.

**Split on `/`.** Segments are split on `/` (no escaping). An empty segment matches nothing, so
`a//b` only has its whole-string reading. Segments are assigned to the levels space ⊇ tab ⊇ pane
in order, with levels allowed to be skipped:

| Segments | Readings |
|---|---|
| 1 | pane · tab (only if it has exactly one pane) · space (only if it has exactly one tab with exactly one pane) |
| 2 | space/tab (sole pane) · tab/pane · space/pane |
| 3 | space/tab/pane |
| ≥4 | none (whole-string reading only) |

**Matching per level, all exact and case-sensitive** (no trimming, folding or fuzzy matching, as
for `--pane`):
- space: label or space ID;
- tab: label or tab ID;
- pane: label, any live agent name, or pane ID.

Each segment matches independently, and a reading that matches several entities at one level
yields every combination.

**Narrowing.** Seat narrowing is its own value, independent of the `--pane` selector.
- When a command's `--seat` is present, the parser stores `--space`/`--tab` in a dedicated
  `SeatNarrowing { space, tab }` on `ParsedCli`, and never fills `pane_selector`.
  `resolve_cli_targets` never reads `SeatNarrowing`.
- The stage resolves `space` (exact ID or unique label) and, only if given, `tab`: exact ID or
  unique label within the given space, or else within the caller's space, as for `--pane`.
- There is no tab defaulting: `--seat alice --space proj` searches every tab of `proj`.
- Every candidate outside the given space, or outside the given tab, is dropped before tiering.

**Pooling and tiering.** Candidates from all readings are pooled and deduplicated by pane ID.
Each pane keeps the list of readings that matched it. Panes are then ranked by distance from the
caller's live pane:
- tier `tab`: same tab;
- tier `space`: same workspace, other tab;
- tier `any`: everywhere else.

With no live caller pane (outside Herdr, or a stale `HERDR_PANE_ID`), everything is tier `any`.
The first non-empty tier wins:
- **One pane:** it is the result.
- **Several panes:** `Conflict` (exit 1).
- **No pane in any tier:** `NotFound` (exit 1). The detail names the selector and says it is
  neither a seat ID nor a matching name.

**Conflict detail.** It lists every candidate, with a safety cap of 32 and an `N omitted` count
beyond it (the user asked for every match; 32 only guards pathological topologies). Each candidate
gives:
- the reading that matched (`tab=tryout pane=alice`);
- its `space/tab/pane` path, rendered per D6's renderer contract;
- its pane ID;
- its seat ID, `seat=none` when the pane has no resolved seat.

The detail ends with the qualification hint: a longer path, `--space/--tab`, or an exact seat or
pane ID. Looking up candidate seats takes at most 32 bounded `collect_pane_seats` reads. A failed
lookup prints `seat=?` and does not hide the conflict.

**Considered.** Choosing between readings by a fixed priority (for example, tab/pane before
space/pane). Rejected: the user asked for all readings, with misresolves caught by distance
tiering plus the `resolved:` line. A fixed priority silently shadows the other readings.

**Considered.** Sharing tier semantics with `--pane`. Rejected for this run: `--pane` keeps its
strict scoped rule (docs/agent-usage.md "Ordinary `seat resolve`" paragraph). Changing it would
alter `launch`, `seat resolve` and `rebind` targeting, which needs target intent, not a search.
Unifying the two is a possible follow-on, not part of this tree.

### D4. Pane → seat, per command class

Once the resolver has picked a pane (D2 step 3 or D3), each command maps it to a seat as below.

| Seat argument | Pane → seat | Rationale |
|---|---|---|
| `inbox`, `pending-receipts`, `diagnostics`, `warnings`, `thread list` `--seat` | lookup: `pane_seat` (existing mapping, never allocates). No seat → today's `no_seat_for_pane` error | reads never allocate (TRUST-POLICY C4/A1) |
| `seat inspect SEAT`, `seat retire SEAT`, `seat rebind SEAT`, `seat rebind --replace NEW` | lookup | operator repair acts on an existing seat. The name only locates it; the operator still decides explicitly with `--operator` |
| `handoff --existing --seat` | lookup, same as handoff's existing `--existing --pane` path (exactly one resolved seat) | the peer already exists |
| `invite --seat`, `send --require-ack`, `send --seat` (direct 1:1) | guarded allocation: journaled `ResolveSeat` via `ServiceAllocation` scope, the same `allocate` closure `--pane`/`--require-ack-pane` use | consistent with the pane forms. An empty future agent pane can receive work, and restoration holds still refuse |

An exact seat ID (D2 step 1) or the nickname form (D2 step 2) skips this step: the seat is
already known.

`--cooperative-seat` stays exact and unchanged. It is hook and launch plumbing (caller identity),
never a recipient.

### D5. Freezing and retry

Resolution completes before any journal intent is written or any command is dispatched,
following the pattern `threads::resolve_cli_threads` and `panes::resolve_cli_targets` use. The
frozen `SeatId` replaces the raw string in the parsed action. Raw selector strings live only in
`ParsedCli` (a `seat_selectors` list, each entry naming the action slot it fills), never in a
`SeatId`, as `thread_selector` does today. Journal intents and retry records carry only the
canonical ID, so a rename, move or new pane after the fact never changes a retried operation.
`retry REF` never re-resolves.

### D6. The `resolved:` line

It is printed only when a name was resolved (D2 step 4), not for an exact ID, a direct pane ID or
the verified nickname form. It is one line per resolved argument, on **stderr**, so `--json` and
`--machine` stdout stays parseable. It is written before the command's own output:

```
resolved: project/tryout/alice w4:p7 seat=sAb12Cd34 (tier: space)
```

**Renderer contract (the one definition; every other mention points here).** Each part of a
rendered path is the plain label when that label is *safe*, and the host ID otherwise. A label is
safe when all of these hold:
- it is non-empty and at most 64 bytes;
- it contains no `/`;
- it contains no Unicode control (Cc), format (Cf) or whitespace characters;
- it is not shaped like a pane or tab ID;
- it is unique among its siblings at that level (no other space, no other tab in the space, or
  no other pane in the tab with the same label);
- it does not collide with what a three-segment reading could confuse it with. At the pane level,
  no other pane in the same tab has it as an agent name or pane ID; the pane's own agent name
  does not count, so a launched pane named after its label still renders its label. At the tab
  and space levels, no sibling has it as an ID.

No `{:?}` quoting and no escape grammar is used. Every part is therefore either a label that only
this entity carries at that level, or an ID, which never contains `/`. A rendered full path, typed
back as a seat argument, has a three-segment space/tab/pane reading that matches exactly the
rendered pane. It resolves to that same pane except in two residual cases, which the docs state:
- some other label literally equals the whole rendered string;
- the pane label equals an existing seat ID, so the pasted path takes the nickname form (D2
  step 2).

The resolve round-trip fixture includes a seat-ID-equal label to pin the second case.

`seat_selector` owns rendering and parsing together. Two tests pin them:
- a string round trip;
- a resolve round trip over a fixture with duplicate pane, tab and space labels, a label equal to
  another pane's agent name, and a label containing `/`. For every pane, resolving
  `render(pane)` returns that pane.

**One line per name.** One `resolved:` line is printed per distinct name argument (a repeated
`--require-ack NAME` prints once). An exact ID or verified nickname prints nothing.

**Machine output.** `--json`/`--machine` change nothing here. The line stays on stderr, and the
conflict and not_found candidates stay in the error's `detail` text: no wire or schema change,
and scripts disambiguate by rerunning with an ID or a longer path.

**Sizes (AGENTS.md (f)).**
- One `resolved:` line: 40–100 bytes typical, at most about 260 bytes (three 64-byte parts,
  pane ID, seat ID, tier).
- One conflict candidate row: about 80–200 bytes.
- The qualification hint: one trailing line, about 120 bytes, not repeated per row.
- Worst case at the 32-row cap: about 6.5 KB. This is only reached in pathological topologies;
  typical conflicts are 2–3 rows, under 600 bytes.

The path is always the **full live** `space/tab/pane` path of the chosen pane, whatever reading
or alias matched (a one-segment `alice` still prints `project/tryout/alice`). The path parts are
rendered per the renderer contract above. A seat allocated by the guarded path prints its new ID. **Size:** 40–100
bytes per line for typical labels (at most about 3 × 64-byte bounded parts plus IDs). Static
explanation stays in the skill (AGENTS.md token-efficiency rules (a), (b), (d), (f)).

### D7. Clap surface

All seat argument fields stay `String` and gain one help line: "SEAT: an exact seat ID, or a name
path SPACE/TAB/PANE (any part may be omitted)". `--seat` stops conflicting with `--space`/`--tab`
on the commands that flatten `PaneSelector`. With `--seat`, their values go to `SeatNarrowing`
(D3 Narrowing), never to the pane selector. `--seat` still conflicts with `--pane`, because a pane
selector and a seat selector would both name the target.

`send --seat` drops the "direct send accepts only its --seat recipient; pane ACK selectors are
not supported" refusal for `--space/--tab` only. `--require-ack-pane` stays refused for direct
sends, and the direct-ACK rule ("names only the target") applies after freezing, comparing frozen
IDs.

### D8. Errors and exit codes

| Outcome | Code | Exit |
|---|---|---|
| no match | `not_found` | 1 |
| ambiguous | `conflict` | 1 |
| nickname disagrees | `conflict` | 1 |
| host unreachable | `host_unavailable` | as today |
| seat mapped to a pane missing (reads) | `invalid_request` | 2, existing `no_seat_for_pane` |
| pane maps to more than one seat | existing `mapping_error` | — |

These match the existing `--pane` outcomes, so callers handle no new codes.

### D8a. Narrowing under `handoff --existing`

Under `--existing`, `--space`/`--tab` have no launch meaning: the existing peer is not launched.
With `--seat`, the parser moves them out of the handoff request's selector into `SeatNarrowing`
(D3), and they only narrow the seat name. Handoff's request never carries them, so its
seat-with-space/tab refusal (`handoff_delivery.rs` ~238/347) cannot fire, and no stage-side
clearing exists.

### D8b. Inventory of seat arguments

Every clap argument whose value is a seat is a `SeatSlot`. The one exclusion is
`--cooperative-seat`, which is caller identity plumbing and stays exact (D4). A parser test
walks the clap command tree and fails if any seat-valued argument (id or value name `SEAT`,
`NEW`, or a `--seat`/`--require-ack`/`--replace` flag) is neither recorded as a slot nor on the
reasoned exclusion list, so a future seat argument cannot bypass resolution.

### D9. Docs, skill, trust policy

- **`docs/agent-usage.md`.** A new "Seat selectors" paragraph next to the "Thread IDs and names"
  paragraph (line ~64). The pane-selector paragraph (~133–135) gains one sentence saying `--seat`
  names search beyond the caller's tab, by tier, while `--pane` keeps its scoped rule. This
  supersedes the "no implicit search across spaces or tabs" statement for `--seat` only.
- **`integrations/skill/SKILL.md`.** One line in the send/invite area: `SEAT` accepts names, and
  the `resolved:` line shows what was picked.
- **TRUST-POLICY.md.** C1 gains one sentence: a seat-argument name is a locator that the CLI
  resolves once to a canonical seat before submission. It allocates only where the matching pane
  form already allocates (invite, ACK recipients, direct send), through ordinary guarded
  resolution, and never moves a seat or ends a binding. No invariant is weakened. The sentence
  makes explicit that name selection is the same locator path as the existing `--pane` forms.
  A second sentence covers operator repair by name (`seat retire`/`rebind`/`--replace`): it acts
  only on the one seat the name uniquely resolves to through the current canonical mapping
  (lookup, never allocation). That seat is shown on the `resolved:` line, and the operator still
  decides with `--operator`. The name is not heuristic continuity evidence: it moves no seat by
  itself.

### D10. Tests

- **Pure resolver unit tests** over hand-built `HostTopology`, covering:
  - every row of the D3 readings table, and the whole-string label containing `/`;
  - pane ID, tab ID and space ID segments;
  - agent-name match;
  - sole-pane and sole-tab aliases;
  - tier precedence (same tab beats same space beats any), and no caller (all tier `any`);
  - narrowing;
  - conflict listing and its bounds (32 and 33 candidates, the 32-row cap boundary);
  - `NotFound`;
  - empty segments and four or more segments.
- **CLI integration tests**, through the existing fake host and daemon test harness used by the
  pane-selector tests:
  - each command in D4's table, with a name and with an exact ID;
  - exact ID with the host unavailable still works;
  - a name with the host unavailable fails `host_unavailable`;
  - nickname accepted, nickname mismatch `Conflict`;
  - the `resolved:` line present only for names, and on stderr;
  - reads never allocate (no `ResolveSeat` intent);
  - direct `send --seat NAME` to an empty pane allocates and sends;
  - `retry` of a direct send after a pane relabel keeps the frozen seat.

## Out of scope / follow-on

- Changing `--pane` to tiered resolution.
- Daemon-side seat name resolution.
- Messaging the main tab's agent to merge and release. That is the enclosing run's finish step,
  not part of this tree.

## Key abstractions

**`seat_selector`** — new module (`src/cli/seat_selector.rs`), the pure name resolver over one `HostTopology` snapshot.
- Owns: splitting a selector into readings (whole-string reading + the D3 segment table, with empty and ≥4 segments yielding nothing); per-level matching (space label|ID, tab label|ID, pane label|agent name|ID) and the sole-pane / sole-tab aliases; narrowing by already-resolved space/tab IDs; pooling and deduplicating by pane ID while keeping each pane's matched readings; ranking by `Tier` (closed enum `Tab | Space | Any`) and choosing the first non-empty tier; nickname-prefix verification (the same pool with tiering off, plus a membership test against the seat's mapped target); one path renderer (labels when safe per D6's Renderer contract, else host IDs, bounded; no `{:?}`) used by conflict candidates, the `resolved:` line and the nickname-mismatch message.
- Hides: which reading matched, the readings table, tier precedence, and the bounded wire text (the 32-candidate safety cap, `N omitted`, the trailing qualification hint).
- Must not: read the host or call the daemon, know which command it serves, allocate or look up seats, or print. Topology and caller come in as values; a candidate's seat is supplied by the runtime stage as a fallible lookup closure (rendered `seat=?` on failure), so the module stays pure and unit tests use hand-built topologies and a fake lookup.
- Relationships: a leaf. It depends only on `host::observation` types and `ApiError`. `panes.rs` keeps its own scoped `--pane` rule (spec D3 "Considered"); the two share only the parent-narrowing helper (see `PaneSelector`).
- Lifecycle: stateless functions over borrowed snapshots, one call per seat argument.
- Errors: a typed outcome, one pane (with tier, readings and path), `Ambiguous` (candidates) or `NoMatch`; the nickname check passes, or returns `Mismatch` (with the seat's live path) or `Unverifiable`. Each converts to `ApiError` (`not_found` or `conflict`, D8) in one place inside this module. Callers branch on the typed outcome, never on message text.

**`SeatSlot`** — new closed enum in `commands.rs`, beside `ParsedCli`. It names every action position a seat argument can fill: the seat of inbox, pending-receipts, diagnostics and warnings; the thread-list membership; the invite recipient; the direct-send target; `require-ack[i]`; the seat of inspect, retire and rebind; rebind `--replace`; the handoff existing seat.
- Owns: two `match` arms per slot. One is the setter that writes the frozen `SeatId` into the action (like `threads::selector_mut`); the other is the slot's D4 pane-to-seat class.
- Hides: where in each `CliAction`/`MutationSpec` a seat lives, and the D4 table. Today `resolve_recipient_seats` branches on `Invite` to choose allocate versus lookup, and `default_seat`/`resolve_recipient_seats` repeat the read-seat field match. Both go through this setter, so read-seat writes live in one place.
- Must not: resolve, look up or allocate; the stage does.
- Relationships: data owned by `ParsedCli`. The stage depends on it; it depends on nothing at runtime.
- Lifecycle: created in `parse` with its raw string, kept in `ParsedCli.seat_selectors`, drained by the stage. Retry and journal records never see it (spec D5).

**`SeatMapping`** — new two-variant closed enum, `Lookup | Allocate`, returned by `SeatSlot`.
- Owns: the D4 decision as a value.
- Hides: which reads, operator commands and handoff paths never allocate (TRUST-POLICY C4/A1).
- Must not: carry behaviour. The stage maps `Lookup` to the existing `pane_seat` (with `no_seat_for_pane`/`mapping_error` unchanged) and `Allocate` to the existing journaled `ResolveSeat` allocation under the `ServiceAllocation` scope.
- Relationships: the allocation is extracted once from `resolve_recipient_seats`'s `allocate` closure into a helper in `mod.rs`, shared by the `--pane`/`--require-ack-pane` path and this stage, so two callers exist now. The extraction is move-only: the closure body, nothing else. Refuse-before-allocate order is kept by `resolve_cli_seats`, not by the helper. Before its first `Allocate` mapping, the stage runs the same gates that today precede the closure, under the same conditions:
  - the agent-without-session refusal: only when `run_caller_scoped` would apply it (actor not `Human`, not the `--operator` branch, and `caller_needs_identity`);
  - `require_send_time_receipts_capability` for p0 sends;
  - the `INVITATION_CONFIRMATION_V1` capability for confirmed non-operator invites.

  A refused command therefore never leaves a journaled `ResolveSeat`, and operator orphan invites keep working from any pane.

**`resolve_cli_seats`** — new runtime stage in `src/cli/mod.rs` (spec D1), the only code that turns a `SeatSlot` raw string into a frozen `SeatId`.
- Owns:
  - the D2 order per slot: exact ID via `SeatInspect`, then the nickname form, then a direct pane ID, then a name;
  - a private classification (exact | nickname | pane ID | name) that decides whether mapping is skipped and whether a `resolved:` line prints;
  - the lazy, once-per-invocation topology and live-caller reads, shared across all slots like `resolve_cli_targets`;
  - resolving the `--space`/`--tab` narrowing once;
  - seat annotation of conflict candidates (at most 32 bounded lookups);
  - mapping through `SeatMapping` and freezing through `SeatSlot`;
  - the post-freeze ID checks moved out of parse time (D7: direct ACK names only the target; no duplicate recipients after dedup);
  - writing the `resolved:` lines to stderr before command output (D6).
- Hides: when the host or daemon is touched. An exact ID never reads topology, so it works with Herdr down.
- Must not: match names (`seat_selector` does), know command kinds (`SeatSlot` does), write any journal intent other than the existing allocation, or run on `retry`.
- Relationships: it calls `seat_selector`, `SeatSlot`, the shared allocate helper, `pane_seat` and `SeatInspect` through the lazy connection. It runs after `resolve_cli_threads`/`resolve_cli_targets` and before `derive_caller`, `resolve_recipient_seats` and dispatch; otherwise a read with a placeholder seat could be defaulted to the caller. Direct-send errors pass through the same `direct_send::pre_executor_error` seam as pane resolution.
- Lifecycle: one call per invocation; it owns nothing afterwards.
- Errors: `HostUnavailable` (topology read, for the nickname and name forms and for an exact ID with `--space`/`--tab`); `not_found`/`conflict` (resolver); `invalid_request` (`no_seat_for_pane`, for reads and operator commands); `mapping_error` (a pane with several seats); allocation retry failures as today. No new codes; callers distinguish by `ErrorCode`.

**`ParsedCli`** — extended: gains `seat_selectors` (a list of `SeatSlot` + raw string) and `seat_narrowing` (the `--space`/`--tab` values given with `--seat`, D3 Narrowing; never turned into a `pane_selector`). Action fields hold a placeholder `SeatId` until the stage freezes them, as `pending-pane-selector` does. The stage itself is the enforcement: at its end it fails (`invalid_request`, a bug-class error) on any slot it did not freeze. `commands::dispatch`'s guard also refuses leftovers, but `dispatch` is test-only, so that is not the production guarantee. Parse-time `SeatId` comparisons move into the stage.

**`PaneSelector` / `panes.rs`** — extended: `resolve_selector`'s hard-wired `--space`/`--tab` parent resolution (exact ID or unique label via `unique()`) is extracted into one helper that `resolve_selector` and the seat stage's narrowing both call, so the flag families cannot drift. The helper resolves a space (exact ID or unique label) and an optional tab (exact ID, or a unique label in that space). `--tab LABEL` without `--space` is scoped to the caller's space, and outside Herdr it needs `--space` or a tab ID. It returns `space` plus `Option<tab>`. Tab *defaulting* (the caller's tab, or the sole tab) stays in `resolve_selector`'s `--pane` path only; seat narrowing never defaults a tab.

**`handoff_delivery` existing-peer path** — extended: it receives an already-frozen seat ID in `request.seat`, and no longer rejects `--seat` with `--space`/`--tab`. Its `--pane` path is unchanged.

Variation:
- Seat-argument positions grow as commands gain seat arguments. `SeatSlot` absorbs them: a closed enum whose two `match` arms are the one place a slot's kind is chosen, registered where `commands.rs` collects `seat_selectors`.
- Reading shapes and tier rules are fixed by D3, in `seat_selector`'s single readings table and its `Tier` enum.
- Level matching (label, agent name, ID) has one matcher per level in `seat_selector`.
- Pane-to-seat policy is fixed at two kinds, `SeatMapping`.

Walk-through: `send --seat tryout/alice --body hi`.
1. `parse` records `SeatSlot::DirectTarget("tryout/alice")` and leaves a placeholder target.
2. The thread and pane stages run.
3. `resolve_cli_seats` misses on `SeatInspect`; the last segment is not a seat ID, and the string is not a pane ID.
4. One topology/caller read, then `seat_selector` pools the space/tab, tab/pane and space/pane readings: tier `space` holds one pane, `w4:p7`.
5. `SeatMapping::Allocate` goes through the shared journaled `ResolveSeat`, which returns `sAb12Cd34`.
6. The `SeatSlot` setter freezes the target and the direct-ACK check passes. stderr gets `resolved: project/tryout/alice w4:p7 seat=sAb12Cd34 (tier: space)`.
7. The intent journals only the ID, so `retry` never re-resolves.

Future change: `--seat` on `history` touches one new `SeatSlot` variant (field write + `Lookup`) and one registration in `commands.rs`; the stage and the resolver do not change.

Per spec D1, "Key decisions made": the stage lives in the ~4.3k-line `src/cli/mod.rs`. That is accepted, and it stays thin because matching, slot kinds and mapping class live in their own types. A task may still move it into a sibling file (`src/cli/seat_resolution.rs`) to keep `mod.rs` edits small.
Per spec D5: raw strings live in `ParsedCli`, so action fields carry placeholders until freezing. the stage must refuse leftovers at its end, and the stage runs before caller defaulting.
The spec's sketch of a pure resolver returning `Conflict` is refined. Candidates need daemon seat IDs, so the resolver returns a typed ambiguity, and the error is rendered after the stage supplies the seat-lookup closure.

Digest:
- seat_selector: owns readings, per-level matching, narrowing, pooling, Tier ranking, nickname verification and the bounded path/conflict text; new reading or matching behaviour goes here; callers never match labels or format candidate paths themselves.
- SeatSlot: owns where each seat argument lives in an action and its D4 mapping class; a new seat-taking argument is a new variant registered in parse; callers never match action kinds to write a seat or choose allocate versus lookup.
- SeatMapping: owns the lookup-or-allocate decision as a value; a new class would be added here (the spec commits to none); callers never allocate for a Lookup slot.
- resolve_cli_seats: owns the D2 order, the lazy host/daemon reads, freezing, post-freeze ID checks and the resolved: lines; new seat-argument forms go into its classification; downstream code never sees a raw seat string and retry never calls it.

## Post-Implementation Notes

> *As this design is implemented and iterated on — bug fixes, adjustments, anything that diverged from the assumptions above — append a dated note here, whether or not a formal debugging skill was used.*
