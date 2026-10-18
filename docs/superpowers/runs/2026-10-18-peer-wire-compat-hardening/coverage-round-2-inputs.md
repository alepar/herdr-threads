## Goals

### ht-j87z (root)
A herdr-threads node of this release relays, byte-exactly and verifiably, a journal entry whose signed layout it does not understand; every shipped wire shape stays decodable by CI-frozen fixtures; and a repo check fails any change that would make the peer wire less version-tolerant.

## Task tree

- ht-j87z · Peer wire compatibility hardening (epic) · Goal: a node of this release relays byte-exactly and verifiably a journal entry whose signed layout it does not understand; every shipped wire shape stays decod · deps: none
  - ht-j87z.1 · Spike: enumerate journal row readers and opaque-entry seam · Read-only investigation, output is a note committed at docs/superpowers/runs/2026-10-18-peer-wire-compat-hardening/spike-journal-readers.md. · deps: none
      owns: none
      consumes: none.
      owns: the spike note contract = exhaustive journal-row reader list with file:line plus an explicit go/no-go line 'refactor needed: yes/no' that decides ht-j87z.11 (no => .11 closed as no-op and nothing else is blocked on it).
  - ht-j87z.2 · Envelope prefix invariant and opaque verification · Design §1-2. · deps: none
      owns: EntryPrefix (PREFIX_LEN=68, parse/write), OpaqueEntry (verify_opaque), StoredEntry enum, and the shared synthetic-HTJRNL99 test helper (test_support) that signs an unknown-layout entry honouring the prefix invariant
      consumes: Envelope::canonical_bytes, MAGIC.
  - ht-j87z.3 · journal_opaque storage and StoredEntry reader seam · Design §4. · deps: ht-j87z.1, ht-j87z.11, ht-j87z.2
      owns: journal_opaque table (schema 4, additive) and the Chain-row reader Journal::entries_from
      consumes: OpaqueEntry, EntryPrefix, StoredEntry (owned by ht-j87z.2), spike note.
      consumes: StoredEntry, OpaqueEntry. Note: StoredEntry type itself is owned by ht-j87z.2 (consumes it).
  - ht-j87z.4 · Wire form, capability gating, ingest/serve of opaque entries · Design §3, §5. · deps: ht-j87z.2, ht-j87z.3, ht-j87z.8
      owns: WireItem::{Known,Opaque}, cap const OPAQUE_CAP="envelope.opaque.v1", withheld-entry diagnostic counter
      consumes: OpaqueEntry, StoredEntry, FORK_CAP pattern.
  - ht-j87z.5 · Two-node opaque relay integration tests · Design §10. · deps: ht-j87z.10, ht-j87z.4
      consumes: WireItem, OPAQUE_CAP.
  - ht-j87z.6 · Freeze script, 2.1.1 golden session fixtures and decode test · Design §6. · deps: none
      owns: golden fixture format (session.jsonl, envelopes.jsonl, MANIFEST.md with file hashes)
      consumes: PeerFrame::decode, Envelope::try_from+verify, FactRegistry::builtins.
  - ht-j87z.7 · 2.1.1 shadow decoder and forward-compat tests · Design §7. · deps: ht-j87z.4, ht-j87z.6, ht-j87z.8
      owns: frozen 2.1.1 wire structs (PeerFrame w/ serde(other) Unknown, PeerHello, HaveLog, WireEnvelope 13 fields)
      consumes: current encoders.
  - ht-j87z.8 · scripts/check-wire-compat lint, baseline and wiring · Design §8. · deps: none
      owns: rules R1-R4
      consumes: wire struct files list.
  - ht-j87z.9 · Docs: operations limits, trust note, release checklist, changelog · Files: docs/operations.md, TRUST-POLICY.md (only if relay text needs it), docs/release.md, CHANGELOG.md. · deps: ht-j87z.10, ht-j87z.4, ht-j87z.6, ht-j87z.8
  - ht-j87z.10 · Opaque entries: causal-apply parking, fork handling, resync diagnostic · Design §2. · deps: ht-j87z.3, ht-j87z.4
      owns: opaque-aware apply parking (same outcome as parked_unknown, re-attempted after upgrade), opaque-vs-known fork proof on prefix fields, 'cannot re-issue' diagnostic (daemon log line + sync status counter)
      consumes: StoredEntry, WireItem.
  - ht-j87z.11 · Journal reader seam refactor (conditional on spike) · Move-only refactor so one reader seam exists for journal rows (design §4). · deps: ht-j87z.1
  - ht-j87z.12 · Integration sweep: peer wire compat hardening · Verify the goal end to end: opaque relay (HTJRNL99 synthetic entry) across a capability-advertising relay, withheld toward a non-advertising peer; golden fixtur · deps: ht-j87z.1, ht-j87z.10, ht-j87z.11, ht-j87z.13, ht-j87z.2, ht-j87z.3, ht-j87z.4, ht-j87z.5, ht-j87z.6, ht-j87z.7, ht-j87z.8, ht-j87z.9
  - ht-j87z.13 · Promote stored opaque entries when the layout becomes understood · Design §2 (upgrade path). · deps: ht-j87z.10, ht-j87z.3
      owns: upgrade promotion: on start/apply, try to parse journal_opaque rows with the layouts the binary understands, move them into the known chain path and unpark successors
      consumes: StoredEntry, Chain-row reader, synthetic helper (.2).

## Changes since the previous round
- ht-j87z.13 created (promotion of stored opaque entries) — answers cov-3
- ht-j87z.1, .2, .3, .4, .6, .7, .8, .10, .11 amended (owns/acceptance/deps) — answer cov-1,4,5,6,7,8,9,10,11,12
- ht-j87z.4 now depends on .8; .7 depends on .8
