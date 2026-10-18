cov-1 · r1 · GAP · R11/R12 byte identity · applied — acceptance added to .7 (encode byte-identity) and .6 (canonical_bytes re-derive)
cov-2 · r1 · GAP · R7 append-only · rejected — .6 acceptance already has the MANIFEST hash guard (reviewer input was truncated)
cov-3 · r1 · GAP · R4 promotion after upgrade · applied — new leaf ht-j87z.13
cov-4 · r1 · GAP · opaque size bound · applied — .2 OPAQUE_MAX_BYTES, .4 enforcement
cov-5 · r1 · GAP · fork frames with opaque toward non-advertising peers · applied — .4/.10 gating, .7 corpus case
cov-6 · r1 · UNOWNED-SEAM · StoredEntry/EntryPrefix · applied — .2 owns line rewritten, .3 consumes
cov-7 · r1 · UNOWNED-SEAM · spike note contract · applied — .1 owns contract with go/no-go
cov-8 · r1 · UNOWNED-SEAM · lint file list/baseline · applied — .8 owns; .4 and .7 depend on .8; tests/ excluded
cov-9 · r1 · UNOWNED-SEAM · status/diagnostic fields + have-head contract · applied — .4 owns
cov-10 · r1 · UNOWNED-SEAM · synthetic HTJRNL99 helper · applied — .2 owns, others consume
cov-11 · r1 · ORPHAN · .11 · applied — close-as-unneeded path documented
cov-12 · r1 · GAP · TRUST-POLICY accepted limit · applied — decided accepted limit, edit in .4
