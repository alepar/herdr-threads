# Base composition before implementation

Merged main6107075f (v0.6.3) into the settled design branch after design clearance. No conflicts; the specs index clean merge retains both runs. Reviewed source delta from the original7c40 baseline: literal name/topic search and filtered-directory cursor name revision, supporting control revision bump, CLI help and protocol comment. No receipt, invitation, launch or hook behavior changed upstream. Implementation must retain this discovery behavior while editing overlapping CLI/protocol/store files. Final code roast and readiness checks cover this combined candidate.

## Authorized v0.6.4 composition after Task6

Upstream main `bcb49485a4dd72837bef311d0065a0f7af477523` composed as `bfe1044e3755e6aba1f3c860db6ec0751617cbfe` after gated Task6 leaf merge54796f2d. Unique upstream commits f674333f and bcb49485 change only release metadata and Claude-mod delivery logging/docs/exact assertions. Preserve ``io.log(`delivered ${ids.length} message(s)`)`` without the duplicated herdr-threads prefix on later Task9 composition. All seven composed files byte-equal upstream main; source/schema contracts unchanged. Parent approved no extra routine full checks; final PR roast/readiness covers combined candidate.
