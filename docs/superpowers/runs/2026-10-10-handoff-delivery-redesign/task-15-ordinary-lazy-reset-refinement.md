# Task15 ordinary-Lazy signal reset refinement

Narrow DESIGN checkpoint; fresh exact-delta interface review and parent final authorization precede production edits. This qualifies ONLY reset/ownership wording in ordinary-Lazy proposal48daad34d8b371ec8781404e4f6e4ddac788f01f0d976946e3a66028391a5cb7. That artifact/chronology remains intact. Its instruction for Refresh to normalize through binding-aware load is SUPERSEDED: Refresh MUST retain load_raw and the governing M invariant that passive maintenance never resets presentation state. No new fields/schema/algorithm/work budget are added here.

## Existing two owners and exact initialization predicate

D invitation_content.rs load_raw reads current private row. load is the presentation binding-reset site: a presentation generation/execution/instance mismatch constructs fresh presentation state, preserves monotonic control epoch counter and existing mod-maintenance fields. refresh uses load_raw and its separate mod_generation/mod_execution mismatch branch; that branch resets only existing mod-maintenance cursors/header. Actual signal extension adds four counters already selected in48: ordinary_publication_after/high, ordinary_passive_generation, ordinary_scan_passive_generation.

Compute matches from the ORIGINAL loaded row BEFORE changing EITHER identity header:

`same_instance = state.instance_id == claim.instance`

`presentation_matches = same_instance && state.generation == admitted_generation && state.execution == claim.execution`

`mod_matches = same_instance && state.mod_generation == admitted_generation && state.mod_execution == claim.execution`

`signal_initialized_for_claim = presentation_matches || mod_matches`.

Only when BOTH matches are false may the first admitted owner zero all FOUR signal counters. Existing seat/consumer row key and canonical scope/admission bind the instance and native session; no duplicate execution/native-ID fields. A repaired/foreign-instance mod header does not count as initialization. Existing empty-row initialization remains its established behavior; it cannot reset nonexistent presentation state or carry old signal.

## Refresh rule

Refresh remains load_raw. In addition to canonical binding admission, a NONEMPTY private row with state.instance_id != claim.instance is explicitly corrupt: REFUSE before any maintenance/header/counter/save effect. Do not install a mod header while preserving a foreign instance header, which would repeat initialization forever. Existing empty-row initialization is unchanged. For a nonempty same-instance row, validate current binding and compute the predicates above before modifying mod_generation/mod_execution. If its mod header mismatches, it runs the EXISTING mod-only reset exactly as before; it additionally zeroes the four signal counters IFF signal_initialized_for_claim was false. Then it installs the existing admitted mod_generation/mod_execution header and advances bounded signal maintenance. If presentation header already matches, preserve all four counters even though mod cursors/header require their first transition.

For every existing nonempty row, Refresh preserves presentation instance/generation/execution, outbox/token/frozen descriptor identities+masks, control episode/counter, ordinary cursor/version/done/body/carry/partial, five-way output rotation, and all other presentation-owned fields EXACTLY. The fourth NEW signal counter may be initialized on a genuinely new signal scope; it is not permission to reset the preexisting ordinary walk. No call to binding-aware load, reset of presentation state or copied fresh State may be hidden in Refresh. Refresh outputs only passive progress; actual model output remains separately admitted Pull.

## Presentation reset rule

Actual presentation Pull keeps binding-aware load. If presentation header already matches, preserve signal normally. If presentation binding resets, use ORIGINAL same-instance mod_matches to choose signal values: matching mod header means COPY all four signal counters into the fresh presentation State; otherwise fresh defaults zero them. Preserve existing mod-maintenance fields as load currently does. Then install admitted presentation identity as usual and reset ONLY the existing presentation walk/body/carry/outbox lifecycle under its established rules. At the START of the newly created ordinary walk freeze latest native/passive tuple; never treat copied old ordinary_scan_passive_generation as an already-consumed new walk. No old body/carry/token is emitted under the new binding.

Confirmation may invoke load transiently for scope checks, but NotCurrentPage/StaleBinding/invalid-stamp refusals save NOTHING; a transient fresh value cannot durably initialize a signal scope or reset private fields. This refinement does not authorize confirmation-as-presentation-reset. Hook has no ModRefresh: its first real presentation Pull initializes/resets the four counters through the presentation owner alone.

## No double reset or lost positive generation

Refresh-first Watch transition: neither old identity matches => reset four once, install current mod header, inspect finite manifest range and possibly increment passive generation. Later new-binding Pull sees mod_matches=true; fresh presentation reset copies the four values, preserving positive generation/frontier. New ordinary capture freezes current latest generation and queries canonical pending content.

Pull-first Watch transition: neither old identity matches => presentation reset defaults four once and installs current presentation header; Pull may advance them and start its ordinary walk. Later Refresh sees presentation_matches=true BEFORE updating its stale mod header, so it preserves all four and only resets existing mod cursors/header. It cannot erase positive generation, consumed frontier or active ordinary scan signal.

After either first transition at least ONE header matches the admitted current binding. Repeated calls by either owner do not reset the signal. Once both have transitioned, both match. Every manifest is therefore inspected at most once per fixed consumer/binding by this shared signal walk, not once per owner. No additional prefix replay or duplicate generation is introduced by owner ordering. Restart preserves both headers/counters. A genuinely later binding may restart prefix exactly once; the original48 finite historical-prefix latency and no-every-transient promise apply. Constant SQL/state writes are in the deciding transaction; rollback installs neither header nor cursor, so a retry cannot falsely believe initialization committed. Existing canonical writer serialization/owner admission must prevent stale read-save overwriting the other owner's committed header/counters; fresh review verifies this actual path.

All pending already-published addressed Lazy rows remain discoverable: first owner scans retained publication prefix; second owner preserves that progress; fresh presentation walk always starts with canonical pending query, even if Refresh consumed an earlier signal before Pull began. Grace/registration never requires a new addressed arrival to bootstrap maintenance; original more/worker safety continuation remains. Unrelated publications still only advance bounded signal cursor, never passive output/version/native/stall state. A pending row displayed in between can be skipped without lost remaining content. Neither old-binding signal nor old outbox/body is authority for output; current stream/binding/lease admission and closing priority remain unchanged.

## Exact review/regression delta

Prove Refresh-first and Pull-first transitions with positive signal/frontier already advanced, current/old mod+presentation identity combinations, same-instance requirement, same generation/different execution, restart between owners, and repeated unchanged calls. Assert Refresh leaves existing active outbox/token/masks/control epoch/ordinary body+cursor+carry byte-for-byte unchanged; assert subsequent genuine Pull resets old presentation state while preserving already-initialized four counters. Existing governing M no-presentation-reset regression must stay GREEN.

Test first old/new binding with pending Lazy publication before registration, maintenance during reconnect grace then actual admitted new Watch Pull; no lost generation, no old-binding output, no redundant within-binding prefix replay/second reset. Hook initializes once without mod header. Invalid/NotCurrentPage confirmation changes no durable reset/header. Every new ordinary traversal snapshots latest native/passive generation at START after its own presentation reset. Actual visit bounds remain additional100 manifests/200 keyed probes/3seeks, refresh160/pull433 and unchanged155731 carry; this correction adds no schema fields, source walk, query or output item.

Read-only coordination: interface_review_13 source audit agreed with the exact pre-update OR invariant and noted confirmation no-save refusals. No source/Cargo tests were run by planner. Source author retains implementation and proof; this artifact makes no GREEN/implementation-authorization claim.
