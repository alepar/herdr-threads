# Final verification

Candidate `7546897944cd4701674a640e0a7f07c83dfa7266`, tree `59713c1a6f77563711ce6ff4cc91859ef52d8709`. Full command `nice cargo nextest run --locked --all-targets --all-features`: 7653 passed, 48 skipped, 216.369s; scoped process/temp leak check passed. Clippy 0.309s, harness-neutral/default-feature/fmt/diff checks passed. Existing release-proof prebuild used. Raw logs and exact configurations/hashes: `final-verification.json`. Five-release matrix and public retry evidence: `profile-directories.md`.

Initial unprepared run: 7653 passed,48 skipped,428.731s including390.552s release proof. Its build preparation omitted CI's existing --prebuild. A later observation also captured peer nextest/release compilation; no causal production regression is inferred. Retain original measurement: `final-verification-initial.json`.

Seven historical raw logs remain missing; exact-source new checks do not reconstruct them. One historical concurrent-libtest topology failure remains unattributed; required nextest passes do not exonerate its source. Authentication/model activation were not measured by the stand-in matrix. Raw local evidence is retained in this integration worktree for main.

The intermediate prepared sweep had7652 passes and one apply-lane backlog assertion failure in387.486s while peer release compilation was observed. See final-verification-contention.json and apply-lane-investigation.md. The quiet confirmation does not reconstruct its failing interleaving or establish causal attribution; retain the gap with the caller/main qualification owner.
