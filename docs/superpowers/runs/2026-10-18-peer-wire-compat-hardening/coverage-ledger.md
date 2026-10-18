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
cov-13 · r2 · UNOWNED-SEAM · understood-layout hook/status contract/have-head/OPAQUE_MAX_BYTES/TRUST-POLICY owner/spike+3 duplicate lines · applied — owns/consumes lines rewritten (single consolidated lines), .9 deps and scope extended
cov-14 · r2 · GAP · upgrade e2e · applied — .5 depends on .13 with upgrade scenario
cov-15 · r2 · GAP · promotion preserves bytes/hash/crash-safe · applied — .13 acceptance
cov-16 · r2 · GAP · lint negative fixtures · applied — .8 acceptance
cov-17 · r2 · GAP · golden HTJRNL01 parses under EntryPrefix · applied — .2 acceptance
cov-18 · r2 · UNOWNED-SEAM · unpark trigger owner · applied — .10 owns API, .13 calls
cov-19 · r2 · GAP · shadow decoder faithful to v2.1.1 source · applied — .7 acceptance
cov-20 · graph · GRAPH-EDGE · .10 <- .4 · applied — apply parking/union readers do not consume wire form; status field set ownership moved to .10, .4 now depends on .10
