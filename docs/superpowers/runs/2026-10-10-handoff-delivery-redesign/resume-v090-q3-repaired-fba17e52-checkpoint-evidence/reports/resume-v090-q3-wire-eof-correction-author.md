# Q3 raw server natural-EOF correction

2026-10-11. Read-only investigation started while candidate source was frozen during the composed-51 run at parent-recorded `3511fb84`. Read the actual `/private/tmp/ht-C-v090-q3-composed-3511fb84-baseline51/run.log` and the successful raw-server path. Parent later reported exact owned-run cancellation, exit 100, scoped leaks zero under run UUID `bdd0e0c7-6938-469e-b383-19db93367101`, and explicitly authorized only the CLI lifetime correction plus bounded natural-EOF helper control. No Cargo/runtime, Git/index, Herdr or other source changes were made by this author.

## Root cause and evidence

`ServerControl::install` saves `stream.try_clone()` in `control.state.stream`. The worker's local stream drops after the script writes Accepted and returns, but that retained clone keeps the server endpoint open. `run_session` reads/drains successfully and then waits for EOF. `server.finish()`, which shuts down the retained clone, is called only after `run_session` returns. This is a circular lifetime dependency, not lock contention. A receive timeout merely repeats the Watch loop while its supplied `alive` predicate remains true; it cannot invent EOF.

This exact path accounts for the six slow CLI controls in the actual log: all four connected Q3 negotiation controls and the two existing connected presentation controls. Never-connect/unwind helper controls passed because they invoke cleanup before running a successful client stream loop. This finding identifies a fixture regression, not six independent product negotiation defects. Parent-recorded final run result is 33 PASS / 18 FAIL: twelve assertion failures and six SIGINT outcomes, wall time 142.083 seconds (listed test time 124.432 seconds). The source's separate active replacement race failures are actual meaningful frame/health REDs and are untouched here.

## Minimal lifetime correction

`ServerControl::install` now returns a `ServerStreamGuard` owned by the actual worker exchange scope. Its Drop takes and drops the active cancellation clone after normal script completion, an early receive/I/O error, or script panic unwind. The mutex guard is released before the socket clone drops. Existing explicit cancellation still atomically takes/shuts down that same clone, wakes readiness waiting, joins the worker and removes its directory. Dropping a subsequently empty guard is harmless. No handshake, stream or test-run budget was increased, and existing exact raw-byte/no-connection assertions remain.

New exact selector: `cli::watch::tests::q3_raw_watch_server_success_closes_the_socket_before_finish`. It opens the raw helper, sends a known framed request, verifies the helper captured those exact bytes, reads a known framed reply, and requires natural EOF with a two-second client read timeout **before** calling `finish`. Only afterward does it verify temporary-directory removal. The prior fixture would time out at this EOF assertion and then clean up through Drop, rather than hang in a product session. The corrected fixture should complete immediately. Parent owns actual execution; no new PASS claim is made here.

## Quiescent release

Only `tests/cli/watch.rs` changed. Its pre-correction SHA-256 was `23e90adf66944fd1150fbd1578857c7d7db5dcf2917e5a5218d67ea601c79824`; source was checked against that captured snapshot before the authorized write. Released SHA-256 is `d38c7b3e732ea653c030f7213230fffe73cdb42fc0d582f45c8015c363e310e0`. Scoped `rustfmt --edition 2024 --config skip_children=true` and its check passed.

Patch artifact: `resume-v090-q3-wire-eof-correction.patch`, 92 lines, SHA-256 `3df0f38f5bab6a84744f314c100a44a3cc16dd589127b5e19b295b1134aa4609`. Before/after snapshots remain ignored WS artifacts. The file is quiescent and released to the parent; production registry source remains owned by its architecture author.
