# Q1 first-correction evidence manifest

Exact evidence copies only; source files were preserved. Each destination was created under this run directory and verified byte-equal to its source with the SHA-256 shown below. No tests or gates were executed by this evidence task.

Recorded results:

- Six-control first correction: HEAD `5a7ce5a3a1e89d95f29582d77cbd0240feb48c20`, tree `268aa3fd79a206f0de0968f8152e355afb9e22c9`; selection exit 0, six passed (719 skipped), run exit 0, scoped leak check exit 0. This does not supersede the independent FAIL reviews.
- Cancellation causal baseline: HEAD `89a430fb75859e7a273f8629f8fd9e1c2962be93`, tree `b75e77e0caa8341f136e3adea412a8bf21c1a61f`; selection exit 0, two failed (724 skipped), run exit 100, scoped leak check exit 0. Cancellation control reports a cancelled terminal continuation committed private progress; sibling control fails with `StoreBusy` / malformed JSON. The sibling failure is recorded as observed, not attributed to the cancellation defect.
- Source gates for staged tree `268aa3fd79a206f0de0968f8152e355afb9e22c9`: fmt, neutral-core, all-feature clippy and default-feature checks each exit 0; aggregate exit 0.
- Independent specification and quality reviews are copied unchanged, including their FAIL conclusions for cancellation preservation and sibling-control adequacy. Their scoped static passes are not a complete-specification pass.

| Source | Destination (relative to this directory) | Bytes | SHA-256 |
| --- | --- | ---: | --- |
| `/private/tmp/ht-C-v090-continuation-six-first-correction/execution.json` | `v090-5a7-continuation-six-first-correction-execution.json` | 3368 | `addd7dc3e68f961fb1cc4abb96bf419324abcd60819b133ce9a3df31214e5bb8` |
| `/private/tmp/ht-C-v090-continuation-six-first-correction/run.log` | `v090-5a7-continuation-six-first-correction-run.log` | 1301 | `6d7abae146380dc3db90227a3e181b1a8b4df51c0380bd79b82a0838bbb066fb` |
| `/private/tmp/ht-C-v090-continuation-six-first-correction/list.stderr` | `v090-5a7-continuation-six-first-correction-list.stderr` | 220 | `ebf8287443ce466d6cf18c7ece68eadc924b89cbbdd02a3d0f79248150ef6e12` |
| `/private/tmp/ht-C-v090-cancellation-causal-baseline/execution.json` | `v090-89a-cancellation-causal-baseline-execution.json` | 1825 | `471af687b2e67e02eb2c9578b95f54ca6208119aba15478fe54039d0ee7aeae2` |
| `/private/tmp/ht-C-v090-cancellation-causal-baseline/run.log` | `v090-89a-cancellation-causal-baseline-run.log` | 3575 | `9f95d26c07bf4922fc8f42e483468cbd3d7f86c0b9283822a18ad64e0f1b9764` |
| `/private/tmp/ht-C-v090-cancellation-causal-baseline/list.stderr` | `v090-89a-cancellation-causal-baseline-list.stderr` | 219 | `d33c4f0f622c7100493d0661173aa8ac8ca906790271d98e3ee126d2a58fd8a0` |
| `/private/tmp/ht-C-v090-continuation-correction-gates/execution.json` | `v090-268-continuation-correction-source-gates.json` | 1627 | `f81c889a1c1cb5c7335350c919d782a3851036166a9743704ab1d163e28ef242` |
| `/Users/alepar/AleCode/herdr-threads/.worktrees/handoff-delivery-redesign/.superpowers/sdd/2026-10-11-handoff-delivery-redesign-finish/resume-q1-continuation-spec-review.md` | `resume-q1-first-correction-spec-review.md` | 10618 | `be0170faadedfce0c85b57edd634f0f8b2d85c19d9f65f4ad12d14769c5f78ec` |
| `/Users/alepar/AleCode/herdr-threads/.worktrees/handoff-delivery-redesign/.superpowers/sdd/2026-10-11-handoff-delivery-redesign-finish/resume-q1-continuation-quality-review.md` | `resume-q1-first-correction-quality-review.md` | 9421 | `091764aa0ffdeb683a26a6da4ee2a8fca332fb358d4aa19e0cc28a47cd3a4cdb` |

No `list.json` was copied. Existing run documentation, source files and the Git index were untouched.
