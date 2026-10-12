# V090 repair checkpoint: preserved execution and author evidence

Exact immutable execution copies plus snapshots of the latest requested WS reports. This checkpoint preserves failed history and the compilation-only attempt alongside scoped passing reruns. Earlier expanded 657-test run remains FAILED; these focused checks do not establish complete readiness or a passing expanded rerun. Q3 author work is outside this evidence task.

## Recorded executions

### ht-C-v090-repaired-lib57-first

- HEAD `94a7c539491006782497db9af4af8f2610c0806b`; tree `dcb572da466b6f8f63232f75a1358646c58b4e7a`; run ID `cc503c42-9f70-4c13-81bd-804701732232`.
- Summary [   5.245s] 57 tests run: 53 passed, 4 failed, 4785 skipped
- Listing exit 0; runtime exit 100; scoped leak check exit 0.

### ht-C-v090-repaired-lib4-second

- HEAD `6c7bcb572108209a07e41b6ca7f7d5ca4f4e392b`; tree `1ed0a4a53301e6edc0198ab10b7d8e22ff52df69`; run ID `82602e03-53d5-48a3-86e6-2e8f257e1214`.
- Compile E0277 (`u64: ToSql`); listing exit 101; no runtime executed; run.log absent.
- Listing exit 101; runtime exit not applicable (not executed); scoped leak check exit 0.

### ht-C-v090-repaired-lib4-third-compile-corrected

- HEAD `31819c1f9f5c960f27541128f743614970b10a17`; tree `19c7e09e3c90cfec66ff6782775c6ef4cfdb699a`; run ID `bdc69c56-f94f-435f-9455-3116776fa84b`.
- Summary [   0.788s] 4 tests run: 4 passed, 4838 skipped
- Listing exit 0; runtime exit 0; scoped leak check exit 0.

### ht-C-v090-repaired-hook6-first

- HEAD `31819c1f9f5c960f27541128f743614970b10a17`; tree `19c7e09e3c90cfec66ff6782775c6ef4cfdb699a`; run ID `716eb99c-663f-4f47-bd52-35118fc3cb12`.
- Summary [   5.215s] 6 tests run: 2 passed, 4 failed, 77 skipped
- Listing exit 0; runtime exit 100; scoped leak check exit 0.

### ht-C-v090-repaired-integration5-first

- HEAD `31819c1f9f5c960f27541128f743614970b10a17`; tree `19c7e09e3c90cfec66ff6782775c6ef4cfdb699a`; run ID `9f5157a4-bb23-4701-b7d4-2bfb4a3baa99`.
- Summary [   3.889s] 5 tests run: 5 passed, 232 skipped
- Listing exit 0; runtime exit 0; scoped leak check exit 0.

### ht-C-v090-repaired-carry32-33-first

- HEAD `31819c1f9f5c960f27541128f743614970b10a17`; tree `19c7e09e3c90cfec66ff6782775c6ef4cfdb699a`; run ID `d29a5fa9-a6a8-45c6-99d2-5659ba95e262`.
- Summary [   0.298s] 1 test run: 1 passed, 726 skipped
- Listing exit 0; runtime exit 0; scoped leak check exit 0.

### ht-C-v090-main565-materialization-merge-controls

- HEAD `16ede4ed2eaf3c24b62077f8f38cb0faf24b23cc`; tree `9b0f6151d01e5b071b4cfbda28e9af62286da9b3`; run ID `d5d052f6-26ad-4822-bc40-41a936f6249f`.
- Summary [   0.180s] 2 tests run: 2 passed, 4841 skipped
- Listing exit 0; runtime exit 0; scoped leak check exit 0.

## Reports and limits

Schema/carry, AGY, Mod failure triage, Mod independent-oracle proposal, fixture-quality review and Main565 impact reports are copied unchanged to distinct checkpoint paths. Their historical cutoffs, findings, proposal status and scoped limits remain intact. No proposal was applied or author conclusion changed by this task.

## Copy verification

Every destination below is byte-equal to its source and SHA-256 verified. All source files and previously frozen reports are preserved. Paths in the destination column are relative to this checkpoint directory.

| Source | Destination | Bytes | SHA-256 |
| --- | --- | ---: | --- |
| `/private/tmp/ht-C-v090-repaired-lib57-first/execution.json` | `ht-C-v090-repaired-lib57-first/execution.json` | 22780 | `67f7befc57d5e0d905c7395a04e69f7da7ba7c3ab860c6eed71837498ddc0a03` |
| `/private/tmp/ht-C-v090-repaired-lib57-first/run.log` | `ht-C-v090-repaired-lib57-first/run.log` | 12957 | `c26a77ea59a58486a173983c73b64484ab38c63f2715f95e3c89d72445fcc75a` |
| `/private/tmp/ht-C-v090-repaired-lib57-first/list.stderr` | `ht-C-v090-repaired-lib57-first/list.stderr` | 220 | `d0c23bc3db4275b9be866382bedf727c6b24cca4185c7a407cc068fd7fa55a2b` |
| `/private/tmp/ht-C-v090-repaired-lib57-first/leaks.log` | `ht-C-v090-repaired-lib57-first/leaks.log` | 25 | `87e8218c7e39ad3f7c328894df8a3c19809b0772116e37ea6d9e45195ffd6a4e` |
| `/private/tmp/ht-C-v090-repaired-lib4-second/execution.json` | `ht-C-v090-repaired-lib4-second/execution.json` | 911 | `8a6479829843d881f8ec71b91bbbe782e7fbadc4147bc25a5a5440163cf12ff3` |
| `/private/tmp/ht-C-v090-repaired-lib4-second/list.stderr` | `ht-C-v090-repaired-lib4-second/list.stderr` | 1457 | `34aad7f88093ce4a8247936aaf2a387ce5b51efda7ab41f6c3ed69faaf61f786` |
| `/private/tmp/ht-C-v090-repaired-lib4-second/leaks.log` | `ht-C-v090-repaired-lib4-second/leaks.log` | 25 | `87e8218c7e39ad3f7c328894df8a3c19809b0772116e37ea6d9e45195ffd6a4e` |
| `/private/tmp/ht-C-v090-repaired-lib4-third-compile-corrected/execution.json` | `ht-C-v090-repaired-lib4-third-compile-corrected/execution.json` | 2509 | `59595915b6dfd9e5bac4846113edbb878f11847652611539be38f761480704dc` |
| `/private/tmp/ht-C-v090-repaired-lib4-third-compile-corrected/run.log` | `ht-C-v090-repaired-lib4-third-compile-corrected/run.log` | 931 | `3660e6d1add88db1d1924f40e6574381a25c1c3d9d8d4a1e3b6ea621e2101ec5` |
| `/private/tmp/ht-C-v090-repaired-lib4-third-compile-corrected/list.stderr` | `ht-C-v090-repaired-lib4-third-compile-corrected/list.stderr` | 220 | `e2a57d055a33b7fe50249f963529137d6e744c23f43a78477978d54839123572` |
| `/private/tmp/ht-C-v090-repaired-lib4-third-compile-corrected/leaks.log` | `ht-C-v090-repaired-lib4-third-compile-corrected/leaks.log` | 25 | `87e8218c7e39ad3f7c328894df8a3c19809b0772116e37ea6d9e45195ffd6a4e` |
| `/private/tmp/ht-C-v090-repaired-hook6-first/execution.json` | `ht-C-v090-repaired-hook6-first/execution.json` | 3009 | `b69b15ebdb7eb14d3114f19b0de2c53e672fac8c09303a8c182922654b199bc4` |
| `/private/tmp/ht-C-v090-repaired-hook6-first/run.log` | `ht-C-v090-repaired-hook6-first/run.log` | 8676 | `94faaa77822fa6a1976521c99fa9b123d2414bf4cf0f62396b62e68fb09ab99d` |
| `/private/tmp/ht-C-v090-repaired-hook6-first/list.stderr` | `ht-C-v090-repaired-hook6-first/list.stderr` | 219 | `06f59ac2f9f1c6a2cdc94d1425918206886a69f967ac9fa516de9780e9bf390f` |
| `/private/tmp/ht-C-v090-repaired-hook6-first/leaks.log` | `ht-C-v090-repaired-hook6-first/leaks.log` | 25 | `87e8218c7e39ad3f7c328894df8a3c19809b0772116e37ea6d9e45195ffd6a4e` |
| `/private/tmp/ht-C-v090-repaired-integration5-first/execution.json` | `ht-C-v090-repaired-integration5-first/execution.json` | 2566 | `a8cd24d502d041b9c3ee2a61f367ead142eda9822276ab5f01d8571fc45e55a0` |
| `/private/tmp/ht-C-v090-repaired-integration5-first/run.log` | `ht-C-v090-repaired-integration5-first/run.log` | 1010 | `ffeeac0e9d45057f32deacd723559a1735998e1da1ac74e0751807830746c042` |
| `/private/tmp/ht-C-v090-repaired-integration5-first/list.stderr` | `ht-C-v090-repaired-integration5-first/list.stderr` | 219 | `57e4b8e0be205b111cfdd2f3242047422b5e143296aeaaeecf2610ff27907a4f` |
| `/private/tmp/ht-C-v090-repaired-integration5-first/leaks.log` | `ht-C-v090-repaired-integration5-first/leaks.log` | 25 | `87e8218c7e39ad3f7c328894df8a3c19809b0772116e37ea6d9e45195ffd6a4e` |
| `/private/tmp/ht-C-v090-repaired-carry32-33-first/execution.json` | `ht-C-v090-repaired-carry32-33-first/execution.json` | 1483 | `36f3c898fa4e0667c045308587b853c4213ca9715cac2cae36e46f2dd00355e5` |
| `/private/tmp/ht-C-v090-repaired-carry32-33-first/run.log` | `ht-C-v090-repaired-carry32-33-first/run.log` | 512 | `8c8df326e0a7a4b2695b368c1c55ca83780f82e1496e6c8372f2f371be53b733` |
| `/private/tmp/ht-C-v090-repaired-carry32-33-first/list.stderr` | `ht-C-v090-repaired-carry32-33-first/list.stderr` | 219 | `0d18f6084060882af259b12ed2ba30e1c47f15cf12101608e1fccb7afae5a847` |
| `/private/tmp/ht-C-v090-repaired-carry32-33-first/leaks.log` | `ht-C-v090-repaired-carry32-33-first/leaks.log` | 25 | `87e8218c7e39ad3f7c328894df8a3c19809b0772116e37ea6d9e45195ffd6a4e` |
| `/private/tmp/ht-C-v090-main565-materialization-merge-controls/execution.json` | `ht-C-v090-main565-materialization-merge-controls/execution.json` | 1750 | `a4117e4ecbebc054c4de5313a0dea9b681a969cc49a168deaa1df6f3a77b0f3b` |
| `/private/tmp/ht-C-v090-main565-materialization-merge-controls/run.log` | `ht-C-v090-main565-materialization-merge-controls/run.log` | 628 | `4f11e3458ed95cbdb6a40627a948fba0b38fe377582bcb2df6ee6ebd50a9037f` |
| `/private/tmp/ht-C-v090-main565-materialization-merge-controls/list.stderr` | `ht-C-v090-main565-materialization-merge-controls/list.stderr` | 220 | `6d2206f6ec2ce9a8a6d74f1813d6d642a57dff1f08ecc3d0df5fca8ac3244b17` |
| `/private/tmp/ht-C-v090-main565-materialization-merge-controls/leaks.log` | `ht-C-v090-main565-materialization-merge-controls/leaks.log` | 25 | `87e8218c7e39ad3f7c328894df8a3c19809b0772116e37ea6d9e45195ffd6a4e` |
| `/Users/alepar/AleCode/herdr-threads/.worktrees/handoff-delivery-redesign/.superpowers/sdd/2026-10-11-handoff-delivery-redesign-finish/resume-v090-schema-carry-failure-triage.md` | `reports/resume-v090-schema-carry-failure-triage.md` | 7780 | `70ba785688a177b7f633f7a85f1aa3d4a5fbe3c39f6fda8a7f2c2acfb941e4c1` |
| `/Users/alepar/AleCode/herdr-threads/.worktrees/handoff-delivery-redesign/.superpowers/sdd/2026-10-11-handoff-delivery-redesign-finish/resume-v090-agy-failure-triage.md` | `reports/resume-v090-agy-failure-triage.md` | 13066 | `edfde3af1fd6b223558f8e5b05d50c5d67cdbb72ffd898e878f5fea5fb71094e` |
| `/Users/alepar/AleCode/herdr-threads/.worktrees/handoff-delivery-redesign/.superpowers/sdd/2026-10-11-handoff-delivery-redesign-finish/resume-v090-mod-failure-triage.md` | `reports/resume-v090-mod-failure-triage.md` | 12204 | `57ebde055c88bd18c43a62e88304e19a1647f6a223f59f0a16818ddbdee58e9b` |
| `/Users/alepar/AleCode/herdr-threads/.worktrees/handoff-delivery-redesign/.superpowers/sdd/2026-10-11-handoff-delivery-redesign-finish/resume-v090-mod-independent-oracles-proposal.md` | `reports/resume-v090-mod-independent-oracles-proposal.md` | 3282 | `18f3c772d54bf6545242a9643836e7e7fdd94447cfce0e2d9d103214798f64b8` |
| `/Users/alepar/AleCode/herdr-threads/.worktrees/handoff-delivery-redesign/.superpowers/sdd/2026-10-11-handoff-delivery-redesign-finish/resume-v090-fixture-repairs-quality-review.md` | `reports/resume-v090-fixture-repairs-quality-review.md` | 9545 | `8d07641da798bc772d1c8a9c7be3b656db834318c4076fbd0e61175592e0311c` |
| `/Users/alepar/AleCode/herdr-threads/.worktrees/handoff-delivery-redesign/.superpowers/sdd/2026-10-11-handoff-delivery-redesign-finish/resume-main-565def86-composition-impact.md` | `reports/resume-main-565def86-composition-impact.md` | 8606 | `ac0a8c93547e291313d133e230894728636fd254381eb2d180468241054f2dfa` |

No huge `list.json` was copied. Only new evidence paths were written. Existing run documentation, source files, index, author/Q3 files and historical evidence were untouched. No Git mutation, Cargo, runtime test, Herdr operation or subordinate agent was used. No process was started by this task.
