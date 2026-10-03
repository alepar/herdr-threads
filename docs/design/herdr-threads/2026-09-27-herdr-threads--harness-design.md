# Native harness hooks and launch

## Goal

Give manual and managed native Codex/Claude occupants the same compact recovery path, while transporting verified per-invocation caller evidence so subagents cannot settle a pane's receipts.

Parent: [root design](2026-09-27-herdr-threads-design.md). Bead: ht-4is.8. Earlier siblings: caller-attribution, store, seat-identity, scheduler, daemon and CLI designs.

## Adopted shared-contract detail

The [adopted shared-contract amendment, revision 4](shared-contract-amendment-adopted.md) is normative for the exact types, schema, algorithms and ownership described below. Its adoption is a design decision, not implemented or native-tested evidence. Existing acceptance remains required. Logical publication is authoritative; bounded physical projection cannot hide committed receipt/warning obligations. F6 is resolved by the [trust policy](../../../TRUST-POLICY.md) (2026-10-01), which is normative for continuity and attribution; preserve the BOTH-harness gate `ht-910`; shared types, fake ports and store tests cannot satisfy that gate. Formal design review counters and original task review histories are unchanged by these edits.

## Decision and feasibility gate

Use supported native startup and tool/turn hooks plus Herdr native idle prompts. No nested PTY, replacement agent runtime or model-visible attestation ritual. The exact metadata correlation and invocation-scoped context transport come from the positive two-harness recipe ht-4is.2.3. Production adapters cannot proceed on a negative/partial recipe. The recipe must prove execution-identity freshness against pinned source/native evidence: a fresh RPC returning cached native metadata alone is insufficient and fails closed. If the installed harness cannot meet the gate, the autonomous blocker workflow investigates a supported alternative rather than shipping inherited-environment authority.

Each adapter parses versioned native hook envelopes, normalizes allowlisted evidence, forwards it to the identity verifier and encodes supported native hook output. Keep Codex and Claude adapters separate; their shared semantics do not make their hook envelopes interchangeable. Unknown identity-critical fields/shapes or unsupported versions return explicit degraded capability without settling receipts. Preserve unknown additive fields when native hook output requires pass-through.

## Hook flow

Startup/resume/new/clear verifies the current execution context against a fresh host observation and requests compact thread topic/stats/pending metadata. Verified startup may allocate a seat only when the observed target has no unresolved continuity claim; a recovery-held target reports explicit operator repair/fresh-role guidance without allocating or registering it. Mere host observation never allocates a seat. Ordinary compaction requests recovery metadata within the current occupant. A native child event can obtain read-only discovery but never register itself as the top-level occupant or advance its delivery checkpoint. A repeated startup event is idempotent for the recipe's execution-instance identity.

At supported tool/turn boundaries, request bounded attention changes. PreToolUse (where verified by the recipe) supplies a short-lived invocation context to the individual tool command, not to a global pane environment inherited by later subagents. All accountable CLI operations consume this context; daemon validation remains authoritative: every accountable request obtains a new target observation, compares proven native execution identity, and consumes a short-lived payload-bound internal permit at the store transaction decision. Cached generation or an unexpired tool context alone is insufficient. The recorded fresh observation point and post-observation native race are defined by the identity contract; do not claim atomic native-current-at-SQLite-commit authority. Context is single-invocation scoped, tied to root/execution/tool event, short-lived and excluded from durable client intent. Missing context gives a compact action/error instead of silently downgrading checks.

Do not parse a shell command to decide that its author is the root. A native hook's evidence authorizes its context regardless of whether the command uses quoting, files, stdin or a retry. Supported command rewriting must preserve original shell semantics and complete native hook output, proven by fixture and live tests. If code-mode nested tools take a different path, support that tested path or report it explicitly unsupported; no cross-tool ambient credential fallback.

Hooks return a deterministic bounded hint/metadata response with exact read-only inbox continuation, never unbounded thread history or another automatic checkpoint mutation. Apply the CLI's complete encoded-output budget after the native envelope's escaping/framing. Every peer-controlled field—including message bodies/previews, topic/goal, workspace/tab/pane labels, display names and quoted diagnostic/provenance text—is marked untrusted data. Use a fixed plugin-authored instruction template and an escaped `untrusted_peer_data` container in a supported data/context location. Never interpolate peer values into privileged instructions, native role/control keys, executable commands or instruction sentences. The fixed template explains that quoted peer data cannot override instructions, permissions or receipt semantics. A fixed wrapper with explicitly marked, escaped quoted JSON data may be a supported representation even when the native envelope is textual; a text field alone does not justify omission. The positive native recipe and fixtures establish that actual representation. If the validated path cannot preserve the marked-data boundary, omit peer fields and return canonical IDs/plugin-owned counts and ordinary detail-read commands, and report required topic recovery degraded/unsupported. Such fallback cannot count as satisfying full topic/stats startup recovery. Escaping is a structural boundary, not a claim of immunity to semantic injection. Successful context output or tool completion records only offered/read observations; explicit accept/ACK commands remain necessary. No automatic ACK hook is installed.

## Failure and recursion

A hook has a 1.5-second end-to-end monotonic budget for ordinary check-in; startup ensure/check-in may spend up to five seconds and report unavailable compactly. Queue time counts; remaining budgets clip identity/host calls, and cancellation propagates to their owned work. Fresh target reads have a 750 ms whole-call cap within that remainder; expiration leaves the operation uncommitted and does not stop native agent/tool execution. On daemon/host failure, let the native agent/tool continue according to its normal approval rules, while accountable commands fail explicitly until evidence/service recovers. Hook absence/incompatibility is visible in doctor and health. Do not claim that successful installation proves hook execution.

Use an internal invocation marker only to prevent hook recursion when the plugin's own helper executes; it confers no identity authority. Check-in is cheap and coalesced by attention generation. Routine no-change output is empty. Startup recovery is emitted once per verified execution/generation; pending ordinary obligations remain discoverable/retriable even if output is lost. A verified seat-wide check-in may stop a coalesced warning wake before every warning is displayed; exact continuation/history keep those warnings discoverable.

After a failed startup check-in and service recovery, a resolved, recognized native idle/done target may receive the standard generic recovery hint even while unregistered: `herdr-threads: attention pending; run herdr-threads inbox`. It contains no peer topic/body or credential. The scheduler owns fresh target checks, blocked/known-human-input deferral, finite attempts and elapsed spacing; the hint does not register an occupant, start receipt timers, advance checkpoints, accept or ACK. A recovered native root tool/turn hook must then retry ordinary check-in with fresh invocation proof; only successful verified registration starts previously unstarted timers. Existing deadlines never reset.

This recovery requires a running daemon and supported native hook execution. An indefinitely stopped unsupervised daemon still requires an ensure-capable invocation; a missing/unsupported hook remains degraded and cannot be reported recovered. The positive BOTH-harness recipe gate must demonstrate the actual fresh root callback following the generic hint. Unknown harness/execution identity, unresolved/held targets, shell-only or blocked/working targets cannot receive this idle prompt. Working agents use supported tool/turn boundaries. Child callbacks may read but cannot register or advance the checkpoint.

## Installation composition

Provide explicit setup/inspect/remove for owned harness hook entries. Prefer project/session-scoped supported configuration for this repository and test sessions; no automatic global configuration writes. Preserve other hook entries and native permission settings byte-for-byte where possible, merge structurally otherwise, and detect concurrent modification. Tag owned entries and keep an exact backup/ownership manifest; remove only entries still matching the installed version. If the harness cannot compose a supported scope safely, setup returns an actionable error and prints manual instructions, without replacing an entire config file.

Resolve executable path as an argument array/quoted native hook command according to the harness schema. Paths with spaces must pass tests. No download/install of a different harness is implicit. Publish compatibility and setup instructions with tested version ranges, exact scope and unsetup behavior. Existing hcom hooks remain intact; native tests use the prior research's supported per-session HCOM_DIR isolation and disabled participation/trust for only the new sessions, not global disabling.

## Managed launch

`launch --pane ADDRESS --kind codex|claude` acts only on an explicit existing empty shell pane. It resolves its seat before native launch, so a caller can invite and send the ordinary handoff first. A recovery-held target requires explicit operator repair or fresh-role choice first; managed launch cannot silently allocate around a continuity claim. It does not itself accept or ACK. Verify pane availability immediately before Herdr agent start; never send shell commands into another occupant. Use direct native launch and the owned supported hook configuration. Codex includes --no-daemon in this environment; both agents verify inherited Herdr identity from a non-login tool shell. Preserve user native arguments and permission policy; do not add blanket auto-approve flags.

Managed launch and a manually started agent use the same hooks/check-in. A launch error leaves durable invitations/messages intact. A lost initial prompt cannot erase the thread handoff; verified startup/check-in plus later inbox reads recover it. Launch success means host-observed startup, not handoff receipt. A missing configured hook must be reported in health/doctor and cannot be disguised by auto-ACK.

### Typed managed-launch seam

> **Superseded (2026-10, ht-p03.2 / B4):** the adversarial verification layer described here was removed; see "Cooperative reality (2026-10)".

Use `HostPort::launch_native(NativeLaunchRequest, HostCallContext)` with explicit seat/target/harness/argv, configured-hook reference and expected terminal/generation. Setup owns a narrow configured-hook inspection function returning the supported scope/path/fingerprint descriptor, not authority or an implicit global configuration write. Structural `EmptyShell|Occupied|Unknown` observation must prove emptiness; missing native occupant metadata is not proof. Resolve the seat first, then take the fresh ordered empty-target recheck immediately before supported direct argument-array native start. If an atomic host expected-empty guard is absent, report the optimistic recheck/start race rather than invent it.
<!-- end superseded -->

Preserve caller native argument bytes/order and permissions; add only supported owned-hook configuration and Codex --no-daemon exactly once. An explicit conflicting daemon mode rejects instead of silently rewriting caller policy. Outcome distinguishes bounded host-observed startup from OutcomeUnknown after possible start; do not blindly duplicate a launch. The call uses absolute budgets/cancellation/late-result fencing and never registers, creates availability anchors, advances warning offers, accepts or ACKs. Both managed and manual startup still require the original verified hook path and ht-910 gate.

Check-in metadata includes logical warnings before attribution/physical projection and only advances the current verified binding-generation/execution frontier after producing a reachable bounded offer. Replacement resets that frontier while retaining obligations and spacing; hook output loss is not receipt or proof the model read it.


## Decomposition and validation

Independent Codex and Claude adapters consume the proven recipe and compiled hook/check-in contracts. A shared setup module owns structural config composition/ownership and uses adapter-provided hook declarations. Managed launch owns native-start argument construction and preflight; the root application composition owns connecting all adapters to the identity/CLI/service interfaces.

Fixture tests cover root/child/concurrent children, resume/new/clear/compaction, missing/stale metadata, native command rewriting with Unicode/quoting/stdin, configuration preservation/rollback, paths with spaces, no-change empty output and budget expiration. Native live tests later prove each supported configuration; fixture passing alone is not evidence of model receipt or supported code-mode attribution. Both adapters include hostile topic/label/preview fixtures containing instruction text, fake role/control keys, delimiter closers, quotes/newlines and terminal escapes. Assert fixed instructions/control structure, marked escaped data or omission fallback, encoded byte bounds, exact continuation and zero mutation from output. Test failed startup through its full budget followed by an idle generic hint and fresh root callback; prior to successful registration timers/checkpoints remain unchanged, and child callback cannot perform that registration. Native validation repeats this with no initial prompt and actual model accept/ACK evidence. Same-user explicit administrative child actions are allowed under the separate operator policy, but never become native receipt/checkpoint authority.

## Setup acceptance boundary

Setup inspection tests distinguish installed configuration from observed capability using injected observations. The later application composition task verifies that the real doctor command exposes that distinction. Setup completion does not require its downstream executable to exist.

## Shared-contract adoption record

2026-09-27: Reconciled this canonical spec with the adopted revision-4 shared contract. Detailed normative algorithms/types and preserved revision responses are in the [adopted shared-contract amendment, revision 4](shared-contract-amendment-adopted.md). This is specification work before the next formal design review; no source implementation or native-support completion is asserted.

## Cooperative reality (2026-10)

- **Hooks transport a claim.** The Claude and Codex hooks carry the seat's private context and the lifecycle or tool-boundary CheckIn; the service records the top-level claim as `cooperative_top_level`. Subagents are told (not forced) to read and summarize only.
- **Managed launch has one capability.** `NativeLaunchCapability` is `HostGuardedStart | Unsupported`; the proven-empty-shell capability is removed, so launch relies on the host's guarded start and the optimistic fresh target recheck, never on an empty-shell proof. Launch still creates no registration, anchor, offer, accept or ACK.
- **Ready-command rank.** The hook's plugin-authored command block (`harness::next_actions`) is ordered so a budget keeps a prefix: (1) each thread with a pending require-ACK receipt (its accept, read, then ACK lines; `pinned` ends at the first ACK line); (2) other threads with a required invitation (accept-required, read); (3) one reply form for the first receipt thread; (4) at most `MAX_OPTIONAL_ACCEPTS` (2) plain invitations, labelled "optional, only if you intend to join". The header names the caller's seat and says `thread participants` and `thread show` mark it as self.
- **No lifecycle subscription.** The host lifecycle subscription and its hint types were removed; reconciliation rests on periodic complete snapshots.

Decision record: [root spec §B4](remaining-findings/2026-10-01-remaining-herdr-threads-findings-design.md#b4-remove-the-pre-cooperative-verification-layer) (ht-p03.2 deleted the code, ht-p03.5 rewrote these docs). Normative trust rules: `TRUST-POLICY.md` (branch `trust-model-invariants`), which supersedes any text above that requires adversarial proof of who is calling. Text above that describes the removed layer stays for history and is marked superseded where it names a removed symbol.

## Optimistic admission and the versions document (2026-10)

The compiled recipe tables (`claude::RECIPES`, `codex::RECIPES`) stay the authority for which harness versions are understood. `harness::admission::classify` maps any observed version to exactly one row of a ladder, evaluated top to bottom, first match wins, identically for both harnesses:

| Row | Observed version | Admission |
|---|---|---|
| 1 | unparsable | refused |
| 2 | inside a recipe's `known_broken` range (checked before every admitting row) | refused, naming the range and the newest working version |
| 3 | listed by a recipe | listed |
| 4 | Codex only: unlisted, not older than every recipe minimum, hook-schema fingerprint matches a recipe | schema-matched, live-unverified |
| 5 | older than every recipe minimum | refused |
| 6a | unlisted, newer than every recipe maximum | optimistic |
| 6b | unlisted, inside the supported span (a gap between recipes or inside a recipe's version set) | optimistic |

An optimistic version assumes a recipe (6a: the recipe with the greatest maximum; 6b: the recipe whose span contains it, else the nearest recipe below). It maps to the cooperative, live-unverified harness state and is never `Supported`. Health shows it as an informational note, not as degraded, and `doctor` as a warning: `optimistic — newer than verified X` or `optimistic — unlisted within the supported span`, then `assumed compatible with recipe ID`, with `; major version change` when the major differs from the assumed recipe's maximum and, in `doctor`, the issues URL. Hook payloads the assumed recipe cannot parse are counted and logged (rate limited) and Health reports `N hook payloads not understood (HARNESS)` instead of silence; the CLI reports a parse failure to the daemon only when it advertises `hook.parse_failure_report`.

**Source of truth.** `docs/compatibility/harness-versions.json` is generated from the recipe tables (one row per listed version with its recipe, evidence level `live`, `no_model` or `none`, and `known_broken` ranges) and a guard test fails when the committed file and the tables differ (`HT_BLESS=1` regenerates it). Health and `doctor` wording, the setup sandbox constants and the documented version lists derive from the same tables.

**Re-observation and the canary.** An admission-observer lane re-runs admission on a Pacer tick (60 s) and replaces the cached observations when the resolved binary's path, inode, size or mtime changes, logging the change; `doctor` checks the `claude` on `PATH` as it does Codex. `scripts/harness-canary.sh` is the single canary implementation (isolated installs, a no-model tier 0, a model tier 1 when keys exist, bisect to the first failing version) and the scheduled workflow only calls it; its output suggests a new recipe or a `known_broken` range, never an automatic code change. See [harnesses.md](../../compatibility/harnesses.md) for the registry and canary liveness rule.

Decision records: [root spec §B6](remaining-findings/2026-10-01-remaining-herdr-threads-findings-design.md#b6-optimistic-harness-admission-and-the-version-canary) and the nested [harness canary spec](remaining-findings/2026-10-01-remaining-herdr-threads-findings--harness-canary-design.md).

## Post-Implementation Notes

*As this design is implemented and iterated on — bug fixes, adjustments, anything that diverged from the assumptions above — append a dated note here, whether or not a formal debugging skill was used.*

### Guarded native start

[Guarded native start disposition](guarded-native-start-disposition.md) extends availability preflight to a supported host-side shell/prompt check-and-start operation. It preserves Unknown observations and all identity/recovery fences. A finite launch-specific 30-second ceiling is clipped to caller budget; possible-start failures remain OutcomeUnknown. Native qualification is still required.
