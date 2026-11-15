# Integrated commit reorder

Held main base: `91ce0cdeb`. Final source anchor: `cd0bcac30c40ff360f61178b3a0ac5a32c0309d0`. Tree: `7c91e13886262d926f50ff0bfec2f39aab307606`. The coordinator performed the reorder; this publication verified the recorded 51-entry mapping and current Git order. Every entry is patch-equivalent (`=`); the complete pre/post tree is identical. No tests were run for this documentation publication. Native/full/live gates remain separate.

Neutral notice source, evidence and packet fixtures now precede Pi work. Old hashes remain valid historical evidence references.

## Complete old-to-new range diff

```text
 1:  2cc49aada =  1:  2cc49aada Share digest-owned file operations without changing installed state
 2:  b690b322e =  2:  b690b322e fix(named): reserve observation order before capture
 3:  f97d7e0ca =  3:  f97d7e0ca Synchronize named allocation with canonical currentness before admission
 4:  493c8bc86 =  4:  493c8bc86 Record causal named-startup currentness qualification
 5:  eabb28b94 =  5:  eabb28b94 Use real named observation in inherited-name fixtures
 6:  94ecd3268 =  6:  94ecd3268 Record inherited-name fixture compatibility evidence
 7:  78ac1e4a3 =  7:  78ac1e4a3 Bind inherited-name enumeration to the reserved real host
42:  5b2713226 =  8:  d55502a16 fix(delivery): defer notice settlement for explicit manual presentation
44:  98da1df4d =  9:  fc4884a16 docs: record deferred notice seam validation
46:  91e233980 = 10:  0149b7743 test(watch): freeze production manual packet fixtures
 8:  570192c59 = 11:  d11cd09a0 docs: research and design pi harness integration
 9:  df7308a58 = 12:  c17c8eb2e docs: record pi integration base and model continuity check
10:  b801b34a3 = 13:  5021b851a docs: freeze pi extension adapter contract for implementation
11:  aa58d3976 = 14:  d6bdd1de2 Add native pi extension boundary delivery
12:  9bf684a90 = 15:  2fcb433c0 docs: record pi runtime retry and ordering constraints
13:  4f2c8990e = 16:  746250fa3 Fence pi attention reoffers and join detached watch cleanup
14:  046ffe861 = 17:  895b5504e docs: define bounded pi wake novelty fallback
15:  c7a57f91b = 18:  324572737 Rearm recovered pi wake witnesses for the successor owner
16:  7421d9b3e = 19:  8ebac7576 docs: record accepted pi extension evidence and integration seams
17:  c08609dc4 = 20:  0fcd43457 docs(pi): require fail-closed hook owner evidence
18:  a7d0ca5a6 = 21:  8ff3b968f docs(pi): distinguish hook refusal from empty success
19:  881f16907 = 22:  d62bd3917 Add optional process claim consistency validation
20:  ffe12e86b = 23:  6c0ad1bb2 Refuse failed local evidence before hook orchestration
21:  15e26273f = 24:  846031985 Allow channel-owned lifecycle output without mail offers
22:  34fd9201d = 25:  6a05c5ff8 Register the pi TUI adapter and owned extension setup
23:  e10fd3e7f = 26:  0f04b0b75 docs(pi): record Rust adapter verification and core seams
24:  fcc44f56a = 27:  b947319af Repair the pi trust amendment section boundary
25:  dcd2c1294 = 28:  505e34a3c Use shared owned-file operations for pi setup
26:  4b05f7d0d = 29:  d00b9c537 docs(pi): record setup extraction compatibility evidence
27:  3b679873a = 30:  6c129463f docs(pi): select guarded native wake after real TUI probes
28:  8d729db2c = 31:  c5b74c382 docs(pi): preserve accepted Rust adapter review evidence
29:  f89a085d2 = 32:  b6f384187 docs(pi): define owner-fenced native turn-state reports
30:  146970c28 = 33:  ba4308b08 docs(pi): record accepted native screen observability limits
31:  593a99936 = 34:  77e409d2f feat(pi): guard native wake and qualify terminal delivery
32:  277cb0690 = 35:  0715f50ab fix(pi): make native driver negatives and cleanup conclusive
33:  c5ba06081 = 36:  de6b1ccc7 test(pi): align native wake policy expectations
34:  692a69ed0 = 37:  e49f4f1f3 fix(pi): sample fresh native state for blocked probes
35:  8f460f7fe = 38:  de242b775 test(pi): prove eligible holds and cancelled settle delivery
36:  8a386d112 = 39:  632cfa0bd fix(pi): recognize default unknown context usage after compaction
37:  92bd8d66c = 40:  8495cd942 style(pi): format unknown usage regression assertions
38:  62bc385d0 = 41:  748406a63 test(pi): update native registry expectations and record bounded gates
39:  5f8fbcd34 = 42:  884709b5c test(pi): require exact native transformed-target evidence
40:  d108f38dc = 43:  ab892e951 Fix pi watcher test cleanup and missing-mode diagnostics
41:  89788571d = 44:  ab5277339 docs: retain Pi PR review and scoped dispositions
43:  e45e71aeb = 45:  965032112 docs(pi): amend notice delivery and confirmation contract
45:  d7c70fe9b = 46:  eb45ce253 docs: record accepted deferred-notice source review
47:  6dc221c3d = 47:  7807673c2 Fix pi notice delivery with explicit whole-page presentation
48:  b5f19ac3f = 48:  a31d3f5a5 Discard incompatible pi recovery hints before canonical replay
49:  ad04f0cbe = 49:  3e7434a1f docs(pi): record reviewed notice delivery repair
50:  e00c692e1 = 50:  cec4ef30e test(pi): qualify canonical notice recovery in native TUI
51:  c733d97ca = 51:  cd0bcac30 test(pi): stage notice wake before clearing the owned draft
```

## Current chronological order

```text
2cc49aada Share digest-owned file operations without changing installed state
b690b322e fix(named): reserve observation order before capture
f97d7e0ca Synchronize named allocation with canonical currentness before admission
493c8bc86 Record causal named-startup currentness qualification
eabb28b94 Use real named observation in inherited-name fixtures
94ecd3268 Record inherited-name fixture compatibility evidence
78ac1e4a3 Bind inherited-name enumeration to the reserved real host
d55502a16 fix(delivery): defer notice settlement for explicit manual presentation
fc4884a16 docs: record deferred notice seam validation
0149b7743 test(watch): freeze production manual packet fixtures
d11cd09a0 docs: research and design pi harness integration
c17c8eb2e docs: record pi integration base and model continuity check
5021b851a docs: freeze pi extension adapter contract for implementation
d6bdd1de2 Add native pi extension boundary delivery
2fcb433c0 docs: record pi runtime retry and ordering constraints
746250fa3 Fence pi attention reoffers and join detached watch cleanup
895b5504e docs: define bounded pi wake novelty fallback
324572737 Rearm recovered pi wake witnesses for the successor owner
8ebac7576 docs: record accepted pi extension evidence and integration seams
0fcd43457 docs(pi): require fail-closed hook owner evidence
8ff3b968f docs(pi): distinguish hook refusal from empty success
d62bd3917 Add optional process claim consistency validation
6c0ad1bb2 Refuse failed local evidence before hook orchestration
846031985 Allow channel-owned lifecycle output without mail offers
6a05c5ff8 Register the pi TUI adapter and owned extension setup
0f04b0b75 docs(pi): record Rust adapter verification and core seams
b947319af Repair the pi trust amendment section boundary
505e34a3c Use shared owned-file operations for pi setup
d00b9c537 docs(pi): record setup extraction compatibility evidence
6c129463f docs(pi): select guarded native wake after real TUI probes
c5b74c382 docs(pi): preserve accepted Rust adapter review evidence
b6f384187 docs(pi): define owner-fenced native turn-state reports
ba4308b08 docs(pi): record accepted native screen observability limits
77e409d2f feat(pi): guard native wake and qualify terminal delivery
0715f50ab fix(pi): make native driver negatives and cleanup conclusive
de6b1ccc7 test(pi): align native wake policy expectations
e49f4f1f3 fix(pi): sample fresh native state for blocked probes
de242b775 test(pi): prove eligible holds and cancelled settle delivery
632cfa0bd fix(pi): recognize default unknown context usage after compaction
8495cd942 style(pi): format unknown usage regression assertions
748406a63 test(pi): update native registry expectations and record bounded gates
884709b5c test(pi): require exact native transformed-target evidence
ab892e951 Fix pi watcher test cleanup and missing-mode diagnostics
ab5277339 docs: retain Pi PR review and scoped dispositions
965032112 docs(pi): amend notice delivery and confirmation contract
eb45ce253 docs: record accepted deferred-notice source review
7807673c2 Fix pi notice delivery with explicit whole-page presentation
a31d3f5a5 Discard incompatible pi recovery hints before canonical replay
3e7434a1f docs(pi): record reviewed notice delivery repair
cec4ef30e test(pi): qualify canonical notice recovery in native TUI
cd0bcac30 test(pi): stage notice wake before clearing the owned draft
```
