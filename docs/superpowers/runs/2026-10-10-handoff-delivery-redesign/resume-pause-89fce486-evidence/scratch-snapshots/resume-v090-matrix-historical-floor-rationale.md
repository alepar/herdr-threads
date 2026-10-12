# Scratch historical657 floor successor

Status: proposed helper-only patch for parent application and review when quiescent. This is not final source clearance. Original scripts and historical inventories remain unchanged; no final seal, Cargo command or product runtime was generated/run.

Patch: `resume-v090-matrix-historical-floor.patch`, SHA-256 `79865e314cd3f8ed752a94c003e86359a4002bcb60a02037cfc6239b7775a5ff`. Paths are relative to this WS; parent may apply with `patch -p1 -i resume-v090-matrix-historical-floor.patch` from this directory after review. Do not apply in the candidate source directory.

Inputs it applies to:

- `resume-v090-prepare.py`: SHA-256 `eb83ebd3e8cf7f68fdc08cfb53cca12096c896296b32191dced950bd3c556484`.
- `resume-v090-run.py`: SHA-256 `dce287cba7abcd89e0a0fce0ad63af50ac2bf1e333deff5391f7c4d09cbc267f`.
- Historical floor: the actual existing `resume-v090-9a573c3f-final-inventory.json`, verified SHA-256 `c433673ebf52ce5d64b4352f8b9f6d512dcad60edb3a66d5034a7b22f699f3a8`, 657 exact keys (651 focused +6 separate). This is not an inferred original391/Main union.

Existing source-difference discovery can forget a removed historical author-only test. The patch adds the pinned historical keys to the existing preparation union, so `current.one()` rejects missing or duplicate registrations before writing metadata. It records one historical source proof per657 key, separately from the unchanged original391 map. Proof fields retain historical path, file/function hashes, tree, module registration chain and selection class; current source hashes are still freshly computed rather than incorrectly required to equal historical code.

The runner verifies the pinned artifact bytes independently, requires exactly657 unique complete trace rows, verifies each historical source proof against that artifact, and requires each mapped current key with its original execution class. Inventory/seal input hashes must explicitly name the pinned floor. Checks occur before evidence-directory creation or Cargo. Runtime provenance records the floor path/hash and existing frozen-stage checks recheck its hash. Existing mandatory-six validation and separate exact Cargo commands are untouched.

No test domain is expanded: the observed `e12e90df` selection already contains all657; it remains737 (731+6). Current source discovery still adds present Main/composed Q3/Q4/Q5 cases. The patch does not select all5918 parser-discovered registrations, introduce new renames, change current registration logic, relax the final clean-source seal, or generate a final inventory. The parent must refresh after pending source work, with the unique-label commands in the audit.

Validation performed only in memory: both proposed scripts parsed with `ast.parse`; the new pure metadata validator accepted the real657 fixture and rejected five independent mutations (missing current key, missing trace, forged historical function hash, changed execution class, missing input pin). Both original helper contents were compared after scratch creation and remained identical. No helper main, Cargo, source test or runtime was invoked. The patch was generated directly from those originals and reviewed as a unified diff; parent application and whole-helper final preparation remain pending.
