# Scoped live Pi qualification

The single authorized live run passed: **39.501s, exit0**, OpenRouter model
`deepseek/deepseek-v4-pro`, source `d55f7e984`, candidate `a8aef2df…`. Root
executed it once; this publisher independently checked the sanitized artifacts
and started no runtime or credential access. Exact identities, full frames,
receipt provenance and timestamps are in [final-live-evidence.json](final-live-evidence.json).

The native runtime reported four `after_provider_response` callbacks with
HTTP200. All three outer starts and four native context callbacks retained the
same Pi SID, provider and model. The starts were the baseline, one exact
`herdr-threads: wake up`, and the later explicit user turn.

Public send target `mAZm2GyaQoOR` contained exactly `RECEIPT-EXACT-FRAME`.
The identical finalized custom-message entry appeared in the current branch and
actual native context at1794752138394. The canonical receipt was acknowledged
at1794752138414 with `via=context`, `cooperative_mod_delivery`, the same SID
and current binding generation. This is context receipt.

The actual wake-turn tool list contains one `read` of the installed skill and
**zero inbox calls**. No redundant inbox round trip followed the already
delivered frame. Subsequent native contexts retaining the same branch entry
are conversation history, not additional wake turns.

The passive p1 negative was cooldown-eligible before send: existing eligibility
at1794752170.388, observed1794752170.3934703, send completed1794752170.4400933.
No new native start occurred during the following2.044907s observation. The
explicit next turn began at1794752172485 and received the exact passive frame
in branch/context at1794752172490. Provider, model and SID remained unchanged.

Live execution used the frozen binary at
`target/debug/deps/herdr_threads-75705d562a6fb0ec`. Root recorded the same a8
binary hash before/after and restored `target/debug/herdr-threads` to those
bytes; the publisher independently hashed both postrun paths and confirmed
they match. Driver `50a3f9fb…`, fixture `08174d10…`, official runtime entry
`e79626f2…`, and the installed extension digest are recorded separately.

Owned cleanup passed with helper reaped, private workspace/root removed,
empty cleanup errors and process/temp scan exit0. Root's additional outside
same-ID scan also returned0 for run
`e6500a20-fb8e-4e23-9327-674e4929736b`. The temporary `provider.env` credential
file was removed; the authorized source file was read only.
The driver completed its exact-secret retained-evidence scan before successful
exit; this publisher reviewed sanitized evidence only and did not reread the
credential source or repeat a raw-secret scan. No secret was placed in this
report.

The earlier automatic approval rejection remains a prelaunch, unattempted live
run. This later explicitly authorized run is the one actual live result. Busy,
notice, draft/selector and invitation behavior retain their separate scripted
evidence; live invitation acceptance remains outside this scope. Full-suite,
review and the open suite-runtime budget disposition are separate records.
