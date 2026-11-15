# Fresh native observations in the qualification driver

The first integrated native attempt on `3e839bb0c` stopped in the multiline-home
safety probe. It preserved its failed result and completed owned cleanup: helper
reaped, root removed, exact-run process/temp scan clean. It is not a passing
14-case qualification run.

The pane already displayed the controlled `first\nsecond` draft before the send.
The driver nevertheless selected the latest periodic state record, which still
held four spaces from the preceding equivalence probe. Relative to the pre-send
pane capture:

| Observation | Relative time | Editor / result |
| --- | --- | --- |
| Latest earlier periodic state | -136 ms | Four spaces, UTF-8 hex `20202020` |
| Controlled terminal input burst | -5 to -1 ms | `first`, newline, `second`, cursor-home |
| Before pane capture | 0 ms | Rendered multiline draft |
| New periodic state | +65 ms | `first\nsecond`, hex `66697273740a7365636f6e64` |
| After pane capture | +2055 ms | Byte-identical styled pane |

Native start counts were 4 before and 4 after. No terminal input occurred within
the held interval. The failure compared asynchronously sampled states from
different input generations. Equal pane bytes alone were not used to dismiss a
possible mutation.

The qualification-only fixture now responds to an owned session/nonce request
with a fresh snapshot through its existing public context getters. The driver
waits for the intended editor before taking the pre-send pane and run baseline;
it obtains another fresh observation after the hold. It retains exact styled
comparison and checks the original run baseline through the post-observation.
Captured proof includes each nonce, observation time, editor/session/model/provider
and run count. No production extension, composer policy or core behavior changes.
No keyboard input or private runtime API is used for observation requests.

The deterministic regression failed on the original stale-editor comparison,
then passed with fresh observation. A second test rejects old nonce data and waits for the intended editor; the
sampler also restricts each request to its native session. All 10 driver self-tests pass (1.180 s).
Existing synchronous-send-start, UI/model-preservation and bounded cleanup tests
remain enabled.

Independently reviewed frozen source patch SHA-256:
`c9baffba5b3bb9896d6cf2f28dc7595ef6e69e3699a5edc959658bca5337081c`.
Focused native safety passed in 210.685 s on the uncommitted frozen patch above
against production candidate `ac61f5958bc0dea2eadb41b1c27f6aaa669ec2b8089d9d71a3f936149ad25cff`.
All seven draft/selector before-after pairs retain equal styled output, exact
native editor and model/provider/session, and unchanged pre-trigger run counts;
each pair has distinct fresh observation nonces. Cleanup reports `ok=true`,
helper reaped, root removed and exact-run leak status 0.

Qualification driver SHA-256:
`8d7754f88342c14aa29099c038bf0dd50f9b282d5a60332f6e99b8958db5e16f`.
Fixture SHA-256:
`f102cc5e147844c59d7b7e85731a3a470d1f3cea39c2464f414e6c17954e6330`.
The complete native/live/full-suite gates remain separate acceptance evidence.
