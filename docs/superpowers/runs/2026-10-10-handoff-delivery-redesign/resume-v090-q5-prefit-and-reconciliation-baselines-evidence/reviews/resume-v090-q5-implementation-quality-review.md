# Q5 continuation implementation — REVISE one semantic-validation gap

2026-10-11. Fresh static review of ignored `resume-v090-q5-continuation-implementation.patch`, SHA-256 `94c8dcee5522a94d3af5a3feb0b7c0899af6095ec1dfc4031827e72642bb230c`, against approved design `19a6a008fdd9d755869182086e3a68f73fcb33f025747c4197c7fbf637091e80` and its accepted routing/shadow amendments. Parent identifies frozen candidate68 with gates passing; no runtime active. Read actual parser, argv generators, renderer/final writer and patch. No candidate/source/test edits, Cargo/runtime, Git/Herdr writes or nested agents. Only this report was written. No formal roast or counter change.

**Verdict: REVISE Q5-IQ1.** Core budget restoration, genuine routing conflicts, default omission, refit termination and final-byte enforcement pass static review. One remaining gap permits malformed genuine typed/global options to normalize because shadow clap validation bypasses the semantic parser layer.

## Q5-IQ1 — Shadow structure validation loses semantic checks for accepted options

Patch `read_continuation_byte_slot` strips the immediate human actor at patch lines 37–39, shadows string option values, and validates with `commands::command_for_registry(...).try_get_matches_from(shadow)` at lines 162–164. It does not run `parse_cli_in_registry` or the actor checks in `parse_argv_in_registry`. The helper nevertheless explicitly accepts cooperative-seat/target/harness/role, agent-id and Search thread options. The real CLI performs several necessary checks only after clap:

- `commands.rs:1962–1998` requires the entire cooperative selector group, validates the actual seat/agent IDs and checks the named harness against the registry.
- `commands.rs:1814–1826` rejects human namespace mixed with cooperative selectors or agent-id.
- Search typed conversion at `commands.rs:2522–2527` validates the actual optional thread ID.

Concrete static counterexample: a reduced Message result with typed next cursor `opaque` and argv `herdr-threads human body m1 --cursor opaque --max-bytes 1024 --cooperative-seat s` can pass the helper's shadow clap structure and be rewritten to 2048. The real parser rejects the incomplete selector group and its human/cooperative combination. Similarly, Search `--thread` with an empty/invalid value is shadowed to a harmless string without raw validation, so the helper can accept a genuine malformed filter option. These are not the explicitly retained leading-hyphen Search-literal generator limitation.

**Minimal correction:** preserve the recognized actor for validation and validate semantic/raw typed fields before shadowing them, preferably by reusing the pure existing parser with an actor-preserving shadow plus narrowly necessary raw checks. For Search, retain/validate the real thread selector and its relation to the typed request rather than sanitizing it unconditionally. Alternatively reject cooperative fields that are outside the supported service-generated continuation forms instead of claiming to validate them. Whichever narrow strategy is chosen, do not introduce a broad new CLI grammar or decode opaque cursors. Continue preserving the approved inherited option-looking selector/route data behavior.

Add no-output/no-mutation negatives for incomplete cooperative groups, human/cooperative and human/agent-id combinations, invalid harness/IDs where supported, and malformed actual Search thread values. Keep valid supported actor/routing/data cases byte-identical except the budget field. This is a malformed-response contract failure, not new caller-verification or authority hardening. No execution was performed; the counterexamples follow directly from the inspected parser control flow.

## Paths that pass static review

**True budget slot and raw routing.** The scanner consumes recognized option values before interpreting their bytes, so a route value `--`, `human` or `--max-bytes` cannot itself become a delimiter/option. Raw state-dir/host values are checked for empty/length/NUL and compared per flag before shadowing. Identical repeats survive, distinct repeats fail across leading/trailing or inline forms. Real numeric budgets must be unique, parse as u32, equal E and lie within 256..MAX_PAGE_BYTES. Actor removal offsets are restored when mapping the final write slot into the original argv. Numeric limits are checked against the request. Unknown genuine flags and clap-structural conflicts remain rejected.

The two interpretations handle a known generated leading option-looking selector versus actual delimiter placement. Only one unambiguous budget slot may survive; no mutation occurs until both attempts finish. The actual selector equals the typed command selector. Main opaque cursor equality is checked against the typed result without decoding, and body offset-form continuations retain existing parser offset/cursor conflict rules. The helper changes only the recognized value/inline budget token, or inserts a budget before the true delimiter/end for legitimate default-omitting History. Other raw tokens remain byte-identical. Body/Search require an explicit byte option. History omission is allowed only when E is the real default; body/system/no-next and unrelated typed result forms are untouched as designed.

**Refit loop.** M is captured once. Every subsequent reduced response is normalized before mode lookup and exact rendering. On overflow only E changes; the command's original body position, page cursor, selectors and row limit remain fixed. The next daemon response replaces the old selection and modes, so there is no local row/body trimming or accidental use of a previous continuation. With positive overflow, `max(overflow,current/4)` is positive, making E strictly decrease until success or the existing minimum-budget error. No arbitrary retry bound or normalized-result equality escape is added.

**Reselection to no Lazy rows and final decorations.** A refitted response cannot take the old modes-empty early return. Its normalized bytes are still measured against M, and `require_final_bound` persists enforcement in the mode snapshot. The final `ReadModes::write` checks `(enforce_final_bound || !is_empty())` after annotation and after run_wire_result's optional decoration stage, before the external writer sees bytes. Thus losing all Lazy rows during refit does not lose the final bound. Initial never-refitted JSON/no-mode behavior remains unchanged. The returned typed response is the normalized response that the final renderer consumes; there is no later fresh mode lookup.

**Failure atomicity and limited scope.** Slot validation precedes argv mutation; all fitting is completed before external output. Normalization/render/budget errors return before body cursor display effects. The writer still performs write_all and flush before the caller can issue AdvanceCursors; write/flush failures preserve existing no-display behavior. No canonical store mutation, wire/schema field, body/cursor fabrication or user-budget increase is introduced. The public error is the short fixed `invalid read continuation budget` (32 UTF-8 bytes), with no body/argv disclosure.

The patch does not repair the inherited Search generator's bare leading-hyphen literal continuation, as explicitly agreed. A shadow-slot PASS does not claim that such an inherited argv is executable. Real follow-through tests must use followable generated forms, while separate token robustness tests prove preservation without overstating broader generator behavior.

## Required evidence and exact snapshots

After Q5-IQ1 is resolved, run the unchanged actual 65536-byte/2048-byte multibyte body continuation control and focused History/Search refit controls. Include genuine default omission, multiple pages, row/cursor/filter/routing preservation, mixed/no-Lazy reselection, restored-digit/annotation/late-decoration bytes, malformed-option zero output/effects, insufficient original budget and write/flush failure. This static review neither compiles the patch nor supplies runtime or physical-byte results.

Reviewed base source SHA-256 (candidate files were only read):

- `src/cli/mod.rs`: `2bf8fa203b76a738a21b2cab069f18b3ffbe274e3c9793d7cd15ad70b5dee4e8`
- `src/cli/output.rs`: `85309d3a32cea8c6b7f21324a1323eda74e07c32b234f2afae6b3fbe698cbe01`

Only the two paths above are changed by the reviewed patch. No other material issue found; Q5-IQ1 remains the sole requested revision.


## Successor patch rereview — scoped static PASS

2026-10-11. Read the whole successor scratch patch and verified SHA-256 `8f4fcd987732e25852b2d9305bd34eb097b8051d43657d2786e3ee3e030e5cde`. Compared its changes with actual Message/History/Search/contextual argv producers and the approved amended strategy. Parent identifies candidatee12 unchanged. The two candidate base source hashes remain exactly those recorded above. No patch application, candidate/source/test edits, Cargo/runtime, Git or Herdr writes, or nested agents. Initial REVISE of `94c8dcee...` is preserved.

**Q5-IQ1 is resolved.** Genuine cooperative-* and agent-id options are no longer admitted or shadowed; they fail the limited generated-option whitelist before parser validation or argv mutation. Actual continuation producers emit routing/output globals and optional human namespace, not those identity selectors, so this narrow rejection avoids bypassing their richer semantic admission rules without changing the general CLI parser. A selector or consumed routing value whose bytes merely spell one of those flags remains data under the approved position-aware handling.

Search `--thread` is now accepted only for a Search request that actually has a thread, exactly once, with an original value equal to the typed request's thread and accepted by ThreadId::parse. The final presence check also rejects omission of a required thread filter. Empty, invalid, mismatched, unexpected and duplicate thread values cannot be laundered through the shadow literal. This retains the real generated producer shape and preserves the original raw token on success.

The unaffected whole-patch logic still passes static review: raw route duplicates compare before shadowing; consumed `--` remains data; output/numeric conflicts use the existing grammar; two selector-placement interpretations must agree on one budget slot; default omission is restricted to default-effective History; body offset compatibility remains without cursor decoding; only the actual budget slot changes after successful validation. Typed result fields and all other argv tokens remain untouched. The inherited Search literal escaping limitation remains expressly outside this repair.

Normalization still precedes each refitted render/mode lookup. Effective E decreases strictly without changing the original read position/row limit or increasing M. A reselected empty mode set cannot bypass fitting, and the retained final-bound flag enforces actual final bytes after decoration before external output. Existing nonrefitted behavior and flush-before-display ordering remain unchanged. No new material issue was found on this whole successor review.

**Verdict: PASS for the exact successor patch's scoped static quality review.** Compilation, the actual 2048-byte long-body continuation witness, History/Search/route/malformed/no-mode controls and final byte measurements remain separate parent-owned gates; no runtime GREEN or integration clearance is claimed here.
