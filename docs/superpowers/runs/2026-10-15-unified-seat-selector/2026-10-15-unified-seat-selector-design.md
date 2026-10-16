# Unified seat selector — design

Run: [2026-10-15-unified-seat-selector](run.md) · branch `super-auto/unified-seat-selector`

## Goal

Every CLI argument that names a seat accepts an exact seat ID or a name: a pane, tab or space
label, a live agent name, or a `/`-joined path of them. The CLI resolves the name to exactly
one seat, or fails and lists every candidate. Nearer panes win ties: the caller's tab, then
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
person who sees `project/alice/sAb12Cd34` in `read` output cannot paste it back. They must look
up an opaque ID first.

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

   This is how `read` output (`SPACE/PANE/SEAT_ID`, labels replaced by host IDs when absent) can
   be pasted back.
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

**Narrowing.** `--space`/`--tab`, when given alongside `--seat`, are resolved as today (exact ID
or unique label, `unique()` in `panes.rs`). Every candidate outside the narrowed space or tab is
dropped before tiering.

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

**Conflict detail.** It lists at most 8 candidates, with an `N omitted` count. Each candidate
gives:
- the reading that matched (`tab=tryout pane=alice`);
- its `space/tab/pane` path, each part a label or the host ID when there is no label, escaped
  with `{:?}`, bounded like today's `describe`;
- its pane ID;
- its seat ID, `seat=none` when the pane has no resolved seat.

The detail ends with the qualification hint: a longer path, `--space/--tab`, or an exact seat or
pane ID. Looking up candidate seats takes at most 8 bounded `collect_pane_seats` reads. A failed
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

The path parts are labels, or host IDs when there is no label, each escaped and bounded like
conflict candidates. A seat allocated by the guarded path prints its new ID. **Size:** 40–100
bytes per line for typical labels (at most about 3 × 64-byte bounded parts plus IDs). Static
explanation stays in the skill (AGENTS.md token-efficiency rules (a), (b), (d), (f)).

### D7. Clap surface

All seat argument fields stay `String` and gain one help line: "SEAT: an exact seat ID, or a name
path SPACE/TAB/PANE (any part may be omitted)". `--seat` stops conflicting with `--space`/`--tab`
on the commands that flatten `PaneSelector`. It still conflicts with `--pane`, because a pane
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

### D10. Tests

- **Pure resolver unit tests** over hand-built `HostTopology`, covering:
  - every row of the D3 readings table, and the whole-string label containing `/`;
  - pane ID, tab ID and space ID segments;
  - agent-name match;
  - sole-pane and sole-tab aliases;
  - tier precedence (same tab beats same space beats any), and no caller (all tier `any`);
  - narrowing;
  - conflict listing and its bounds (more than 8 candidates, escaping);
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
- Owns: splitting a selector into readings (whole-string reading + the D3 segment table, with empty and ≥4 segments yielding nothing); per-level matching (space label|ID, tab label|ID, pane label|agent name|ID) and the sole-pane / sole-tab aliases; narrowing by already-resolved space/tab IDs; pooling and deduplicating by pane ID while keeping each pane's matched readings; ranking by `Tier` (closed enum `Tab | Space | Any`) and choosing the first non-empty tier; nickname-prefix verification (the same pool with tiering off, plus a membership test against the seat's mapped target); one path renderer (`space/tab/pane` as labels, else host IDs, `{:?}`-escaped and bounded) used by conflict candidates, the `resolved:` line and the nickname-mismatch message.
- Hides: which reading matched, the readings table, tier precedence, and the bounded and escaped wire text (8 candidates, `N omitted`, the qualification hint).
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
- Relationships: the allocation is extracted once from `resolve_recipient_seats`'s `allocate` closure into a helper in `mod.rs`, shared by the `--pane`/`--require-ack-pane` path and this stage, so two callers exist now.

**`resolve_cli_seats`** — new runtime stage in `src/cli/mod.rs` (spec D1), the only code that turns a `SeatSlot` raw string into a frozen `SeatId`.
- Owns:
  - the D2 order per slot: exact ID via `SeatInspect`, then the nickname form, then a direct pane ID, then a name;
  - a private classification (exact | nickname | pane ID | name) that decides whether mapping is skipped and whether a `resolved:` line prints;
  - the lazy, once-per-invocation topology and live-caller reads, shared across all slots like `resolve_cli_targets`;
  - resolving the `--space`/`--tab` narrowing once;
  - seat annotation of conflict candidates (at most 8 bounded lookups);
  - mapping through `SeatMapping` and freezing through `SeatSlot`;
  - the post-freeze ID checks moved out of parse time (D7: direct ACK names only the target; no duplicate recipients after dedup);
  - clearing the narrowing flags it consumed, so handoff's "seat with space/tab" refusal (`handoff_delivery.rs` ~238/347) does not fire;
  - writing the `resolved:` lines to stderr before command output (D6).
- Hides: when the host or daemon is touched. An exact ID never reads topology, so it works with Herdr down.
- Must not: match names (`seat_selector` does), know command kinds (`SeatSlot` does), write any journal intent other than the existing allocation, or run on `retry`.
- Relationships: it calls `seat_selector`, `SeatSlot`, the shared allocate helper, `pane_seat` and `SeatInspect` through the lazy connection. It runs after `resolve_cli_threads`/`resolve_cli_targets` and before `derive_caller`, `resolve_recipient_seats` and dispatch; otherwise a read with a placeholder seat could be defaulted to the caller. Direct-send errors pass through the same `direct_send::pre_executor_error` seam as pane resolution.
- Lifecycle: one call per invocation; it owns nothing afterwards.
- Errors: `HostUnavailable` (topology read, only for the nickname and name forms); `not_found`/`conflict` (resolver); `invalid_request` (`no_seat_for_pane`, for reads and operator commands); `mapping_error` (a pane with several seats); allocation retry failures as today. No new codes; callers distinguish by `ErrorCode`.

**`ParsedCli`** — extended: gains `seat_selectors` (a list of `SeatSlot` + raw string). Action fields hold a placeholder `SeatId` until the stage freezes them, as `pending-pane-selector` does. `dispatch`'s guard refuses a leftover seat selector, as it does for pane and thread selectors. Parse-time `SeatId` comparisons move into the stage.

**`PaneSelector` / `panes.rs`** — extended: `resolve_selector`'s hard-wired `--space`/`--tab` parent resolution (exact ID or unique label via `unique()`) is extracted into one helper that `resolve_selector` and the seat stage's narrowing both call, so the flag families cannot drift. That helper keeps today's rule: `--tab LABEL` without `--space` is scoped to the caller's space, and outside Herdr it needs `--space` or a tab ID.

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
Per spec D5: raw strings live in `ParsedCli`, so action fields carry placeholders until freezing. `dispatch`'s guard must refuse leftovers, and the stage runs before caller defaulting.
The spec's sketch of a pure resolver returning `Conflict` is refined. Candidates need daemon seat IDs, so the resolver returns a typed ambiguity, and the error is rendered after the stage supplies the seat-lookup closure.

Digest:
- seat_selector: owns readings, per-level matching, narrowing, pooling, Tier ranking, nickname verification and the bounded path/conflict text; new reading or matching behaviour goes here; callers never match labels or format candidate paths themselves.
- SeatSlot: owns where each seat argument lives in an action and its D4 mapping class; a new seat-taking argument is a new variant registered in parse; callers never match action kinds to write a seat or choose allocate versus lookup.
- SeatMapping: owns the lookup-or-allocate decision as a value; a new class would be added here (the spec commits to none); callers never allocate for a Lookup slot.
- resolve_cli_seats: owns the D2 order, the lazy host/daemon reads, freezing, post-freeze ID checks and the resolved: lines; new seat-argument forms go into its classification; downstream code never sees a raw seat string and retry never calls it.

## Post-Implementation Notes

> *As this design is implemented and iterated on — bug fixes, adjustments, anything that diverged from the assumptions above — append a dated note here, whether or not a formal debugging skill was used.*
