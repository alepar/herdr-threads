# Final verification

Source tip: 55bc7cea8952a1f68f458c3c2d70a1c39884d711, the exact PR-roast input. Subsequent finalization changes only run evidence and design index/notes; verify with `git diff 55bc7cea HEAD -- src tests integrations TRUST-POLICY.md Cargo.toml Cargo.lock scripts` (empty).

Passed command (131 tests, 1.32s):

```sh
nice cargo test --locked --all-features --lib -- store::control::tests::invitation_rejection store::control::tests::rejection_retains store::materialization::tests:: store::receipts::tests:: cli::skill::tests cli::commands::tests::rejection_requires_exact_invitation_and_bounded_nonblank_reason cli::journal::tests::invitation_rejection store::cooperative_checkin_tests::builtin_warning_dedup store::cooperative_checkin_tests::programmatic_notices_settle_page_by_page store::connection::tests::delayed_warning_attribution store::connection::tests::invitation_warning_clear store::queries::tests::inbox_batch_keeps_open_and_clear store::queries::tests::active_warnings_union store::attention::tests::saturated_other_seat_transitions
cargo fmt --check
git diff --check
nice cargo clippy --locked --all-targets --all-features -- -D warnings
nice scripts/check-default-features
```

Every command exited 0. Package manifest resolved inside this integration checkout. nice emitted a sandbox priority warning; Cargo emitted no compiler warnings. No full suite or helper daemons were run.
