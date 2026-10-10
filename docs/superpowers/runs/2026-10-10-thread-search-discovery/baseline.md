# Baseline and reproduction

Base: f3431b310d90595c19f7d37aeef887f3c15bb018 (main).

Live read-only reproduction used verified routing instance 51a03f2a-3c7e-46f5-b694-3aed374557d7:

- `herdr-threads thread list --all --search psa-global`: no rows.
- `herdr-threads thread show teW1XBj6v`: active thread, name psa-global, topic important system wide announcements.
- `herdr-threads thread list --all --search 'important system wide'`: active teW1XBj6v and archived t06xmRo08, both named psa-global.
- Source: directory query matches only `topic.contains(needle)`; picker uses case-insensitive subsequence matching on combined name/topic.

Focused baseline: `CARGO_TARGET_DIR=/Users/alepar/AleCode/herdr-threads/target nice cargo test --locked --all-features directory_`.
Initial fresh-worktree compilation took 3m53s. Library 37/37 and combined 7/7 passed. One service test failed because the sandbox refused its isolated daemon socket bind (`PermissionDenied`).

Exact rerun outside sandbox: `CARGO_TARGET_DIR=/Users/alepar/AleCode/herdr-threads/target nice cargo test --locked --all-features --test service resolution::direct_socket_hook_keeps_selected_check_in_and_directory_continuations`: 1/1 passed in 0.52s. No baseline code failure identified. No full suite run.
