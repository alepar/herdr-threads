# Named startup currentness (ht-kye2)

A first named allocation could return before its observation worker published the new name. With an already published empty snapshot and background scheduling paused, a valid immediate lifecycle check-in then failed `CallerUnverified: mapping held or known invalidated`. This is a general named-host synchronization defect, independent of any harness or the earlier host sequence correction.

The sections through the initial validation below record the original correction and its historical evidence. [The S1–S3 follow-up](#s1s3-follow-up-seat-local-currentness) describes the current implementation: stable resolves read canonical currentness without republishing, and unrelated named lifecycle changes cannot reject a still-current owner.

## Correction and authority

The daemon resolves or replays the journaled allocation, releases its allocation writer turn, then uses the existing bounded `with_observation` current-target publication path before returning success. A narrow read-only canonical named-target check verifies the returned owner against the just-published admission, complete baseline, host boot/epoch/incarnation, target order and generation, local live named mapping, retirement, recovery holds and post-publication lifecycle revision. No mapping or binding is allocated, repaired, moved or released by that check.

A new named row has not yet acquired the saved `structural_*` proof established by complete named enumeration. The stronger ordinary target-owner/launch guard requires that proof and remains unchanged. The new internal store port instead validates the existing cooperative named ownership model against canonical currentness; its default is Unsupported. The production SQLite implementation checks one query snapshot and rolls it back read-only. Unsupported stores cannot report admission-ready success. No wire field, capability, provenance, caller verification or admission exemption is added. TRUST-POLICY A2/A3 and the existing permit path remain in force, so no normative amendment is needed.

A replay retains the original historical allocation result (including `created`), but must pass a new currentness check. An old result cannot claim a retired owner or replace its successor. If the initial complete snapshot is missing, the existing capture wait requests the observation lane and is bounded by the request budget and existing capture cap; failure retains the journaled allocation for recovery by the same operation. The fix adds no sleeps, blanket retries or harness workaround, and changes no lane-lock implementation.

## Core accounting

| Core path | Change | Default / unchanged behavior | Compatibility evidence |
|---|---|---|---|
| `src/identity/repair.rs` | Named allocation/replay waits for current-target publication and canonical check after releasing writer | Other ordinary resolution, continuity and target checks unchanged | Immediate lifecycle, bounded first-baseline recovery, stale replay regressions; existing named/tmux cases |
| `src/ports.rs` | Internal `check_named_target(ResolvedTargetCheck)` | Default Unsupported; existing strong target check unchanged | Existing store implementations compile without new override; shipped SQLite overrides only the named check |
| `src/store/mod.rs` | Route named currentness through a read-only query transaction | Permit issuance and check-in registration unchanged | Real DomainService permit/check-in regressions and existing cooperative tests |
| `src/store/seats/resolution.rs` | Read canonical named ownership after exact shared publication validator | No saved-proof relaxation, allocation, hold release or replay mutation | Retired owner/successor regression and existing publication/target tests |
| `src/service/dispatch.rs` | Register the unit regression module only | Production dispatch unchanged | Real production resolve and CheckIn dispatch in regression |
| `tests/service/named_startup.rs` | Real named host/store/coordinator/service fixture with controlled background scheduling | No ambient config, process environment, service or host mutation | Causal RED/GREEN below |

## Causal validation

The fixture uses the real `NamedHost`, `SqliteStore`, `OrdinaryIdentity`, shared fair writer, scheduled observation-store adapter and `DomainService` permit path. A prebuilt host session deliberately leaves worker scheduling paused; only the fixture's explicit release runs a production capture. Its empty baseline is produced, stamped and published by the real non-closing named observation path, not fabricated SQL evidence. There is no manual observation, timer, retry or background capture between the critical resolve and lifecycle check-in. The attached-host index uses the standard fixture publication hook. Scratch is owned by `TestIsolation`; no child or service process is spawned.

Initial fixture execution exposed a missing attached-host index entry and is not credited as causal evidence (build 3m09s, run0.33s). After correcting attachment, `heavy tb env HT_LEAK_RUN_ID=named-startup-20261114 cargo test --locked --all-features --lib named_startup_tests -- --nocapture` failed all three regressions (build52.86s, run0.33s): immediate lifecycle returned `CallerUnverified: mapping held or known invalidated`; absent-baseline resolve incorrectly returned success; retired allocation replay incorrectly returned the old successful allocation. These failures were observed before production edits.

The corrected source is frozen in `f61702105708cc81a40d9725402df3fb4fde4077` (parent `1942d2edd`); the source commit contains only the six core/test paths listed above. The evidence-only note follows separately. The coordinator owns its later placement ahead of harness integration history.

- GREEN, same regression command: 3/3 passed (build2m11s, run0.41s). The empty-baseline path succeeds through unchanged lifecycle permit/registration, then subsequent capture retains one seat and one binding. With no baseline and no scheduled capture, the 40ms request returns a transient refusal within a 1s hang guard, creates no binding, and the same operation recovers the same allocation after production capture. Retired replay is refused while the successor stays resolved.
- `heavy tb env HT_LEAK_RUN_ID=named-startup-20261114 cargo nextest run --locked --all-features --lib --status-level fail --final-status-level fail -E 'test(named) | test(tmux) | test(current_target) | test(cooperative_tests::)'`: 355/355 passed (build0.02s, run12.624s). Existing named/tmux/current-target/cooperative tests were not edited.
- `heavy tb env HT_LEAK_RUN_ID=named-startup-20261114 cargo nextest run --locked --all-features --test named_seats --status-level fail --final-status-level fail`: 12/12 passed,0 skipped (build1m27s, run21.176s). The unchanged target runs real private daemons and isolated host fixtures, covering the ordinary worker-enabled named startup path.
- `heavy tb cargo clippy --locked --all-targets --all-features -- -D warnings`: passed,58.09s.
- `scripts/check-harness-neutral`: passed,0 violations,90 allowlisted occurrences,55 entries.
- `scripts/check-wire-compat`: passed,0 violations,0 stale,456 fields,24 variants,274 baseline entries. `git diff --check` passed.

Every Rust/test/build command uses the required `heavy tb` wrappers, one existing seeded target and default job/test caps of four. No full suite, stress/repeat gate, new target, credential/profile access, shared service mutation or harness production edit is part of this fix. Compile times above are recorded separately from runtime; no incremental-build qualification is claimed.

The existing observation-lane mutex remains unchanged. The deterministic pause withholds worker scheduling before lane acquisition; it does not simulate an unbounded host read already holding that mutex. The missing-capture wait's bounded refusal is proven, while no new global deadline guarantee for pre-existing blocked host/lane behavior is claimed. No extra locking infrastructure was introduced.

The only new refusal reason is the internal Unsupported default's 24-byte `named target currentness`, wrapped by the existing error format. Successful public text is unchanged; mapping/publication refusals reuse existing diagnostics and are bounded. No per-message or per-turn model text is added.

- `heavy tb scripts/check-default-features`: exit0, quiet success; the script emitted no elapsed timer.
- `heavy tb cargo fmt --check`: exit0, quiet success on exact source HEAD `f61702105708cc81a40d9725402df3fb4fde4077`.
- `heavy tb scripts/check-no-leaked-processes --run-id named-startup-20261114`: exit0; `no leaked test temp files` and `no leaked test processes` (0.28s wrapper call).

All required scoped gates passed. Retained private completion excerpts are explicitly labeled excerpts from actual tool returns rather than raw tee logs; original commands were not teed. The public note contains sanitized commands/results without machine paths. The existing seeded target is retained for coordinator qualification; every private test process and scratch lifecycle is finished. Full-suite and final integrated qualification remain coordinator-owned.

## S1–S3 follow-up: seat-local currentness

Independent review found three defects in the initial correction. The named post-publication check required the entire host's lifecycle revision to equal admission plus one, so an unrelated name's allocation or retirement could reject an unchanged owner. Every resolve and historical replay also published a new observation and requested an explicit capture, advancing global send-eligibility and audience fences. Finally, a structural named observation probed tmux focus even though its consumers never read that focus value.

The current named path first reads the exact canonical local, resolved, nonretired seat/name/target owner and its seat generation. It checks the effective observation against the seat's saved structural proof, the complete active snapshot, current host boot/epoch/incarnation, invalidation and recovery holds. A valid observation returns before entering the observation lane or requesting a capture. The allocation operation still retains its historical result and `created` value; that result supplies no currentness authority.

The saved proof is a publication freshness floor. A new allocation has no proof; named reattachment clears all seven proof fields atomically with its generation transition. The existing lifecycle-fenced current-target publication now stamps a missing named proof, as a complete named snapshot already does. A snapshot captured before allocation or reattachment cannot stamp the successor because its publication lifecycle fence fails. Missing evidence, including after a failed first publication and a different operation key, therefore uses the existing bounded publication path.

Freshness orders `(host epoch, observation sequence)` lexicographically. A newer daemon epoch is fresh even when its sequence restarts below the saved sequence, provided the fixed named boot/incarnation/terminal identity matches, both connection epochs match their own host epochs, the effective epoch is the canonical current epoch, and the structural sources are recognized native-current-target or coherent-enumeration evidence. This includes resolved seats with an old complete proof and no open binding to carry.

After a slow-path publication, the existing exact admission/effective-observation validator remains mandatory. The final check pins the seat generation read before observing and retains local ownership, name/target/generation, boot/epoch, retirement and hold checks. Only the redundant global post-publication lifecycle equality is removed. An own-seat unresolve/reattach transition still refuses the old generation. Ordinary resolution and launch guards are unchanged.

Structural named observations now report `focused=false` without inspecting tmux bindings. The actual wake gate retains its pane-mode, focus and focused-empty safety reads immediately before typing. No wire, provenance, caller, permit or receipt authority changes; TRUST-POLICY A2/A3/C1 remain in force, so no normative amendment is needed. Successful public text is unchanged and no model-visible text is added.

| Changed neutral core path | Follow-up change | Default / preserved behavior | Focused evidence |
|---|---|---|---|
| `src/identity/repair.rs` | Canonical named currentness read before observation; carry captured seat generation into slow-path check | Ordinary resolution, continuity and launch checks unchanged | Real service fresh-operation/replay, cursor, sealed-send and original startup regressions |
| `src/ports.rs` | Internal `NamedTargetStatus`, `NamedTargetCheck` and read-only status port | Unsupported default refuses; ordinary `ResolvedTargetCheck` unchanged; no wire shape | Shipped SQLite and real service tests; existing implementations retain a default |
| `src/store/mod.rs` | Read-only named status transaction and typed slow-path check | No allocation or permit issuance on the read path | Real service cursor and sealed preparation survive repeated resolution |
| `src/store/seats/resolution.rs` | Seat-local generation check, proof floor and reattachment proof clearing | No ownership/hold/retirement relaxation; historical result bytes retained | Unrelated allocation/retirement succeeds; own ABA, retired predecessor, hold, invalidation and failed-publication negatives |
| `src/store/seats/observation.rs` | Stamp a missing named proof during genuine fenced publication; expose existing proof reader internally | Other host proof, publication, eligibility and closure behavior unchanged | Stale snapshot cannot stamp a replacement; restart without an open binding accepts a newer epoch; existing named presence cases |
| `src/host/named.rs` | Structural observation omits unused focus probing | Wake-time focus/pane checks and fixed structural identity unchanged | Portable binding-probe tripwire, structural-focus case and five unchanged isolated tmux focus tests |

### Follow-up causal and focused evidence

All commands use the existing capped `heavy tb` runner and warm target; job and test caps remain four. Compile time is recorded separately and is not an incremental-build benchmark.

- Causal RED: one `cargo nextest run --locked --all-features --lib` selection of the three new regressions failed 3/3 as expected (build 2m19s; run 0.761s). Unrelated allocation produced `TargetUnresolved`; unchanged resolution advanced lifecycle `2→3`, eligibility `4→5` and admission/decision sequence `3→4`; structural observation entered the tmux-binding process read.
- Initial focused GREEN attempt: 34/35 passed (build 2m54s; run 2.155s), including all three causal regressions, actual participant-cursor continuation and sealed-send publication, original immediate lifecycle/no-baseline/retired-replay cases, failed publication/new operation, stale snapshot, retired predecessor, newer epoch with no binding, and five unchanged isolated tmux focus cases.
- The remaining new test expected one generation increment after both production unresolve and reattach; each transition increments generation. Its single expectation was corrected from `+1` to `+2`. The post-publication refusal still uses the captured pre-transition generation. The corrected case passed in the coordinator's combined focused run (0.307s; shared build 1m33s, combined runtime 1.471s). That combined run deliberately included four unrelated expected-RED delivery cases; it is not credited as an entirely green run. All 35 named-focused cases now have passing evidence across these two runs, against unchanged production source.
- Independent review conditionally approved the exact source and verified the one-line test delta. Required combined clippy/default-feature/format/neutral/wire and final execution gates remain coordinator-owned and pending here.
- Both owned RED and focused-run leak checks passed: no leaked test processes or temporary files (0.294s and 0.300s wrapper calls). The coordinator's combined corrected run also passed its scoped leak check. The owned private tmux fixtures were stopped and reaped.

The first S1 regression exercises actual canonical publication followed by an unrelated allocation and retirement before the final check. It pins the store decision; it does not claim to pause a runtime coordinator thread. The no-focus tripwire proves the structural path does not inspect a held tmux binding, while the isolated tmux cases separately preserve real wake-time focus behavior. No full suite, live/native harness, real profile, shared service or new target was used for this follow-up.
