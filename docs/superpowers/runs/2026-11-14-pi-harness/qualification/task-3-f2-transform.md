# PR1-F2: make transformed-negative qualification causal

The original transformed-boundary case could pass if watch delivery arrived only
for the retry: no target proposal was transformed, its receipt stayed pending,
and the later retry still acknowledged it. The r2 result supplies no causal
transformed-target proof and is withdrawn as evidence for that predicate. This
is a qualification gap, not proof of a production false receipt.

A deterministic delayed-watch regression executed that complete old case and
failed because no assertion rejected the absent target (`AssertionError not
raised`). The corrected case fails immediately if the exact target witness is
missing; it never retries the negative until a target eventually appears.

The later fixture transformer logs the actual entries it receives and the exact
entries object it returns, with recipient session and requested target metadata.
The driver requires the exact message-header ID and full unique body in a real
pre-transform entry, then the same custom type with replaced content in the
returned entries. Target metadata alone is insufficient. It additionally requires
that replacement in actual native context, absence of the original body there,
a pending canonical receipt, and full original-body observation plus receipt on
the subsequent real user retry. A fresh public-context observation before stream
release yields through the event loop; it is not watch-delivery proof.

All 16 driver self-tests pass in 1.853 s. The actual fixture test checks its log
against its returned entries and rejects empty proposals, wrong session, wrong
ID, partial body and unchanged content. Independent scoped review approved patch
SHA-256 `7c8bfada260aaf1cbdb25c30f63bffb39e517980b792e69ffb92726d0199c1a9`.
Only the qualification driver and fixture changed.

The actual native `negative` case passed in 2.067 s against an owned immutable
reflink of candidate `198e81ff9307f3e88c29cd47ba8ae425d0ed4f6283e5bb80a676636058895cce`,
whose embedded extension is the reviewed pre-F1 production snapshot. This is
F2 driver evidence, not qualification of F1 notice-retention changes. It witnessed
target `mmLcvWT1syyX` with its full unique body, returned replacement and actual
replacement context, pending receipt, then original-body recovery and ACK. The
separate truncated-body negative also passed, including explicit full-body read
and ACK. [Selected exact evidence](task-3-f2-evidence.json) retains the witness,
source/fixture hashes, receipt provenance and cleanup.

Native cleanup verified helper exit 0/reaping, scratch-root removal, exact-run
process/temp scan 0 and no errors. The self-test run's scoped scan also returned
0. The owned frozen binary was removed after its hash and evidence were preserved.
Production SDK transformation evidence is shared with the extension writer's
frozen affected-suite run; no duplicate SDK run was performed for this driver-only
change. Final integrated SDK/native/full/live gates remain separately required
after F1 integration and review.
