# Outside-adapter report (R13)

`scripts/outside-adapter-report` measures how much of a change lands outside the agy adapter,
so the cost of the harness-adapter work is visible in the run report. The orchestrator runs it
at the final SHA (phase 6).

```
scripts/outside-adapter-report --base SHA --head SHA [--repo DIR] [--json]
```

Exit 0 on success, 2 on bad arguments or unknown SHAs. Python 3 standard library only
(3.9-compatible). It needs `git`; for the neutral-check counts it adds a detached worktree per
SHA and always removes it.

## Partition rule

The diff is authoritative; trailers only annotate, so a missing trailer cannot hide cost.

`git diff -M base head` is split per path. A path is **adapter** when it is under
`src/harness/agy/`, under `tests/fixtures/agy/`, or matches `tests/harness/agy_*.rs`. The
`builtins()` row counts as adapter too: added or removed lines of `src/harness/registry.rs` that
contain `Registration::new(&crate::harness::agy::` move to the adapter bucket (listed as
`src/harness/registry.rs (builtins row)`); the rest of that file stays outside. Everything else
is **outside**.

## Categories (outside paths only)

| Category | Paths |
|---|---|
| `trait-extension` | `src/harness/adapter.rs`, `src/harness/registry.rs` |
| `test-infra` | `scripts/agy-*`, `docs/validation/agy-native.md`, `docs/evidence/agy-*/**` |
| `docs` | `README.md`, `docs/**`, `TRUST-POLICY.md`, `AGENTS.md`, `integrations/*/README.md` |
| `tests` | `tests/**` |
| `reworked` | other `src/**` paths whose diff removes lines, or that a commit naming a core-sweep or delivery-policy bead (`ht-poh.2.*`, `ht-poh.3.*`, `ht-poh.8`) in its subject or trailer touched |
| `callers` | remaining `src/**` paths |
| `other` | anything else (manifests, CI, changelog) |

## Sections

- `## Summary`: adapter versus outside files and lines.
- `## By category`: outside totals per category and each outside file.
- `## By subepic`: per commit group (first `ht-poh(.N)*` in the subject mapped to `ht-poh.N`, else
  `unattributed`): commits, outside paths and lines from the commits' own diffs, and the
  `Outside-adapter:` trailer text.
- `## Neutral check counts`: `scripts/check-harness-neutral --json` run at both SHAs (or
  `not present at <sha>`), and the per-category delta.
- `## Allowlist delta`: entry lines added and removed in `scripts/harness-neutral-allowlist.txt`, by category.
- `## Missing trailers`: non-merge commits without an `Outside-adapter:` trailer (still counted above).

`--json` emits the same data as one object.

## Sample (abbreviated)

```
## Summary
| Bucket | Files | Added | Removed |
| adapter | 2 | 120 | 0 |
| outside | 6 | 7 | 4 |

## By category
| trait-extension | 2 | 2 | 0 |
| reworked | 1 | 2 | 1 |

## By subepic
### ht-poh.2
- commits: 2; outside lines +4/-1; outside paths: `src/harness/adapter.rs`, `src/cli/setup.rs`
- trailer: src/harness/adapter.rs, src/cli/setup.rs

## Allowlist delta
- -2 to-migrate

## Missing trailers
- `1a2b3c4d` tweak app without a trailer
```
