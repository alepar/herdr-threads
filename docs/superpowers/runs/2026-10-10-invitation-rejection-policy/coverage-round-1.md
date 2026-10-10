# Root coverage round 1 — ht-xms

Three fresh isolated, input-bounded reviewers received the entire root spec prose, every bulk-tree node's actual id/title/description/blocking dependencies (including the root epic), the same canonical requirements, the demotion flag/rationale, and empty parent/previous-round/ledger inputs. Reviewers were forbidden tools. The requested opus model was unavailable; all used the inherited model. All three returned valid reviews. No repository/code exploration informed verification. No task-tree mutations were made by this pass.

requirements: 7 · mapped: 7 · unmapped: 0 (none)

| Requirement | Reviewer 1 | Reviewer 2 | Reviewer 3 |
| --- | --- | --- | --- |
| R1 mandatory decided bad-fit rejection, exact ID, meaningful reason | ht-xms.1 | ht-xms.1 | ht-xms.1 |
| R2 preserved inspect/unresolved/preapproved/required/subagent exceptions | ht-xms.1 | ht-xms.1 | ht-xms.1 |
| R3 attributed native warning to frozen recipients | ht-xms.2 | ht-xms.2 | ht-xms.2, ht-xms.3 |
| R4 pre/post projection visibility, current-binding settlement, no wake | ht-xms.2, ht-xms.3 | ht-xms.2, ht-xms.3 | ht-xms.2, ht-xms.3 |
| R5 exact/new-key replay, historical info unchanged, no duplicate/redelivery | ht-xms.2 | ht-xms.2 | ht-xms.2 |
| R6 preserved stale/foreign/required/receipt guards | ht-xms.2, ht-xms.3 | ht-xms.2, ht-xms.3 | ht-xms.2, ht-xms.3 |
| R7 focused tests/fmt/clippy/default-feature checks and handoff | ht-xms.3 | ht-xms.3 | ht-xms.3 |

Mapping is broad ownership, not a claim that every acceptance detail is explicit. Reviewers 1 and 3 independently identified the two narrower acceptance omissions below. Reviewer 2 considered the root spec and broad owned flow sufficient, and reported no findings. Union verification retains the independently evidenced omissions for root disposition rather than voting them away.

## Verified union for root disposition

### COV-1 — GAP: explicitly assign current-binding carried-prefix settlement regression

Reported by reviewers 1 and 3; verified against supplied inputs. Spec: “Check-in settles only its carried bounded prefix for the current binding,” and verification names “check-in offer settlement.” ht-xms.2's enumerated tests omit check-in settlement entirely. ht-xms.3 names existing families and a general combined sweep, not this new regression. This is a specific test/acceptance omission within an otherwise mapped R4, not a new independent deliverable or unspecified design decision.

Proposed fix: amend ht-xms.2 acceptance to exercise a rejection-notice check-in offer, prove only its carried bounded prefix settles for the current binding, leave uncarried notices pending, and preserve receipt state. Root owns final disposition and tracker edit.

### COV-2 — GAP: explicitly assign historical info-event compatibility regression

Reported by reviewers 1 and 3; verified against supplied inputs. Spec: “an older info event is not rewritten or redelivered.” ht-xms.2 names “exact/new-key replay” and “prevent wakes and replay redelivery” without specifying an existing info event; these can all be satisfied by testing freshly created warn events. Neither leaf names historical-fixture coverage. This is a specific test/acceptance omission within mapped R5.

Proposed fix: amend ht-xms.2 acceptance with a retained historical info-event fixture and exact/new-key replay assertions that its event/payload/severity remain unchanged and no warning-job, attribution, or delivery rows are created. Root owns final disposition and tracker edit.

## Other reviewed findings and verification

| Stable ID | Reviewer claim | Verification / recommended disposition |
| --- | --- | --- |
| COV-3 | Reviewer 1: GAP for native author/source assertions and independent wrong-thread/invitation controls | Not retained as a separate verified gap: ht-xms.2 owns canonical warning classification, exact guards, and existing unsafe/required controls; the supplied spec defines the canonical rejecting-seat/source checks and independent controls. Root may make these test details explicit while amending the same leaf, but broad ownership is present and no separate missing behavior is evidenced. |
| COV-4 | Reviewer 1: UNEXERCISED-CONFIGURATION because no-wake assertions absent from enumerated tests | Rejected: no-wake is a required property, not an enumerated runtime configuration; ht-xms.2 explicitly requires “prevent wakes.” The supplied spec explicitly requires its no-wake exercise. No configuration-smoke task justified. |
| COV-5 | Reviewer 1: NARRATIVE-EDGE for both sweep dependencies | Rejected: both blocked-by lines use the expressly valid fixed artifact token “all leaves (integration sweep).” Both actual leaf producers exist. The prompt explicitly allows this token. |
| COV-F1 | Flag sweep ht-xms.2 sp:demoted-by-session | Disposed: retain atomic leaf; substantive rationale recorded in ledger. All reviewers accepted the rationale. |

Every node serves the goal. The warning flow and combined sweep form the walking skeleton. No concrete unowned exchange was evidenced. Only blocking edges are ht-xms.3 ← ht-xms.1 and ht-xms.3 ← ht-xms.2; both have valid reason lines. Root epic has no blocking deps. Parent-child containment was excluded from the blocker graph. No `(needs: ...)` citations exist, so mechanical acceptance graph checks have zero citations to evaluate. No uncited acceptance references to a dependent producer or cycles were found. No missing spec input or degraded review qualifier remains.

Root applied COV-1/COV-2 by strengthening ht-xms.2 acceptance and additionally made native author/source, independent wrong-thread/invitation, no-wake before/after attribution, delayed attribution, and atomic projection rollback/retry assertions explicit. Graph unchanged. Round 2 is required and reviews that amended leaf. This agent neither edited tracker/code nor sent Herdr messages.
