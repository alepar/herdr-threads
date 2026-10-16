R1 Every seat argument (inbox/pending-receipts/diagnostics/warnings/thread list --seat, invite --seat, send --require-ack, send --seat direct, handoff --existing --seat, seat inspect/retire/rebind SEAT, rebind --replace) still accepts an exact seat ID with unchanged behaviour and no Herdr read.
R2 The same arguments accept a name: whole-string label, or 1-3 '/'-separated segments read as every space/tab/pane assignment (with skipping), matching labels, IDs and live agent names, with sole-pane-tab and sole-tab-space aliases.
R3 Candidates are pooled and tiered by proximity to the caller (same tab, same space, anywhere); one pane in the first non-empty tier resolves; several -> conflict listing bounded escaped candidates (reading, path, pane ID, seat); none -> not_found; host unreachable -> host_unavailable, never passing a name through.
R4 The read-output nickname form PREFIX/SEAT_ID resolves to that seat only when the prefix matches its live location, else conflict.
R5 --space/--tab narrow a --seat name; --seat still conflicts with --pane.
R6 Reads and seat-repair/handoff-existing lookups never allocate; invite, ACK recipients and direct send allocate through ordinary guarded resolution as the pane forms do.
R7 The resolved seat ID is frozen before any journal intent or dispatch; retry never re-resolves.
R8 A name resolution prints exactly one compact `resolved:` line on stderr; exact IDs print nothing extra.
R9 --pane and --require-ack-pane keep their current behaviour.
R10 docs/agent-usage.md, the skill and TRUST-POLICY.md describe seat selectors consistently with the behaviour.
R11 Tests exercise every seat-taking command with a name and with an exact ID.
R12 Every CLI argument whose value is a seat is routed through a SeatSlot (or on an explicit, reasoned exclusion list), and a test keeps it that way.
R13 An ambiguous name's error lists every matching candidate (a safety cap of 32 with an omitted count only for pathological topologies).
R14 --space/--tab given with an exact seat ID or a nickname is defined: the seat's live location must lie inside them, else conflict; without narrowing no Herdr read happens.
R15 The resolved: line (and each conflict candidate) shows the full live space/tab/pane path, whatever reading or alias matched.
