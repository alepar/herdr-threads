# Completed helper worktree cleanup

Final status: **16 authorized completed checkouts removed**, **129,509,318,656 bytes (120.615 GiB)** in allocated files deleted. Net available disk space increased **40,675,422,208 bytes (37.882 GiB)** from the first-removal snapshot to final verification; currently **143,279,779,840 bytes (133.440 GiB)** are available. All branches and commits remain, all protected checkouts and D warm target remain, no authored artifacts required copying, no removals are blocked, and cleanup has stopped.

Removed the six user-authorized completed helper checkouts with normal `git worktree remove`. Their `du -sk` allocations totaled **458,747,904 bytes (437.496 MiB)**. All six helper branches and exact commits remain in the repository.

| Removed path under ROOT/.worktrees | Retained branch | Source HEAD | Checkout bytes |
| --- | --- | --- | --- |
| `finish-watch-owner-replay` | `helper/handoff-resume-watch-owner-replay` | `ac58bbfb03f5` | 76,451,840 |
| `finish-composed-hook` | `helper/handoff-composed-public-ensure` | `f8073334f2fb` | 76,660,736 |
| `finish-composed-commands` | `helper/handoff-composed-requirement-command` | `0d178d5a0bf9` | 76,652,544 |
| `finish-composed-permit-diagnostic` | `helper/handoff-composed-permit-diagnostic` | `e762aecf5115` | 76,677,120 |
| `finish-mod-activation` | `helper/handoff-finish-mod-activation` | `bda5331cd0ea` | 76,099,584 |
| `finish-watch-activation` | `helper/handoff-finish-watch-activation` | `7e315042b7b2` | 76,206,080 |

ROOT: `/Users/alepar/AleCode/herdr-threads/.worktrees/handoff-delivery-redesign`.

Before removal, each exact canonical path was a registered worktree with clean tracked/index state and no untracked or ignored files. No artifact copying or cache-only deletion was required. A read-only `ps -axo pid=,ppid=,command=` scan found no process command containing an allowed helper path. No processes were started or stopped.

The four composed helpers match retained f330bdab imports by stable patch ID: ac58bbfb → 95e7e3bf, f8073334 → 9ff4913a, 0d178d5a → f970d6fa, e762aecf → f330bdab. MOD bda5331 and WATCH 7e315042 match exact changed-file SHA256 at retained D ancestors 7bdb3678 and 9ca1a64d. The manifest records source blob IDs, SHA256, sizes, branch refs, import mappings and ancestry.

Fresh verification confirms all six paths and registrations are absent, retained branch heads are unchanged, all source commits/blobs remain readable, and every explicitly protected checkout is still registered and present. No forced removal, branch deletion, pruning, push, stash, Herdr mutation or runtime/Cargo test was performed. No initially authorized helper is blocked.

Volume available bytes before/after removal: 102,604,357,632 → 103,256,530,944. Concurrent work can change volume free space, so the attributable reclaim figure is the sum of removed checkout allocations; APFS shared extents can also affect physical free space.

Durable evidence:

- [Completed helper manifest](worktree-cleanup-evidence/2026-10-11-completed-helpers-manifest.json)
- [Retained worktree inventory](worktree-cleanup-evidence/2026-10-11-retained-worktree-inventory.json)

Additional retained candidates below are inventory only. Their completion, source preservation and process use need verification plus specific parent authorization before removal. Active D and the other protected paths remain outside the cleanup scope.

| Retained candidate | Checkout bytes | target bytes |
| --- | --- | --- |
| `completed--task-ht-0lo.3.2` | 24,064,253,952 | 23,996,862,464 |
| `super-auto-handoff-delivery-redesign--task-ht-0lo.1.2` | 20,245,176,320 | 20,178,370,560 |
| `super-auto-handoff-delivery-redesign--task-ht-0lo.2.3` | 17,566,572,544 | 17,500,172,288 |
| `super-auto-handoff-delivery-redesign--task-ht-0lo.4.2` | 11,707,822,080 | 11,640,819,712 |
| `super-auto-handoff-delivery-redesign--task-ht-0lo.2.2` | 10,450,698,240 | 10,385,666,048 |
| `super-auto-handoff-delivery-redesign--task-ht-0lo.1.3` | 10,378,407,936 | 10,313,732,096 |
| `super-auto-handoff-delivery-redesign--task-ht-0lo.4.1` | 9,102,729,216 | 9,038,106,624 |
| `super-auto-handoff-delivery-redesign--task-ht-0lo.3.1` | 8,947,929,088 | 8,883,703,808 |
| `super-auto-handoff-delivery-redesign--task-ht-0lo.1.1` | 8,911,814,656 | 8,836,165,632 |
| `super-auto-handoff-delivery-redesign--task-ht-0lo.2.1` | 7,675,166,720 | 7,611,633,664 |
| `finish-advertisement` | 76,292,096 | none |
| `finish-corrected-D-review` | 76,292,096 | none |
| `finish-capacity-repair` | 76,238,848 | none |
| `finish-capacity-review` | 76,210,176 | none |
| `finish-capacity-design` | 76,099,584 | none |
| `finish-hook-activation` | 76,083,200 | none |
| `finish-offer-reclamation` | 76,054,528 | none |
| `finish-private-id` | 76,034,048 | none |
| `finish-schema-proofs` | 76,034,048 | none |
| `finish-interface-tests` | 76,017,664 | none |
| `emit-helper-15` | 75,325,440 | none |
| `js-epoch-helper-15` | 75,268,096 | none |
| `finish-leak-scope` | 69,337,088 | none |

## Expanded completed task cleanup

Removed four additionally authorized completed original task checkouts with normal `git worktree remove`, reclaiming 73,583,824,896 bytes (68.530 GiB) of allocated checkout files, including 73,316,225,024 target bytes. Across all ten removed checkouts the total was **74,042,572,800 bytes (68.958 GiB)**.

| Removed checkout | Retained branch | Source HEAD | Checkout bytes |
| --- | --- | --- | --- |
| `completed--task-ht-0lo.3.2` | `task-ht-0lo.3.2` | `673a27ad3515` | 24,064,253,952 |
| `super-auto-handoff-delivery-redesign--task-ht-0lo.1.2` | `task-ht-0lo.1.2` | `ef048d163d6e` | 20,245,176,320 |
| `super-auto-handoff-delivery-redesign--task-ht-0lo.2.3` | `task-ht-0lo.2.3` | `4bae64390b86` | 17,566,572,544 |
| `super-auto-handoff-delivery-redesign--task-ht-0lo.4.2` | `task-ht-0lo.4.2` | `4d86974cf39f` | 11,707,822,080 |

Each task is recorded completed in run.md/progress.md, each exact source HEAD is an ancestor of retained ROOT and f330bdab, and all four tracked/index states were clean. There were no untracked files or authored ignored artifacts. Reviewed ignored content was standard Cargo target output (empty target/tmp) plus eleven generated CPython .pyc files in task4.2; their source hashes and classification are in the manifest. No artifacts needed copying. A fresh process scan found no exact helper-path command matches.

Fresh verification confirms four removed paths/registrations are absent, all four branches still point to their original source commits/trees, and every protected checkout remains present. Active D, ROOT, V090, f330, automatic-validation, mutant and parked B were preserved. No blocked removal remains.

Volume available bytes for this expanded pass: 102,759,628,800 → 120,797,184,000, a net change of 18,037,555,200 bytes. These checkouts used APFS-cloned caches, so deleted file allocations are not equal to newly freed physical extents; concurrent builds also affect the net figure.

- [Expanded task manifest](worktree-cleanup-evidence/2026-10-11-completed-tasks-manifest.json)

The retained inventory above predates these four removals; entries for the four removed names are superseded by the expanded manifest. Other candidates remain untouched and require additional scoped authorization.

## Final six completed task checkouts

Removed all six final authorized checkouts normally, reclaiming 55,466,745,856 bytes (51.657 GiB) of allocated files, including 55,069,007,872 target bytes. No authored untracked/ignored evidence existed; ignored files were disposable standard Cargo buildcache only. All six process-path checks were clear.

| Removed checkout | Retained branch | Source HEAD | Checkout bytes |
| --- | --- | --- | --- |
| `super-auto-handoff-delivery-redesign--task-ht-0lo.1.1` | `helper-ht-0lo.8.4-relevance` | `367da744905c` | 8,911,814,656 |
| `super-auto-handoff-delivery-redesign--task-ht-0lo.1.3` | `task-ht-0lo.1.3` | `3d6685a6cddd` | 10,378,407,936 |
| `super-auto-handoff-delivery-redesign--task-ht-0lo.2.1` | `task-ht-0lo.2.1` | `62fff194f38f` | 7,675,166,720 |
| `super-auto-handoff-delivery-redesign--task-ht-0lo.2.2` | `task-ht-0lo.2.2` | `f48857d73b18` | 10,450,698,240 |
| `super-auto-handoff-delivery-redesign--task-ht-0lo.3.1` | `task-ht-0lo.3.1` | `84706c6b365c` | 8,947,929,088 |
| `super-auto-handoff-delivery-redesign--task-ht-0lo.4.1` | `task-ht-0lo.4.1` | `a680e33da7dd` | 9,102,729,216 |

The .1.1 directory was reused for the completed relevance helper, with current helper HEAD367da744 and branch helper-ht-0lo.8.4-relevance. Exact hashes of all three source files match the frozen task-15-relevance-corrected-application.json; final task15 report records application and returned source ownership. Both helper branch367da744 and original task-ht-0lo.1.1 branche70091bf remain retained. The other five current source HEADs are ancestors of retained f330bdab; every original task is recorded closed.

Final-pass volume available bytes: 121,414,975,488 → 143,279,779,840 (net 21,864,804,352 bytes). APFS sharing and concurrent builds affect physical free space.

- [Final six-task manifest](worktree-cleanup-evidence/2026-10-11-final-completed-tasks-manifest.json)
- [Final cumulative verification](worktree-cleanup-evidence/2026-10-11-cleanup-final-summary.json)

All sixteen removed branches and commit objects were freshly rechecked after this final pass. No blocked removal remains. Every protected checkout and D warm target still exists; cleanup stops here. Retained inventory entries for these six paths are superseded by this final manifest.
