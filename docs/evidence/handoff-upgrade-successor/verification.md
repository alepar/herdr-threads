# Frozen warning prerequisite: upgrade-test successor

Base prerequisite:8106f5cade8ac9d4c2dd8e9b3281e8df05abd8ab, unchanged. The real warning migration advances schema24 to25. The combined v22 upgrade test still expected24; change only that expectation to25. All later canonical import, attachment and replay assertions remain unchanged.

Raw red.log: selected actual22 upgrade test fails at25 vs24 (exit100), before behavioral assertions. Raw green.log: all44 handoff tests pass,3170 skipped,10.480sec (exit0), including complete actual22 import/replay and private integration fixtures. clippy.log: locked all-targets all-features -Dwarnings exit0. default.log: check-default-features exit0. cargo fmt --check and git diff --check exit0.

Owned leak IDs: baseline0a3c722f-d090-4e23-b5b6-f8c5f38fb15e and RED/GREEN9f769bcc-21ef-4295-a3d0-5c5dab3f60a6; check-no-leaked-processes returned no leaked test processes for both. No full suite, real configs/native model launches/shared-host changes.
