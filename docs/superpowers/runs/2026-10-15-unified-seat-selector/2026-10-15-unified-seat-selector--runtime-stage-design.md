# Unified seat selector — runtime resolution stage

Parent spec: [2026-10-15-unified-seat-selector-design.md](2026-10-15-unified-seat-selector-design.md) · bead `ht-u3ym.3` · root epic `ht-u3ym`

## Goal

A seat argument the parser recorded as a `SeatSlot` reaches dispatch, journals and retry records
only as one frozen canonical `SeatId`:
- an exact ID never touches Herdr;
- the nickname form is verified;
- a name is resolved through `seat_selector` and mapped to a seat, by lookup or by guarded
  allocation (`SeatMapping`), and announced on stderr;
- every command path (reads, invite, send, direct send, handoff `--existing`, seat repair)
  accepts the frozen ID, and no leftover raw seat string can dispatch.

## Problem description

The root spec (D1, D2, D4–D8 and the Key abstractions entry `resolve_cli_seats`) decides
*what* the stage does. The promotion review found it too large for one bead: it bundles two
helper extractions, the classification and resolution core, the parser flip from inert recording
to placeholders, and per-command integration (direct send, handoff, post-freeze checks). This
spec only splits that work into reviewable leaves and fixes the order between them. It makes no
new product decision.

## Main challenges

- **Keeping each leaf green.** Before this subtree, `ht-u3ym.2` records slots inertly: names are
  still rejected at parse, and the clap conflicts stay. The flip to placeholders must land in the
  same leaf as the dispatch guard and the stage, or a name could dispatch with a placeholder
  seat.
- **Hot file.** `src/cli/mod.rs` is about 4.3k lines. The stage goes into a sibling file,
  `src/cli/seat_resolution.rs`, so leaves touching `mod.rs` keep their hunks small.

## Key decisions made

There are three leaves.

1. **Move-only helper extraction.** This leaf extracts:
   - the `--space`/`--tab` parent resolution from `panes::resolve_selector` into a helper that
     returns the resolved space plus an optional tab. It does no tab defaulting: the caller's tab
     or sole-tab defaulting stays inside `resolve_selector`'s `--pane` path (root D3 Narrowing,
     Key abstractions `PaneSelector`);
   - the guarded `allocate` closure in `resolve_recipient_seats` into a named function in
     `mod.rs` that carries its refuse-before-allocate preconditions: the agent-without-session
     refusal, the p0 send-time receipts capability and the invitation-confirmation capability
     (root Key abstractions `SeatMapping`).

   Behaviour is unchanged. Nothing else waits for this leaf except the stage.
2. **The stage core plus the parser flip, together.** This leaf adds:
   - `src/cli/seat_resolution.rs` with `resolve_cli_seats`: the D2 order, the lazy
     topology/caller reads, narrowing through the helper, conflict seat annotation, the
     `SeatMapping` lookup/allocate dispatch, freezing through `SeatSlot`, and the `resolved:`
     stderr lines;
   - the call site in `run_parsed_in_pane`: after `resolve_cli_threads`/`resolve_cli_targets`,
     before `derive_caller`/`resolve_recipient_seats`, never on `retry`;
   - the dispatch guard refusing leftover seat selectors;
   - in `commands.rs`:
     - **every** seat-argument string becomes a slot with a placeholder action value, because
       `SeatId::parse` accepts nearly any ASCII string, so there is no syntactic ID/name split;
     - `--space`/`--tab` given with `--seat` go to `SeatNarrowing`, never to `pane_selector`
       (root D3);
     - relax `--seat` versus `--space/--tab` (D7) and add the D7 help line;
   - the post-freeze direct-ACK ("names only the target") and duplicate-recipient checks over
     frozen IDs. The parse-time comparisons are removed in the same change, so no window exists
     where a name-form recipient is unchecked.
3. **Command-path integration.** This leaf covers:
   - lifting the direct-send `--space/--tab` refusal (`--require-ack-pane` stays refused), with
     errors routed through `direct_send::pre_executor_error`;
   - `handoff --existing --seat`: consuming the frozen seat and clearing the narrowing flags so
     `handoff_delivery.rs`'s seat-with-space/tab refusal does not fire;
   - checking that the seat-repair commands (`inspect`/`retire`/`rebind`/`--replace`) receive
     frozen IDs.

## Decision points

### S1. Where the parser flip lands

**Chosen:** in leaf 2, with the stage and the guard. Landing it earlier (`ht-u3ym.2`) opens a
window where names dispatch unresolved. Landing it later (leaf 3) leaves the stage with nothing
to resolve but valid IDs, so its tests could not exercise names.

### S2. Where the post-freeze checks live

**Chosen:** leaf 2, in the same change that turns every seat string into a placeholder slot.
Every string becomes a placeholder because `SeatId::parse` cannot tell names from IDs, so
parse-time comparisons over placeholders would be meaningless. Without the post-freeze checks,
a leaf-2 build would accept `send --seat bob --require-ack carol` with two different seats, and
would mis-handle a name and an ID naming the same seat.

**Considered:** leaf 3, as first drafted. Rejected (roast r1): it left an unchecked window and
an undefined syntactic split.

### S3. Leaf tests

- **Leaf 1:** behaviour-preserving, so the existing suite is enough.
- **Leaf 2:** the stage cases the root bead listed, through the `tests/cli/cooperative.rs`
  `Runtime` fixture:
  - a name sent to a direct send allocates, with the `resolved:` line on stderr;
  - an exact ID works with the host down;
  - a name with the host down fails `host_unavailable`;
  - a read writes no `ResolveSeat` intent;
  - a leftover selector is refused by the guard;
  - a direct ACK by name and by ID for the same seat deduplicates;
  - a direct ACK naming another seat is refused;
  - `--seat X --space S` on a multi-tab space searches every tab of `S`;
  - a refused command (an agent without a session, or a p0 send on a daemon without the
    capability) journals no `ResolveSeat`.
- **Leaf 3:**
  - `handoff --existing --seat NAME --tab T` reaches delivery;
  - `seat inspect NAME` returns the mapped seat.

The full command matrix stays in `ht-u3ym.4`.

## Key abstractions

No new abstractions; extends resolve_cli_seats (placed in the sibling `src/cli/seat_resolution.rs`, which the frame allows, with one call site in `run_parsed_in_pane` after `resolve_cli_threads`, so it runs before both the `--operator` branch's `resolve_recipient_seats` at mod.rs ~797 and `run_caller_scoped`), SeatSlot, ParsedCli (dispatch guard), PaneSelector/panes.rs (the parent-narrowing helper extracted from `resolve_selector`), the shared allocation helper (extracted from the `allocate` closure in `resolve_recipient_seats`), direct_send::pre_executor_error, and the handoff_delivery existing-peer path.

## Post-Implementation Notes

> *As this design is implemented and iterated on — bug fixes, adjustments, anything that diverged from the assumptions above — append a dated note here, whether or not a formal debugging skill was used.*
