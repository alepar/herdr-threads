# F1 native notice attempts

Production build anchor: `a19fd881e`; candidate SHA256
`a8aef2df56e6954fe3c12b7d02841f521712ceb403d13f9b93fabdabe2224289`.
All attempts below used this unchanged binary and the retained official runtime
entry `e79626f2dd6f94aa45d30f3fa63cd84319a6eefcd150b353cfaf274366926774`.
The three-case and isolated-wake attempts used scripted provider fixture
`1bbb2f67a76a015beebeb055fb915389fb6aa50178c46d1e7c17e95d7d2380fa`.
No live request or credential read occurred. Shared format, clippy, default,
neutral and wire guards passed before the single 43.21-second warm build.

| Driver source | Attempt | Result |
| --- | --- | --- |
| `a19fd881e` | Native3, 9.25s | `notice_reload` setup failed: unsupported launch profile flag. No notice cause; other cases unexecuted. |
| `9b784fb72` | Native3, 9.01s | Sink launch refused executable/PATH mismatch. No notice cause; other cases unexecuted. |
| `a424ed204` | Native3, 69.62s | Reload PASS17.312s; mixed page PASS14.337s; wake FAIL after a host marker submitted internal draft text. |
| `c045bdd74` | Passive wake-only diagnostic, 26.08s | Case reported PASS17.306s, but internal draft/empty-terminal divergence invalidated visible-draft refusal credit. |

Each attempt preserved its own results, commands, captures and native traces.
Each reported helper reaped, owned root removed, process/temp scan exit0.
The external read-only hash observer missed the first fast failure and was
stopped/reaped (143); it was replaced by immediate read-only setup-file hashes.
All failed attempts remain failures; later corrections do not rewrite them.

The two behavioral passes used a genuine second Pi agent with an owned unread
draft and a real future-deadline Pending receipt. Reload retained the actual
tagged Notice marker until exact native branch/context evidence and canonical
frontier advancement. The mixed positive put the real Notice and agent-created
Invitation in one complete native proposal; presentation did not join the
thread, and a separate public accept did. Hidden controls and native mixed-page
negative coverage are not inferred. The sink's ordinary ACK was distinct from
Notice settlement. These observations remain tied to their exact driver source;
the corrected physical staging requires new three-case evidence.

The failed wake trace contains the exact prompt
`OWNED NOTICE QUALIFICATION DRAFTherdr-threads: wake up` at1794741160863.
Pre-command styled capture showed a literal draft, and post-command API state
reported it, but the immediate daemon screen-read response was not retained.
The planned controller clear had not occurred. No quiet assertion was waived.

The passive diagnostic then directly observed API draft state at1794741845795
versus an empty visible/detection/styled composer at1794741845887, including an
inverse cursor at column0. The divergence persisted in the pre-clear snapshot
at1794741855908. Planned Ctrl+C1794741855914 was followed by the exact host wake
at1794741856724. This supports the fixture-rendering diagnosis without proving
the earlier near-submit screen. Runtime `setEditorText` changed editor memory
without requesting repaint. Root recorded the separately accepted unrendered
third-party-editor limit in `f19c7fca4` and follow-up `ht-1z1n`; the production
extension never sets editor text. User typing retains the visible-draft contract.

The reviewed correction reconstructs only two owned literals through real
terminal input: Ctrl+A, type the same literal, Ctrl+K. H→H→HH→H never exposes an
empty buffer. Every step preserves the original run baseline and, for the cause,
the finite future deadline. Exact current composer row, cursor/style and fresh
API state are required; transcript copies, unknown bindings/layouts and unexpected
runs refuse qualification. The same predicate covers sink invitation recovery,
initial holds and completed reload rendering. No private repaint or core gate
change is used.

Its causal test was RED because the old controller accepted API-H/rendered-empty
without any terminal reconstruction. Final selftests passed29/29 in2.455s,
including failure at every input on an extra run or expired deadline, transcript
decoys, unknown layout/bindings and separate sink scope; same-run leaks0.
Reviewed patch SHA256:
`5b893790e9270066f9e97df5c72d083fff1c8332d2b338fa814abc37ee389145`.
Corrected driver SHA256:
`c5a81559c2ffaa828ca20b1f3abc3a083d1223b34f0b4f1920084180066e75e7`.

Installed-file digests are captured separately for both owned profiles. They
agree within each run and differ across roots because public setup embeds private
state/host routing in LAUNCH. Observed digests: second setup attempt `a4eef77c…`,
behavioral attempt `9776dbfb…`, diagnostic `fc41d313…`; full hashes are retained in
their manifests. This is installation identity, not proof of model uptake.

Scoped source/diagnostic reviews are archived under [reviews](reviews/README.md).
Final corrected native3, compatibility delta, R2, unfiltered attempt2, scoped live
and the full-suite budget disposition remain separate gates at this checkpoint.


## Rendered-guard qualification and retained-page diagnosis

The earlier checkpoint above is historical. Exact run IDs, source identities,
results and cleanup summaries are preserved in [f1-native-evidence.json](f1-native-evidence.json).
Production binary and provider fixture remained unchanged.

| Driver source | Attempt | Result and evidence boundary |
| --- | --- | --- |
| `49524a38f` | Rendered-guard native3, 139.06s | Reload and mixed-page PASS with exact physical/API guards; wake timed out waiting for a captured new Notice. Failed canonical samples were not retained, so this run alone did not explain the timeout. |
| `ea6729312` | Passive pending-state diagnostic native3, 79.38s | Reload PASS17.291s and mixed page PASS16.234s; wake timeout reproduced with bounded pre-cleanup canonical evidence. |
| `ea6729312` | Fresh isolated `notice_wake`, 25.90s | PASS17.192s: physical draft guard, single public clear, actual content-free Native wake, exact branch/context and frontier advance. |

The pending-state diagnostic adds one latest snapshot, at most eight stage
transitions and bounded read-only failure rows/status before cleanup. It changes
no delivery predicate, 35-second wait, guard, admission, pull or settlement.
Thirty driver tests passed, including preservation of the original timeout if a
diagnostic fails; scoped cleanup passed. Its driver hash is
`b5e33bdc366747679a7d588860e556ab8701ef43a0872dd7c3487f47d8a320e5`.

The diagnostic established a genuine new warning `e2I9M0wpynHE` at ordinal9
above frontier5. The current consumer held only the previous condition's clear
**Notice** `e2I9Ap976amQ`, tagged `dn1:9:39:7`, with retirement/confirmation masks0
and more1. The binding/consumer matched, and its manual watch child was alive.
The previous clear body was absent from the last cleanup context/branch/drafts;
no later qualified context followed. This is a body-bearing Notice, not an
`AttentionCleared` control: local application cannot confirm it. Canonical data
was retained; the driver incorrectly assumed one cleanup turn drained later
notices. The actual presentation lease was not observable and is not inferred
from the token or execution ID. These read-only statements were sequential,
not an atomic database snapshot.

This demonstrates the accepted retained-page backpressure, **not yet automatic
progress after release**. Main explicitly required that further liveness proof:
a clear Notice alone intentionally does not wake and may wait until another
turn; a newer actionable warning must still wake and deliver old then new without
additional human typing. Canonical/scheduler tests are recorded in
[notice-liveness.md](notice-liveness.md); the native progression case remains
pending at this checkpoint. No core gate was relaxed to make a test pass.

The selected fresh wake proof used a real rendered draft at1794745397892,
Ctrl+C at1794745407319, exact `herdr-threads: wake up` start at1794745407973,
observed context at1794745407979, and frontier1. Its sink's distinct receipt
remained Pending until deliberate post-assertion cleanup. Both installed
extensions hashed `795e277e5b8d0fc5e00b575da01527c571f19b83ec21c0ba28f610b7c50a8ab8`.
Every run in this table cleaned its owned helper/root and passed its leak scan.

Thus reload/page and isolated wake have explicit passing evidence on the same
production binary, **not a contiguous three-case pass**. Final compatibility,
whole-branch review, corrected unfiltered attempt2 and the authorized live test
remain pending; no credential read or paid request is claimed here.

## Native protected-page successor progression

The later standalone HOL case adds fixture `08174d10…`; production candidate
and official runtime remain unchanged. Three setup/observer failures are
preserved: `658634ec4` (58.79s) and diagnostic `bc1aabfda` (59.88s) incorrectly
waited for informational clear-only watch allocation; `cc15b5fa2` (25.08s)
observed the protected clear but used an invalid prepared-receipt query for a
noACK lazy audience. None reached the newer cause or critical draft release.

The corrected `0042fcf38` run passed `notice_hol` in59.536s (68.12s total).
The actual old clear page remained protected while the newer warning was
pending separately. After the single Ctrl+C release, two automatic host wakes
delivered old then new, with canonical frontier advancement and no further
controller input before completion. All four HOL attempts cleaned their owned
resources and passed scoped leak scans. Exact evidence and the observed later
automatic-wake path are in [f1-native-hol.md](f1-native-hol.md) and
[f1-native-hol-evidence.json](f1-native-hol-evidence.json). This supersedes only
the earlier native-progression-pending status. The subsequent
[final compatibility selection](f1-final-compatibility.md) passed twelve fresh
cases on the same binary; safety/threshold remain explicitly historical.
R2, unfiltered attempt2, live and budget disposition remain separate gates.
