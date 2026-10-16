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
     returns the resolved space and tab IDs, using today's rule;
   - the guarded `allocate` closure in `resolve_recipient_seats` into a named function in
     `mod.rs`.

   Behaviour is unchanged. Nothing else waits for this leaf except the stage.
2. **The stage core plus the parser flip, together.** This leaf adds:
   - `src/cli/seat_resolution.rs` with `resolve_cli_seats`: the D2 order, the lazy
     topology/caller reads, narrowing through the helper, conflict seat annotation, the
     `SeatMapping` lookup/allocate dispatch, freezing through `SeatSlot`, and the `resolved:`
     stderr lines;
   - the call site in `run_parsed_in_pane`: after `resolve_cli_threads`/`resolve_cli_targets`,
     before `derive_caller`/`resolve_recipient_seats`, never on `retry`;
   - the dispatch guard refusing leftover seat selectors;
   - in `commands.rs`, switching non-ID strings to placeholders, relaxing `--seat` versus
     `--space/--tab` (D7) and adding the D7 help line.
3. **Command-path integration.** This leaf covers:
   - the post-freeze direct-ACK and duplicate-recipient checks (reusing `ht-u3ym.2`'s exposed
     functions), with the parse-time copies removed for slots that now resolve at runtime;
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

**Chosen:** leaf 3. They only matter once names can produce IDs that differ textually from what
was typed. Leaf 2 keeps the existing parse-time checks for valid-ID strings, which still runs
them for exact IDs. Leaf 3 moves them after freezing for every slot.

**Considered:** leaf 2. Rejected to keep leaf 2 reviewable. Leaf 2 works without them, and the
only exposure is a name and an ID naming the same seat in one command, which leaf 3 closes.

### S3. Leaf tests

- **Leaf 1:** behaviour-preserving, so the existing suite is enough.
- **Leaf 2:** the stage cases the root bead listed, through the `tests/cli/cooperative.rs`
  `Runtime` fixture:
  - a name sent to a direct send allocates, with the `resolved:` line on stderr;
  - an exact ID works with the host down;
  - a name with the host down fails `host_unavailable`;
  - a read writes no `ResolveSeat` intent;
  - a leftover selector is refused by the guard.
- **Leaf 3:**
  - a direct ACK by name and by ID for the same seat deduplicates;
  - a direct ACK naming another seat is refused;
  - `handoff --existing --seat NAME --tab T` reaches delivery;
  - `seat inspect NAME` returns the mapped seat.

The full command matrix stays in `ht-u3ym.4`.

## Key abstractions

No new abstractions; extends resolve_cli_seats, SeatSlot, PaneSelector/panes.rs, handoff_delivery existing-peer path.

## Post-Implementation Notes

> *As this design is implemented and iterated on — bug fixes, adjustments, anything that diverged from the assumptions above — append a dated note here, whether or not a formal debugging skill was used.*
