# Final scripted native compatibility

The authorized final compatibility selection passed **12/12** in one run on
candidate `a8aef2df…`, built from `a19fd881e`. Runtime source was `fb7bf028e`,
driver `50a3f9fb…`, fixture `08174d10…`, and retained official runtime entry
`e79626f2…`. Total wall time was **311.65s**. Exact before/after binary, driver
and fixture hashes matched. Owned helper/workspace/root cleanup passed, and both
driver cleanup and the final outside-sandbox same-ID process/temp scan returned0.
This is a native compatibility result, not an unfiltered-suite or live result.

[Compact evidence](f1-final-compatibility-evidence.json) retains exact identities,
receipt provenance, negative target transformation, cancellation and named proof.
Full commands, traces and styled captures remain in the private run artifacts.

| Fresh case | Seconds |
| --- | ---: |
| setup | 0.021 |
| busy | 65.135 |
| receipts | 1.112 |
| ordering | 35.219 |
| invitations | 33.062 |
| negative | 31.169 |
| failures | 4.681 |
| lifecycle | 3.329 |
| compaction | 0.755 |
| handoff | 6.582 |
| channel | 50.059 |
| named | 71.875 |

Busy text/tool delivery retained the same outer turn, including the trailing-wake
negative. Exact native receipt/branch proof, priority ordering, separate invitation
acceptance, true target transformation and recovery, truncation refusal and
cancelled-settle recovery all passed. The negative witness includes the target ID
and full body before transformation and the actual returned replacement afterward;
its pending receipt and subsequent true-body ACK are separate observations.
Lifecycle, compaction and current-owner handoff passed. Channel-disabled Native
wake preserved the no-false-context-receipt rule and recovered through explicit
inbox before channel restoration.

Named/tmux ran last in its own verified private session. It passed startup
admission, actual idle p0 receipt, cooldown-eligible passive p1 next-turn delivery,
busy-tool p0 receipt and no trailing wake. It remains distinct from Herdr native
host evidence. All processes and private tmux/Herdr resources were reaped.

## Scope of the fourteen-case mapping

The twelve rows above are fresh F1-candidate results. **Safety and threshold are
historical compatibility evidence**, from the reviewed supplement on candidate
`ac61f595…`: safety182.537s and threshold101.877s, with actual pre-send cooldown
eligibility. They are not re-labelled as fresh a8-binary passes. See
[timing/settle evidence](task-3-timing-settle-evidence.json) and
[original fourteen-case mapping](task-3-final.md).

A source comparison from those observations to the F1 build shows no change in
`src/host/native.rs`, `src/host/named.rs`, priority, threshold or lazy wake admission.
The only composer delta is the already reviewed default post-compaction
`?/capacity` footer spelling. Its15 fixture cases retain strict styled cursor,
rule-pair, selector and draft guards with numeric and unknown usage; final native
compaction/channel now passes on the F1 candidate. Those relevant source paths
are unchanged since the accepted footer fix `a6957f160`. No broad safety run is
inferred from the narrower final delta.

External custom overlays, spaces-only cursor-home drafts and extension-set but
unrendered text retain their explicitly accepted screen-observability limits.
These are not preservation claims. Unknown layouts otherwise fail closed,
built-in selectors remain guarded, and focused-empty protection remains one
minute. hc runtime was not exercised and has no native composer wake by contract.
Scripted invitation acceptance passed; no live-model acceptance is claimed.

The notice reload/page/isolated-wake evidence remains separately hashed in
[native notice attempts](f1-native-attempts.md). The fresh standalone
[HOL progression](f1-native-hol.md) observed two real automatic host wakes after
one draft release, delivering old then new with frontier advancement. Notice
cases are opt-in standalone; no contiguous notice-suite result is invented.

Whole-branch review, the corrected unfiltered-suite attempt, scoped live test and
five-minute suite-budget disposition are separately owned gates. No provider
credential was read or paid request sent in this compatibility run.
