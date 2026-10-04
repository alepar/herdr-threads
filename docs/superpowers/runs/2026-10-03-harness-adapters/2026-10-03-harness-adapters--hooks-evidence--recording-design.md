# Domain-scoped evidence recording

Bead: `ht-3bi.2.4`. Parent: [Generic hooks and evidence](2026-10-03-harness-adapters--hooks-evidence-design.md). Ancestor: [Harness adapters](2026-10-03-harness-adapters-design.md).

## Goal

Record bounded runtime and contract-domain observations transactionally, with adapter-declared verification milestones and honest origin, while preserving shipped Claude/Codex evidence commands, rows and precedence. A new adapter uses the generic v2 path without adding a brand-specific recorder or gate.

## Decisions and boundaries

Use the runtime/domain model from `ht-3bi.2.3` and SQL tables from `ht-3bi.4`. The parent design fixes key grammar, origin enum, field limits, three-table DDL, retention and milestone policy. This spec assigns the recording seams to three reviewable units. Migration numbering is exclusively `.4`: CLI owns v19 and its following v0.2.2 migrations; prototype SQL is provisional, main is absorbed and the adapter migration follows CLI's final schema before freeze.

The store leaf owns transactional rows and bounded reads. The daemon/protocol leaf owns capability/command/result, descriptor resolution and advisory recording policy. The client leaf owns classification/attribution submission and durable send gates. Store code does not resolve native role or caller authority. Daemon recording has no seat/receipt mutation. Client classification cannot manufacture server verification flags.

## Store API and transaction

Add explicit v2 record/read operations alongside existing `EvidenceRecord` APIs. Runtime descriptor validation/canonicalization uses `.2.3`; persistence rejects a key/descriptor mismatch. A record names harness, identity or unavailable reason, domain, origin, contract ID, native event, outcome and resolved milestone. Required milestones are supplied by the server's registered descriptor, never by client JSON. Qualification is resolved before calling the store. Unknown identities/outcomes remain bounded errors or unattributed notes rather than successful milestones.

The transaction inserts/updates the exact runtime descriptor and evidence row, updates first/last timestamps and adds the first success timestamp for an eligible declared milestone. A successful observation never clears a sticky first violation. Verification requires all milestones of that exact domain/contract; another origin, runtime or contract cannot contribute. Empty required sets cannot produce verified-by-use success. Preserve malformed-versus-violation semantics and existing local-broken precedence.

Build keys hash every exact descriptor field according to the parent canonical format. Stable release keys may use an adapter-canonical release descriptor; conflicting descriptors for one key are refused, never silently replaced. Hermes exact source descriptors use build identity unless its adapter separately proves a canonical release representation. Runtime attribution provenance that is not part of runtime identity must remain a separate qualification/source observation, never alter a release identity's canonical representation opportunistically.

Retain 30-day pruning, protected newest 64 evidence rows per harness, and 256-row reads, with deterministic secondary sort keys. Pruning unused runtime descriptors runs in the same transaction and cannot leave referenced descriptors missing. Unattributed reasons retain one bounded latest row per harness/domain/origin. All dynamic SQL selections use declared fixed column/table names and bound values. Legacy rows are untouched and historical evidence remains readable.

## Negotiated wire and daemon

Advertise `hook.harness_evidence_v2` only when the handler exists. Add a separate strict command and recorded result; retain every existing `HarnessEvidence` field, validation and serialized byte shape. The v2 payload carries a registered harness, domain/origin/contract, runtime descriptor or unavailable reason, native event, outcome, optional session, and bounded declared qualification facts. Shared model limits apply before recording. Snake-case events are allowed only by v2's bounded identifier grammar; legacy validation stays frozen.

Resolve the event/domain descriptor through the adapter registry. Class, milestone, required set, role/session qualification and holding policy come from that descriptor. Clients cannot set `verified`, invent a required set or credit synthesized Startup as native lifecycle. Unknown/stale descriptors receive an explicit unsupported/refused advisory result; no fallback credits an unrelated current domain. Contract violation payloads still record the declared failing field when classification supports it; parser value failure is distinct from contract violation.

Preserve bounded pending holding and TTL, but key holds by harness/session/domain/origin/contract. Only an adapter-declared holdable event can enter this queue. Restored holds never move across a domain/contract. Codex resumed creating-version suppression remains sticky and its lifecycle is not held/credited. Hermes direct runtime identity normally requires no holding. Native-shape observations are cooperative projections, not native payload attestation. Manifest fetch remains asynchronous and bounded, with source-aware identity handling; build observations must not request or receive a semver release PASS through suffix removal.

## Client evidence and gates

The hook client obtains contract classification and runtime attribution from its resolved adapter. It selects v2 only after capability discovery. Missing v2 capability is explicitly unsupported for rich evidence and never downgrades Hermes into legacy semver evidence. Claude/Codex retain the existing legacy projection and existing send/hold behavior. No additional startup network work may extend current hook watchdog/global budgets.

Separate v2 gate keys include harness, exact identity, domain, origin, contract and relevant native session. Adapter metadata controls lifecycle always-send, unverified milestone sends and heartbeat; preserve hourly heartbeat and violation/malformed deduplication bounds. Gate persistence uses a new explicit version/file rather than interpreting old resumed suppression files. Successful evidence send records only advisory observation; failed/refused/expired transport never marks a milestone locally verified or suppresses its required retry. Cached gate hints are not canonical authority.

Observers skip check-in, attention fetch, offer allocation/consumption and receipts. They may classify/attribute and submit evidence within their own bounded budget. Tool arguments, results, conversation and user content never enter v2 payloads or evidence logs.

## Leaves and acceptance

1. **Transactional v2 evidence store** owns the v2 store record/read/retention APIs and port plumbing; consumes `.2.3` model and `.4` SQL tables. Focused store tests cover exact identity/domain isolation, milestone completion, sticky violations, descriptor conflicts, bounded retention/read order and unchanged legacy rows.
2. **Negotiated v2 daemon evidence handler** owns capability, strict command/result and descriptor-driven recorder/holding behavior; consumes leaf 1 and `.2.3` types. Focused protocol/daemon tests cover handler discovery, old strict wire fixtures, invalid/unsupported descriptor outcomes, role eligibility, holding isolation/TTL/retry, Codex resumed suppression and asynchronous advisory fetch behavior.
3. **Adapter evidence submission and durable gates** owns `hook_evidence` classification/attribution dispatch, v2 capability fallback and bounded persisted gates; consumes leaf 2's wire/capability and `.2.3` model. Focused client/hook tests cover old daemon fallback, no Hermes downgrade, domain-separated gates, retries/timeouts/heartbeats, legacy Codex resume file behavior, and observer zero-consumption/authority paths.

No leaf owns migration numbering/SQL, Python callbacks, Health rendering, release publication or native model acceptance. Relevant tests only; required fmt/clippy and feature checks, isolated configuration and owned-process hygiene apply. Coordinator owns the final full suite and cleanup. Root owns independent design/code reviews and native final-SHA measurements.

## Post-Implementation Notes

*As this design is implemented and iterated on — bug fixes, adjustments, anything that diverged from the assumptions above — append a dated note here, whether or not a formal debugging skill was used.*

Schema coordination update (2026-10-03): CLI thread names/activity retain v19/v20; immutable additive invitation-rejection overlay is v21 for v0.2.2. Adapter/runtime-evidence migration is v22 after absorbing those merged changes. Historical migrations remain immutable. Preserve invitation effective-state helpers in storage/query/inbox integration.
