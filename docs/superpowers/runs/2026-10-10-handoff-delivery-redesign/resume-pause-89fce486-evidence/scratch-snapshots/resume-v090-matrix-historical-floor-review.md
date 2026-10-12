# Historical matrix floor helper review

**PASS — scoped static review of the proposed helper patch.** The patch closes the removed-author-test selection gap by enforcing all 657 individual historical keys and their provenance, without weakening original391/Main244/current-author selection, execution classes or final source gates. This is not final source clearance, a seal, a runtime verdict or permission to substitute an interim count for refreshed final selection.

Reviewed on 2026-10-11. No helper main/function, Cargo, product runtime, Git command, Herdr command or nested agent was executed. No candidate source, helper, inventory or seal was written. Only this review report was created. Pure metadata checks read JSON, applied the diff in memory, and parsed the resulting Python AST without executing it.

## Exact inputs

| Artifact | SHA-256 |
| --- | --- |
| `resume-v090-matrix-historical-floor.patch` | `79865e314cd3f8ed752a94c003e86359a4002bcb60a02037cfc6239b7775a5ff` |
| Original `resume-v090-prepare.py` | `eb83ebd3e8cf7f68fdc08cfb53cca12096c896296b32191dced950bd3c556484` |
| Original `resume-v090-run.py` | `dce287cba7abcd89e0a0fce0ad63af50ac2bf1e333deff5391f7c4d09cbc267f` |
| Pinned `resume-v090-9a573c3f-final-inventory.json` | `c433673ebf52ce5d64b4352f8b9f6d512dcad60edb3a66d5034a7b22f699f3a8` |
| Original `resume-composed-matrix-preparation-f330bdab.json` | `d568da428d23aaa0b517d0ab6a656e0f1684634fdf18f4281761b12a35546fad` |
| `resume-v090-matrix-historical-floor-rationale.md` | `9bde3cd2537b882284ecfcc03816bdb18e9f91b69a7d98faf0c4d6fe4936def8` |
| `resume-v090-final-matrix-preparation-audit.md` | `22364926bee4caa52dc66a8b284667872c9d4476c84fd1bef7c083862f59aaa4` |

All ten diff hunks match the two original helper files exactly. In-memory successor SHA-256 values are prepare `d71e0c61650a32776d7031b85fbc6a4ca34e5d273f0c0281b89e0ec96862f514` and runner `70a1c545ac1dda7b103f063ca11cb16539dc4d061ee4095d386c7ef4484fb5cf`; both parse as Python. These successors were not written or run.

## Selection and provenance

The actual pinned inventory contains 657 rows and 657 unique target/name keys: 651 `focused_nextest`, six `mandatory_cargo_separate`. Every original391 exact key is present. All 657 rows contain the historical path, file/function SHA-256, tree, registration chain and selection fields carried by the new trace map. These are direct JSON observations, not assertions inferred from a total count or the audit's reported current737 selection.

Preparation adds normalized historical657 keys to the existing union of original391, published Main244, current composed-author differences and the two reviewed canonical dispositions. Existing restored availability/required negative handling remains intact. Each resulting key still passes `current.one`, which requires exactly one actual current source registration, and ignored registrations remain refused. Removing a historical author-only test can no longer erase it merely by removing it from `actual_author`: it remains required by the pinned floor and fails unique resolution. New current-author tests remain additive.

Historical proof is copied from the immutable hash-pinned artifact; current file/function/registration proof is still generated from the selected current tree. The patch correctly does not demand that edited current tests retain historical function hashes, nor relabel historical hashes as current proof. Original391 trace rows remain separate, and the existing two reviewed semantic dispositions are neither expanded nor weakened. The floor uses that same normalization and preserves its original class for every mapped current key.

The new runner independently pins the artifact bytes, requires exactly 657 unique complete historical trace keys, compares each historical source proof field against the pinned row, checks each canonical mapping against the existing reviewed dispositions, and requires a current selected key with the unchanged historical execution class. Both inventory and seal must name the same pinned path/hash. These checks run before evidence-directory creation and Cargo. The historical hash is recorded in runtime provenance and rechecked by the existing frozen-stage checks. Missing trace/key, fabricated source proof, changed class or missing input pin fails closed in the inspected code; this reviewer did not execute mutation fixtures.

## Scope and execution gates

There is no full-suite expansion. The five target roots, exact binary/name predicates, exact enumeration comparison and focused-only nextest execution remain unchanged. Source enumeration may inspect many declarations to establish registrations, but execution uses concrete selected names. The six original recovery cases retain their separate `nice cargo test --locked --all-features --test combined NAME -- --exact` invocations; none is folded into the focused class. The patch does not change their source wait settings or runtime commands.

Preparation still requires explicit `--reviewed-final-commit` for final mode, matching current HEAD/tree, a clean tracked/index state, no merge state and no pending required negative control. Otherwise it produces preparation-only metadata. Runner final-phase validation still rejects preparation-only/staged identities before Cargo; `check_source` retains exact commit/tree, clean state, source/blob hashes and configuration checks, and frozen stages retain source/index/binary comparisons. The patch does not manufacture parent review: the final-commit argument remains an explicit parent attestation, and the parent must actually finish source review before supplying it.

The audit's 737 (731+6) is an observed interim source count, not a new fixed floor or a runtime result independently reproduced here. After remaining work is committed and reviewed, regenerate a fresh uniquely labeled inventory/seal from that exact clean source. Require all historical651 focused plus six separate cases and all current required additions; do not reuse the historical final seal, force737, or interpret metadata preparation as execution success. Existing scoped leak and execution-result accounting remain necessary for a final runtime PASS.

No blocking finding for this helper-only patch. Parent application when quiescent and final refreshed artifact review remain outstanding.
