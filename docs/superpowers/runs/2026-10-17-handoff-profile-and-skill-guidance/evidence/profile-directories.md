# Profile directory integration sweep

The terminal sweep found the selected-directory path connected from public parsing
through canonical admission, tab creation, guarded launch and frozen replay. No
additional behavioral repair or duplicate integration case was needed. This file
records integration evidence; final review and the default-feature/full-suite
gates remain with the coordinator.

## Source and coverage

Sweep base: `2d00081eedf5af0b52f97acc324766020fe3c188`, tree
`408f57edaa2f8a9083aeac093fb791726a162d07`. This merges the configuration-smoke
leaf onto the integration branch containing the native tab transport repair.
The code and fixtures match the five-release measured tree
`4a0ffd863daf1d8491c270208ab1f147a9a98b10`; the sole difference is 25 lines in
`docs/validation/herdr-release-matrix.md` recording the results.

| Boundary | Implementation and existing integration coverage |
| --- | --- |
| Public admission | `handoff_delivery` and `commands` reject pane/existing options and invalid paths before effects; `topology_runtime` checks the directory capability before automatic resolution. Parser tests cover canonical paths, symlinks, quotes, spaces and invalid inputs. |
| Captured selection | `topology_handoff::capture_directories` asks the registered `LaunchPolicy` for its projection, validates it and checks existence. Both routing modes save the context and exact derived launch. |
| Canonical identity | Explicit and automatic bootstrap validators require exact cwd/env/context agreement. Absent context retains historical bytes and empty-env restrictions. Derived-child and retained-report comparisons preserve the context. |
| Native tab creation | Both routes pass frozen cwd/env/context to `CreateTabRequest`; `NativeCli` validates exact agreement before the transport serializes the values. Native19 closes the previously discovered nonempty-env refusal. |
| Guarded child | `execute_guarded_inner` compares the projection with the selected adapter, overlays its captured environment/cwd and uses that snapshot for scope, hooks, fingerprints and prelaunch. Existing executable/PATH and configuration rechecks remain. |
| Frozen retry | Public unfinished-retry tests change caller profile/cwd/native options and assert the real child retains original inputs for both routes and Codex/Claude. Topology tests cover lost replies, symlink changes, removed directories, final pre-start refusal, repair and historical completion without duplicate effects. |
| Other adapters | Hermes's source-bound helper fixture checks the requested root, named-profile scope and child inputs. agy rejects explicit profile selection but its cwd-only public child still works. |

## Previously measured executable matrix

Task9 ran `nice scripts/herdr-release-smoke --case
herdr_release_launch_and_handoff --jobs 3` on tree
`4a0ffd863daf1d8491c270208ab1f147a9a98b10`, scoped run
`fa756dca-e2c5-44f9-91c2-48e3114b33c3`. All listed releases passed:

| Herdr | Result | Seconds |
| --- | --- | --- |
| 0.9.1 | PASS | 80.4 |
| 0.9.2 | PASS | 82.1 |
| 0.9.3 | PASS | 81.3 |
| 0.9.4 | PASS | 85.7 |
| 0.9.5 | PASS | 82.0 |

The retained task9 `target/ht-1urq.9-evidence/green-driver.log`, per-release
logs and `child-records` contain the output and actual child env/cwd observations.
Its public topology suite passed 24 tests, Hermes fixture suite five and agy suite
three; matrix and follow-up scoped process/scratch checks were clean. This sweep
inspected those artifacts rather than repeating the matrix. Earlier RED honestly
recorded the native transport refusal; the native19 repair preceded every GREEN
row. These source-bound historical records remain in the task9 result artifact.

## Guidance and limits

README, handoff help/golden, agent reference and shipped skill consistently describe
new-tab-only absolute existing directories, adapter mapping, frozen retry and
directory repair. The changelog proposes unreleased 2.1.0; no package bump occurs.
The skill uses known names in prose while retaining exact command/receipt IDs;
immediate dependency replies and dispatch use p0, routine updates p1. Receipt
dependence requests an actionable response; missing receipt prompts useful
recovery rather than polling, repeated nudges or restarting peers. ACK still means
receipt only; complete display ACK, invitation acceptance and read-only subagent
rules remain distinct. Runtime wake defaults are unchanged.

Measured UTF-8 skill size: baseline before `0e5fb4895` was 46,380 bytes; final is
47,370, including the launch example. Growth is 990 bytes, within the 1,024-byte
budget. No per-message or per-turn framing was added by this sweep.

These owned credential-free stand-ins prove declared child input consumption;
shell startup may alter those inputs. They do not establish native authenticated
account selection, model activation or live harness behavior. No real profile,
shared-server lifecycle, live quota, release, tag or push was used. Existing trust,
receipt provenance, native guard and possible-start rules were preserved.

## Current-tree checks

Exact commands, source-tree hashes, output excerpts, cleanup and structured
evidence are retained in the task10 implementation report. This documentation-only
sweep introduces no test assertion or behavioral change; its checks exercise
existing coverage against the integrated source.
