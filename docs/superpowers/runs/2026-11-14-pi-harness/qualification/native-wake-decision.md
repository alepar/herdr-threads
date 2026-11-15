# Native wake decision

These are exploratory native-TUI results, not final scheduler or live-provider qualification. The frozen candidate was the reviewed Rust implementation `ca4474c06` (subsequently replayed as `e61d0e355`), SHA-256 `1ef537134a6056b834b33125438c35fae0a9ee9da6f49552b3cca5751a59e5f8`. The official npm pi specimen was 1.5.2, a historical diagnostic rather than an admission requirement. Both experiments used public isolated setup/launch, a scripted provider, private hosts/state and owned senders. No credentials or real configuration were used.

## Why channel self-wake was rejected

With the built-in Ctrl+L model picker visibly open and the underlying editor empty, a priority-zero message caused a second native agent/provider run. No UI prompt event fired for the built-in picker. Checking isIdle, editor text and extension UI-prompt spans therefore did not establish safe idle input.

Public editor replacement could supply a focus witness but can close an already active selector and recreate another extension's editor state. There is no supported safe installation-time witness. Editor takeover, private-property patching and key-history inference were rejected.

## Native path experiment

The second experiment changed only the private installed extension copy by inserting an immediate return in its wake function. This deliberately disabled the unsafe automatic run while leaving boundary delivery and receipts intact. Setup status consequently reported the modified extension unavailable; the public launch still started. This is an explicitly modified experimental specimen, not a claim that the production setup passed with that modification.

| Observation | Result |
|---|---|
| Herdr agent detection | Recognized pi |
| Held text and tool runs | Both reported working; no false idle in these held samples |
| Empty default composer | Two full-width plain rules, a blank editor row and current cwd/stats footer |
| Literal draft | Text remained in the composer without submission or loss |
| Built-in model/settings and inline selectors | Multirow UI replaced the editor slot and could be rejected by a strict composer grammar |
| Host-submitted content-free marker at verified idle | Arrived as an ordinary user prompt; selected model and native session remained unchanged |
| Pending mail and requested receipt | Exact frame matched context, provider request and finalized current branch; receipt changed from pending to acknowledged via context/cooperative_mod_delivery |

The host marker test establishes the native prompt path. It does not establish a daemon ComposerPolicy, threshold scheduling or safety of an unimplemented recognizer. Those require final-tree qualification.

## External custom-overlay limit

An extension custom overlay preserved byte-identical styled composer/footer rows and identical public pane/agent metadata, including idle readiness. Its arbitrary content appeared elsewhere in the transcript. The public read schema supplied no internal focus target or terminal cursor witness.

Custom overlays do emit UI prompt spans, but the runtime queues their emission asynchronously and invokes the UI method immediately. A handler-driven status marker cannot guarantee a veto before opening. The existing focused-pane guard requires an empty composer observed for one minute, which narrows active-user interaction but does not detect internal UI focus.

The coordinator accepted Native wake guarded for built-in/default selectors, with other extensions' arbitrary custom overlays explicitly documented as a limit in TRUST-POLICY and the pi README. This integration opens no overlays. Unknown composer layouts must refuse, and final tests must cover styled whitespace drafts, multiline/wrapped input and cursor movement rather than equating trimmed text with emptiness.

## Spaces-only draft observation

A subsequent isolated capture compared empty input with one and four spaces. With the cursor at the end, plain screens were identical but styled inverse-cursor columns differed. After Ctrl+A, the nonempty spaces-only drafts were byte-identical to empty in both plain and styled captures. Multiline, blank-newline and wrapped drafts remained distinguishable. Cleanup and scoped leaks passed.

The coordinator accepted this narrow screen limit while requiring the styled column-zero cursor check. A native marker may be submitted with extra spaces in that indistinguishable state; this is not proof of universal draft preservation. No render-time widget is added in this epic. Follow-up `ht-1z1n` records investigation of a supported readiness witness that could also address the custom-overlay gap, including its UI ownership and race costs.

## Implementation decision and cleanup

Use the existing PushChannel + IdleWake::Native declaration and a pi-local ComposerPolicy. The watch continues delivering bodies but does not suppress native wake under that existing policy; no fourth registration flag or protocol change is needed. Remove extension automatic self-wake and retain bounded boundary delivery, exact receipts and completed-outcome continuation.

Both experiments closed their owned workspace, stopped the private daemon/host, reaped children and removed helper roots. Scoped process/temp checks passed. Raw synthetic traces, pane captures and specimen hashes remain in the coordinator's ignored evidence workspace pending final qualification and sanitization. No provider credential was accessed.
