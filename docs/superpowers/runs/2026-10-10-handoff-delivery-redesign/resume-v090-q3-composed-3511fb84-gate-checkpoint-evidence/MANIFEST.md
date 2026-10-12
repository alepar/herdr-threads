# Q3 composed-source failed gates: recoverable checkpoint

**C remains NOT READY.** Both completed gate failures and the current quiescent author/review snapshots are preserved. No runtime GREEN, complete source clearance or successful current gate is asserted. Earlier actual failed evidence remains unchanged.

## Completed failed source gates

- `ht-C-v090-q3-composed-df1390d0-gates`: staged tree `4b18f5de8e3af3b859499f73bd69e6dbaa820792`, index SHA-256 `6779c123eaa1ae12d711c726db955359dcd3763484d4090852939306682210c8`. fmt and neutral-core checks exit 0; clippy exits 101 after 29.10720229195431 seconds; aggregate exit 101. Compile failure E0277 at two test reads: `u64: FromSql` unsupported in `tests/service/mod_channels.rs`. No test runtime was executed. No default-feature gate ran in this package because the chain stopped at clippy.
- `ht-C-v090-q3-composed-457efc20-gates`: staged tree `cafa438fe32aaeda1aafa396d84707c135433a24`, index SHA-256 `5b173a977d86f45f05405eb7353b0af631ef8abdcc39c61fd425aa75c3083ad2`. fmt and neutral-core checks exit 0; clippy exits 101 after 36.8364707500441 seconds; aggregate exit 101. Lint failure `clippy::type_complexity` in the test snapshot helper in `tests/service/delivery_pull.rs`. No test runtime was executed. No default-feature gate ran in this package because the chain stopped at clippy.

Each gate execution file names a staged tree/index hash, not HEAD or run UUID. Commit labels in package names and the following source history are parent-provided; this task did not invoke Git. Internal command-log hashes were independently matched before copying.

## Source and continuation state (parent supplied)

- Parent composed 27 files at `df1390d0`; final active-send fence, logical Notice replay and ordinary-scanner issues remain held. The first correction committed as `457efc20`; lint correction committed as `3511fb84`, current clean tree prefix `578eb115`.
- `/private/tmp/ht-C-v090-q3-composed-lint-corrected-gates` is RUNNING at dispatch and was not read or copied. Source/index were frozen during the two completed gates; parent reports no test processes. No process scan was performed by this task.
- Dispatch-completion seven-test scratch patch is pending application after the baseline. No pending patch was applied by this task.
- Source code-roast counters remain 0. Scoped static reviews and gate attempts are not a completed formal code-roast or merge qualification.
- Main `565def86`, version `0.9.0`, remains published and installed, schema 35 / protocol 7, per parent-supplied fully displayed `mzvVHEYr2` #1059. Installer issue `m8FBdxFTq` #1056 belongs to `sk8JoyBoJ`; no reassignment. Earlier installation-pending documents remain historical.

## Report snapshots

Wire, reader, registry, store-correction, controls and logical-Notice query proposal/review reports are copied unchanged. Historical REVISE findings, later scoped re-reviews, pending runtime cutoffs and held production defects remain intact. The copied Notice proposal describes unapplied production/query-test artifacts; its static review is not runtime proof. None of these snapshots upgrades C readiness.

## Byte-exact evidence and SHA-256

Every copy below was verified byte-equal to its preserved source and SHA-256 equal. Destination paths are relative to this checkpoint directory.

| Source | Destination | Bytes | SHA-256 |
| --- | --- | ---: | --- |
| `/private/tmp/ht-C-v090-q3-composed-df1390d0-gates/clippy.log` | `ht-C-v090-q3-composed-df1390d0-gates/clippy.log` | 2159 | `16f9235e151aa95f709d6d107350103f2aa9969494a27953ed1840d08addd93d` |
| `/private/tmp/ht-C-v090-q3-composed-df1390d0-gates/execution.json` | `ht-C-v090-q3-composed-df1390d0-gates/execution.json` | 1285 | `3cf1584f15953ce3d7f1a3bd192fcf65c8b4b8b919a826c825b2489a43da8d60` |
| `/private/tmp/ht-C-v090-q3-composed-df1390d0-gates/fmt.log` | `ht-C-v090-q3-composed-df1390d0-gates/fmt.log` | 0 | `e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855` |
| `/private/tmp/ht-C-v090-q3-composed-df1390d0-gates/neutral.log` | `ht-C-v090-q3-composed-df1390d0-gates/neutral.log` | 130 | `7561605afc0af5f27527d99451825bc26c55fa2f7083bb392ad4244e406b0536` |
| `/private/tmp/ht-C-v090-q3-composed-457efc20-gates/clippy.log` | `ht-C-v090-q3-composed-457efc20-gates/clippy.log` | 796 | `7a69bfada2b8b714d290b8525bfb19473d8c70041ced36f90912ec2a4aea7ca7` |
| `/private/tmp/ht-C-v090-q3-composed-457efc20-gates/execution.json` | `ht-C-v090-q3-composed-457efc20-gates/execution.json` | 1285 | `51f32fd1a37493df623b925e855a344feee9a0a39de1349205909de39459e62b` |
| `/private/tmp/ht-C-v090-q3-composed-457efc20-gates/fmt.log` | `ht-C-v090-q3-composed-457efc20-gates/fmt.log` | 0 | `e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855` |
| `/private/tmp/ht-C-v090-q3-composed-457efc20-gates/neutral.log` | `ht-C-v090-q3-composed-457efc20-gates/neutral.log` | 130 | `7561605afc0af5f27527d99451825bc26c55fa2f7083bb392ad4244e406b0536` |
| `/Users/alepar/AleCode/herdr-threads/.worktrees/handoff-delivery-redesign/.superpowers/sdd/2026-10-11-handoff-delivery-redesign-finish/resume-v090-q3-wire-spec-review.md` | `reports/resume-v090-q3-wire-spec-review.md` | 7763 | `96fd6bdce2a5dfcbef66d4ca82ed8130048ac7edc0d9eb320f1c0e10ce4db9b3` |
| `/Users/alepar/AleCode/herdr-threads/.worktrees/handoff-delivery-redesign/.superpowers/sdd/2026-10-11-handoff-delivery-redesign-finish/resume-v090-q3-wire-causal-corrections-author.md` | `reports/resume-v090-q3-wire-causal-corrections-author.md` | 10564 | `c2c0e08358266412032cd4f32dea301b0499100512b2d638a625986c69be8519` |
| `/Users/alepar/AleCode/herdr-threads/.worktrees/handoff-delivery-redesign/.superpowers/sdd/2026-10-11-handoff-delivery-redesign-finish/resume-v090-q3-reader-invalidation-author.md` | `reports/resume-v090-q3-reader-invalidation-author.md` | 4874 | `4fddc5d27e9f8455076ee13d97b4cb5590d0662110a529a605c0f39b060d2a87` |
| `/Users/alepar/AleCode/herdr-threads/.worktrees/handoff-delivery-redesign/.superpowers/sdd/2026-10-11-handoff-delivery-redesign-finish/resume-v090-q3-reader-quality-review.md` | `reports/resume-v090-q3-reader-quality-review.md` | 6510 | `25a05a9d10d98abc44b8de6da4bc7a93d01164c6eb2ba1b85e7ed0a68707eecd` |
| `/Users/alepar/AleCode/herdr-threads/.worktrees/handoff-delivery-redesign/.superpowers/sdd/2026-10-11-handoff-delivery-redesign-finish/resume-v090-q3-registry-implementation-author.md` | `reports/resume-v090-q3-registry-implementation-author.md` | 7726 | `aae68eb9b97ed7ca0761951507ac338077abdf3ea1c60770a6f7acd215675633` |
| `/Users/alepar/AleCode/herdr-threads/.worktrees/handoff-delivery-redesign/.superpowers/sdd/2026-10-11-handoff-delivery-redesign-finish/resume-v090-q3-registry-spec-review.md` | `reports/resume-v090-q3-registry-spec-review.md` | 9747 | `b111ef23a710d6c2bf44160955a64b6ffc9adbd2117c3b09003296c8a198a9a3` |
| `/Users/alepar/AleCode/herdr-threads/.worktrees/handoff-delivery-redesign/.superpowers/sdd/2026-10-11-handoff-delivery-redesign-finish/resume-v090-q3-registry-quality-review.md` | `reports/resume-v090-q3-registry-quality-review.md` | 8324 | `9cac9ee8a69f8c290295d6ea6b280ba22ee0aeb11bd67f778d70ffe9bbe8309d` |
| `/Users/alepar/AleCode/herdr-threads/.worktrees/handoff-delivery-redesign/.superpowers/sdd/2026-10-11-handoff-delivery-redesign-finish/resume-v090-q3-store-causal-corrections-author.md` | `reports/resume-v090-q3-store-causal-corrections-author.md` | 8178 | `8113e5d3e964838bb5a4e629231f507c2e168860e19a7dec882942786cb49ff7` |
| `/Users/alepar/AleCode/herdr-threads/.worktrees/handoff-delivery-redesign/.superpowers/sdd/2026-10-11-handoff-delivery-redesign-finish/resume-v090-q3-controls-spec-review.md` | `reports/resume-v090-q3-controls-spec-review.md` | 19418 | `47534d8afccdcf068c15b9d84dbd483e7510cb261b3037e93d665ee2da5f8d0d` |
| `/Users/alepar/AleCode/herdr-threads/.worktrees/handoff-delivery-redesign/.superpowers/sdd/2026-10-11-handoff-delivery-redesign-finish/resume-v090-q3-controls-quality-review.md` | `reports/resume-v090-q3-controls-quality-review.md` | 12179 | `53b33a004fc36594fe62fe675bf7626623b8cbbc5ee62f3fd3a719ab9a0d6187` |
| `/Users/alepar/AleCode/herdr-threads/.worktrees/handoff-delivery-redesign/.superpowers/sdd/2026-10-11-handoff-delivery-redesign-finish/resume-v090-logical-notice-replay-proposal.md` | `reports/resume-v090-logical-notice-replay-proposal.md` | 13160 | `4cff9711652c139bb91f89e177c51f41a110b2e7db3b2e50a837e604a5af74cd` |
| `/Users/alepar/AleCode/herdr-threads/.worktrees/handoff-delivery-redesign/.superpowers/sdd/2026-10-11-handoff-delivery-redesign-finish/resume-v090-logical-notice-replay-review.md` | `reports/resume-v090-logical-notice-replay-review.md` | 6594 | `8a4140277d7a406c94ee1ebea5b8f7c040830f8d9068828b5c667bee29edd88b` |

Only new evidence/documentation paths were written. No existing report was overwritten. No source/index/Git mutation, Cargo/runtime test, Herdr command, subordinate agent or process launch occurred. QUIESCENT.
