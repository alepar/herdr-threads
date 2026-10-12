# Direct frozen claims and publication-probe oracle: completed evidence

**C remains NOT READY; formal code-roast counters remain 0.** This checkpoint preserves two direct frozen-claim passes and the publication-probe oracle FAIL→PASS sequence. No earlier failures are overwritten, and no Q4 design or broader runtime clearance is implied.

## Actual completed scopes

### ht-C-v090-direct-frozen-claim-controls-07fdf88d

- HEAD `07fdf88da984869eb2908c25f9ff20caf253dc28`; tree `063aa3709e46e1ed2a21214f763c079d10e86785`; scoped run UUID `74feca6a-8484-465e-a653-d08eec178940`.
- Summary [   0.491s] 2 tests run: 2 passed, 727 skipped
- Listing exit 0, 16.63828025001567 seconds; runtime exit 0, wrapper 0.6977702920557931 seconds; scoped leak check exit 0.
- Exactly 2 expected/selected cases, with individual result counts independently verified.

### ht-C-v090-publication-probe-oracle-07fdf88d

- HEAD `07fdf88da984869eb2908c25f9ff20caf253dc28`; tree `063aa3709e46e1ed2a21214f763c079d10e86785`; scoped run UUID `707085e3-28d0-48c8-af69-2a2fc54c7064`.
- Summary [   0.216s] 1 test run: 0 passed, 1 failed, 4894 skipped
- Listing exit 0, 0.203917833045125 seconds; runtime exit 100, wrapper 0.4236935420194641 seconds; scoped leak check exit 0.
- Exactly 1 expected/selected cases, with individual result counts independently verified.

### ht-C-v090-publication-probe-corrected

- HEAD `3dd448ef061db0ed9dcb3db58e22c50df0265354`; tree `804e9284e76c2f9a00cc0fb87dcc0982faf81fea`; scoped run UUID `87e91b68-c9f3-4a8c-9f9c-4e7db5ab78ec`.
- Summary [   0.401s] 1 test run: 1 passed, 4894 skipped
- Listing exit 0, 26.165037083090283 seconds; runtime exit 0, wrapper 0.6179406250594184 seconds; scoped leak check exit 0.
- Exactly 1 expected/selected cases, with individual result counts independently verified.

## Causal limits

- Direct frozen-claim controls preserve explicit Some and historical None caller claims through durable retry with a changed caller. Both pass at 07fdf88d. These tests address frozen attribution; they do not close the separate invited-member direct-Lazy presentation/settled-episode contract.
- Original publication-probe oracle at 07fdf88d reaches a tuple assertion: actual `(100, 200, 2)` versus old expected `(100, 100, 2)`. This exact failure is retained; it must not be described as a loss of publication progress or as evidence to weaken the actual two-probe source accounting.
- Corrected publication control passes at 3dd448ef. Parent confirms both tuple assertions are verified by this pass. The raw PASS log does not print either tuple; this evidence task did not inspect source to independently restate the second tuple. No counter bound, production behavior or timing budget was changed by this task.
- `/private/tmp/ht-C-v090-q3-current60-3dd448ef` remains RUNNING at dispatch and was not read or copied. Parent confirmation of completion is required before any later snapshot.
- Q4 broader design remains under review/unselected. Focused claim/probe passes do not select that design or provide complete source qualification.

## Byte-exact sources, destinations and SHA-256

Full supplied packages are copied, including raw list inventories, execution/binary provenance, run logs, listing stderr and scoped leak logs. Every destination was verified byte-equal to its preserved source and SHA-256 equal. Destinations are relative to this checkpoint directory.

| Source | Destination | Bytes | SHA-256 |
| --- | --- | ---: | --- |
| `/private/tmp/ht-C-v090-direct-frozen-claim-controls-07fdf88d/execution.json` | `ht-C-v090-direct-frozen-claim-controls-07fdf88d/execution.json` | 1745 | `d397f9b3cf63ada597999dfa007842720b02a0dd2ded1fe5f096fb5fc3cfbd3a` |
| `/private/tmp/ht-C-v090-direct-frozen-claim-controls-07fdf88d/leaks.log` | `ht-C-v090-direct-frozen-claim-controls-07fdf88d/leaks.log` | 25 | `87e8218c7e39ad3f7c328894df8a3c19809b0772116e37ea6d9e45195ffd6a4e` |
| `/private/tmp/ht-C-v090-direct-frozen-claim-controls-07fdf88d/list.json` | `ht-C-v090-direct-frozen-claim-controls-07fdf88d/list.json` | 125543 | `6dbe3886bf99a4493700882ccc90e0acc3737bb43e73b8ef4d79b22063bf9efe` |
| `/private/tmp/ht-C-v090-direct-frozen-claim-controls-07fdf88d/list.stderr` | `ht-C-v090-direct-frozen-claim-controls-07fdf88d/list.stderr` | 220 | `ca817134fcdddcf8d31924fefae703daa5673b27b7f3bd31402002b006bbcf8c` |
| `/private/tmp/ht-C-v090-direct-frozen-claim-controls-07fdf88d/run.log` | `ht-C-v090-direct-frozen-claim-controls-07fdf88d/run.log` | 633 | `e29d6ee275063aca7e7ad1b129076d8b1812fc65bd24a2681f713cb928813f7e` |
| `/private/tmp/ht-C-v090-publication-probe-oracle-07fdf88d/execution.json` | `ht-C-v090-publication-probe-oracle-07fdf88d/execution.json` | 1461 | `7ab3d60b2fb042a09f17819ec6769d23d48e211d6913d65905ea61e6ef358857` |
| `/private/tmp/ht-C-v090-publication-probe-oracle-07fdf88d/leaks.log` | `ht-C-v090-publication-probe-oracle-07fdf88d/leaks.log` | 25 | `87e8218c7e39ad3f7c328894df8a3c19809b0772116e37ea6d9e45195ffd6a4e` |
| `/private/tmp/ht-C-v090-publication-probe-oracle-07fdf88d/list.json` | `ht-C-v090-publication-probe-oracle-07fdf88d/list.json` | 867951 | `7f0ab489dc02cf34bc37074011c70c6992c8b0e5a494aaf09111cace8a650599` |
| `/private/tmp/ht-C-v090-publication-probe-oracle-07fdf88d/list.stderr` | `ht-C-v090-publication-probe-oracle-07fdf88d/list.stderr` | 73 | `eace1726b7004b659d109fe8d7d692015b8d9b90310f4aaac3afa5f19beb649c` |
| `/private/tmp/ht-C-v090-publication-probe-oracle-07fdf88d/run.log` | `ht-C-v090-publication-probe-oracle-07fdf88d/run.log` | 1519 | `2fdf1b24268ecab06c76d82ab08b1befabecae30115c19fdac91efc8f4267716` |
| `/private/tmp/ht-C-v090-publication-probe-corrected/execution.json` | `ht-C-v090-publication-probe-corrected/execution.json` | 1460 | `a49b1d095488aebddc591a4a29cecfe7c162948451bd1b56aeb0fc24ec81b05c` |
| `/private/tmp/ht-C-v090-publication-probe-corrected/leaks.log` | `ht-C-v090-publication-probe-corrected/leaks.log` | 25 | `87e8218c7e39ad3f7c328894df8a3c19809b0772116e37ea6d9e45195ffd6a4e` |
| `/private/tmp/ht-C-v090-publication-probe-corrected/list.json` | `ht-C-v090-publication-probe-corrected/list.json` | 867951 | `7f0ab489dc02cf34bc37074011c70c6992c8b0e5a494aaf09111cace8a650599` |
| `/private/tmp/ht-C-v090-publication-probe-corrected/list.stderr` | `ht-C-v090-publication-probe-corrected/list.stderr` | 220 | `9a3532fe497fbebf50f283dace7459a6c7f103cd998de0276afa2d8f94373928` |
| `/private/tmp/ht-C-v090-publication-probe-corrected/run.log` | `ht-C-v090-publication-probe-corrected/run.log` | 504 | `6a6d848a0b44d753b98aa0b5945feec248cbf8a2ffe426bd1fd7dc79fe2855a9` |

Only new evidence/documentation paths were written. No source/test/index/Git mutation, Cargo/runtime test, Herdr command, subordinate agent or process launch occurred. Earlier original failures remain intact. QUIESCENT.
