# Peer wire compatibility hardening — design

Run: [2026-10-18-peer-wire-compat-hardening](.) · Mode B (one-shot) · base `main` @ v2.1.1 (schema 3)

## Goal

A herdr-threads node of this release relays, byte-exactly and verifiably, a journal entry whose signed layout it does not understand; every shipped wire shape stays decodable by CI-frozen fixtures; and a repo check fails any change that would make the peer wire less version-tolerant.

## Problem description

The peer protocol evolves by convention only: unknown frames and fields are tolerated, optional fields are `#[serde(default)]`, non-base frames are gated by capability strings. Nothing enforces this, no test replays bytes that older releases really emitted, and the signed unit cannot evolve at all: `Envelope` (`src/sync/envelope.rs`) is a fixed struct whose canonical bytes (`HTJRNL01`) are recomputed from named fields, `WireEnvelope` is a fixed 13-field JSON mirror, and the store (`src/store/journal.rs`, `RawEnvelope`) keeps decomposed columns and re-serialises canonical bytes when serving. A node that meets an entry in a newer layout can neither verify it, store it, nor forward it, so a mixed-version mesh partitions at the first layout change even though relaying needs no understanding of the content.

## Main challenges

(a) A relay must not trust unauthenticated header claims: counter/prev/log ids that are not bound to the signed bytes would let a middle node lie. (b) The store, ingest, serve, resync and fork paths all assume decomposed rows; opaque entries must join the hash chain without spreading special cases. (c) 2.1.1 peers are live and cannot be changed: nothing new may be sent to a peer that did not advertise a capability, and a 2.1.1 node reading an opaque entry would answer `protocol_error` and be redialed forever. (d) Frozen fixtures must be reproducible from the repository, not from retained binaries. (e) A lint over Rust source without a parser must not be so fragile that people disable it.

## Key decisions made

1. Add a layout-agnostic *prefix invariant* and an *opaque entry* path, gated by the new capability `envelope.opaque.v1`; do not change `HTJRNL01`, the signed fact shapes or the JSON framing.
2. Freeze fixtures by deterministic generation (seeded keys, fixed HLC) at a release SHA via a script, store them under `tests/fixtures/peer_wire/<version>/`, and decode all of them in one test; forward-compat is tested with a frozen *shadow* copy of the 2.1.1 wire structs in test code, not with retained binaries.
3. Implement the lints as `scripts/check-wire-compat` (awk-based, baseline-file driven, in the style of `check-harness-neutral`), wired into the per-change checks in AGENTS.md.
4. Ship as one minor release (additive capability, additive tests); the envelope work starts with a spike that counts the paths touching `journal` rows before the schema change is fixed.

## Decision points, by section

### 1. The prefix invariant

Recommended: every envelope generation starts with `magic[8] ‖ node[16] ‖ incarnation u32 ‖ counter u64 ‖ prev_hash[32]` (the first 68 bytes), `hash = SHA-256(all canonical bytes)`, `signature = ed25519(author key, hash)`. HTJRNL01 already satisfies this (`Body::canonical_bytes`); a unit test pins it, and the doc comment on `MAGIC` states that a new generation must keep it. With the prefix and the hash binding, a relay can verify without understanding the rest: parse 68 bytes, check they equal the header fields it was sent, hash the raw bytes, verify the signature with the log's genesis public key. Discarded: a bespoke signed header wrapper around opaque bytes (new signature domain, double signing, the author would sign twice); trusting unsigned header fields (relay lying); requiring all generations to be a superset of v1's field set (cannot be enforced).

### 2. The opaque entry

Recommended: a received entry whose magic is not `HTJRNL01` and whose prefix, hash and signature verify is an *opaque entry*: `{log, counter, prev_hash, hash, signature, raw}`. It joins the log's hash chain (continuity checks use only prefix fields), is stored verbatim, is served verbatim, counts toward `have` heads, and is never applied: causal apply of that log stops at it and later entries of the log park as `parked_unknown` does today for an unknown kind, re-attempted after an upgrade (`kind` unknown, major unknown already parks; this is the same outcome one level lower). Fork proofs between an opaque and a known entry at the same counter or prev-link work on prefix fields; resync and fork re-issue treat an opaque entry as "cannot re-issue, report" (a re-issuing owner must understand the entry; this is documented, tested, and surfaced as a diagnostic, not silently dropped). Discarded: dropping unknown-layout entries (partitions the mesh); storing them in a side table disconnected from the chain (breaks `have` heads and fork proof); applying a best-effort parse (unsound).

### 3. Wire form and capability gating

Recommended: the `entries` / `branch_entries` item type gains an alternative opaque item `{ "opaque": { node, inc, counter, prev, hash, sig, raw } }` (lowercase hex), decoded by `PeerFrame::decode` into an enum `WireItem { Known(WireEnvelope), Opaque(OpaqueWire) }`. Capability `envelope.opaque.v1` is advertised in the hello `caps`; a sender emits an opaque item only to a peer that advertised it, and withholds the entry otherwise: the peer's `have` head simply stays behind that counter (no error, no `protocol_error`; a per-link diagnostic counter records the withheld entries). A 2.1.1 peer therefore never sees an opaque item. A known-layout entry is always sent in the existing shape, so mixed 2.1.1 ↔ new traffic is byte-identical to today. Discarded: a protocol-version number (policy is versionless); sending a `raw` field next to the named fields of every entry (doubles every frame, 2.1.1 would ignore it but gain nothing); a new frame type (the item alternative keeps flow control, batching and halving unchanged).

### 4. Storage

Recommended: a new table `journal_opaque(log_ix, counter, raw BLOB, hash, signature, prev_hash)` joined to the chain by `(log_ix, counter)`, with the same guard triggers and fork-freeze semantics as `journal`, and one reader function `Journal::entries_from` returning `StoredEntry::{Known(Envelope), Opaque(OpaqueEntry)}` so serve/resync/ingest each match once. Schema bumps to 4 through the existing migration mechanism (additive table, no change to `journal` rows or the signed bytes); an older binary opening a schema-4 store is already handled by the repo's downgrade policy (it refuses, unchanged). A first spike (task S1) enumerates every reader of `journal` rows and reports the match sites before the table shape is fixed; the design assumes a single reader seam and the spike either confirms it or adds the missing seam as a move-only refactor first. Discarded: nullable columns in `journal` (every NOT NULL invariant and guard trigger would be weakened for all rows); storing only the canonical bytes for all entries (rewrites storage for the common case).

### 5. Mixed-version behaviour

Recommended: the capability makes every combination explicit — new↔2.1.1: no opaque items ever; new↔new: opaque items flow once a future generation exists; future-generation author ↔ 2.1.1 node: not relayed through a 2.1.1 node (documented limit in `docs/operations.md`, recorded in TRUST-POLICY if the trust text mentions relay). Nothing in this release authors a non-HTJRNL01 entry; the opaque path is exercised by tests that author a synthetic `HTJRNL99` entry honouring the prefix invariant and signed by a test key. Discarded: shipping a v2 layout now (no need yet); making 2.1.1 interoperate (impossible without changing it).

### 6. Golden peer-session fixtures

Recommended: `scripts/freeze-peer-wire <version>` (invoked at a release SHA) runs a deterministic session through two in-process coordinators with seeded `NodeIdentity::from_seed` keys and a fixed HLC, records every frame in each direction as one JSON line per frame (`{"dir":"a>b","frame":{...}}` with raw frame text kept verbatim) under `tests/fixtures/peer_wire/<version>/session.jsonl`, plus `envelopes.jsonl` holding one signed entry of every fact kind in `FactRegistry::builtins()` and the fork frames, and a `MANIFEST.md` naming the SHA and generator. The initial fixture is `2.1.1`, generated by running the same script from a worktree at tag `v2.1.1` (the script and its test-support hooks are added there as a patch outside the tag's source by being a pure observer over `PeerFrame` encode: if the tag lacks a hook the script falls back to encoding frames by constructing them from the public types, documented). A new test (one `[[test]]` entry with `required-features = ["test-support"]`, or a `#[path]` module in `tests/combined.rs` if it keeps no process state) decodes every directory: each frame through `PeerFrame::decode`, each envelope through `Envelope::try_from` + `verify`, each fact through `FactRegistry` lookup + payload decode, asserting no rejection, no hash/signature failure, and every fact kind is registered or intentionally parked. Fixtures are append-only: a test fails if an existing directory file changes (hash list in `MANIFEST.md`); the release checklist in `docs/release.md` gains "run scripts/freeze-peer-wire <new version>". Discarded: regenerating fixtures on every test run (proves nothing about old bytes); retaining old binaries in CI (weight, platform pain); a pure property test (no real historical bytes).

### 7. Forward-compat half

Recommended: `tests/support/legacy_wire_v211.rs` holds a frozen copy of the 2.1.1 peer wire structs and their `decode` behaviour (frame enum with `#[serde(other)] Unknown`, `PeerHello`, `HaveLog`, `WireEnvelope` with exactly its 13 fields). Tests (a) encode every frame the current code can emit for a peer that did NOT advertise the new capability and decode it with the shadow decoder, asserting success and equal known-field values; (b) mutate fixtures with extra fields, an unknown `t`, an unknown close reason and an unknown capability, and decode with the current decoder. The shadow is edited only by adding a new frozen sibling file for a new baseline release, never in place; a header comment states it is deliberately a copy. Discarded: depending on the old crate version via Cargo (git tag dependency, whole-crate compile, cannot link two versions of the same crate cleanly).

### 8. The lints

Recommended: `scripts/check-wire-compat` (bash + awk, `--root`, `--json`, exit 0/1/2 like `check-harness-neutral`) over a closed list of wire files (`src/sync/frame.rs`, the `WireEnvelope` block in `src/sync/envelope.rs`, `src/sync/facts/payload.rs`, fact payload structs in `src/sync/facts/*.rs`, `src/protocol/sync_admin.rs`). Rules: (R1) no `deny_unknown_fields` anywhere in those files; (R2) every field of a listed wire struct not present in `scripts/wire-compat-baseline.txt` must carry `#[serde(default` (baseline = today's fields, one `Type.field` per line, generated by `--suggest`); (R3) every `PeerFrame` variant not in the baseline's `variant` rows must have a `// cap: <name>` line directly above it and `<name>` must be a `pub const` string in `frame.rs`; (R4) the baseline file may only grow by lines whose type is new, enforced in review by the check printing the delta. Stale baseline lines fail the check, as in `check-harness-neutral`. Wired into AGENTS.md "Development rules" next to the other per-change check, and into CI where `check-harness-neutral` runs. Discarded: a `syn`-based Rust test (new dev-dependency, brittle against macros); clippy lint plugin (not stable); doing nothing but code review.

### 9. Out of scope

No transport change (the ssh research recommends keeping ssh stdio); no serialization change (the transport research recommends keeping JSON + u32 framing); no change to `message.sent` or any fact payload (ht-i4dd owns that); no authoring of a new envelope generation.

### 10. Testing and rollout

Unit: prefix invariant pin; opaque verify (good, wrong hash, wrong signature, prefix/header mismatch, prefix says other log, counter gap, prev break); store round trip; serve emits opaque only with the capability; fork between opaque and known. Integration: two-node session where the author emits `HTJRNL99`: a capability-advertising relay forwards it, a hello without the capability withholds it and `have` stays behind; fixture decode test; shadow-decoder test; lint self-test with fixtures of passing and failing source. Release as a minor (`v2.2.0` is already branched as `release/2.2.0` in another worktree; the integration branch merges into main and the releasing agent picks the version per the versioning rule: additive feature = minor).

## Key abstractions

- **EntryPrefix** (`src/sync/envelope.rs`): a `Copy` value for the 68-byte generation-independent prefix `magic[8] ‖ node[16] ‖ inc u32 ‖ counter u64 ‖ prev[32]`. `Body::canonical_bytes` writes its first 68 bytes through it, so HTJRNL01 meets the invariant by construction. Must not read past byte 68 or decide whether a magic is known. Short buffer: `EnvelopeError::Malformed{field:"prefix"}`.
- **OpaqueEntry**: a verified unknown-layout entry `{log, counter, prev_hash, hash, signature, raw}`. Shape check against its wire header (prefix equals header, magic is not HTJRNL01, raw within bound) plus `SHA-256(raw)==hash` and `verify_strict`. Must not parse `raw` past the prefix or be applied. Reuses `EnvelopeError` (`Malformed{"prefix"|"magic"}`, `HashMismatch`, `BadSignature`).
- **StoredEntry**: closed enum `{Known(Envelope), Opaque(OpaqueEntry)}` in the sync layer, the one entry type JournalPort, chain rules, coordinator and incarnations exchange; log/counter/prev/hash/verify/size/genesis-candidate are one `match` each. A generation the node learns to understand becomes a parse into `Known`, never a new variant. An opaque entry at counter 1 of an unknown log is `Corrupt(NotGenesis)`.
- **WireItem** (`src/sync/frame.rs`): wire mirror `{Known(WireEnvelope), Opaque(OpaqueWire)}` for `entries`/`branch_entries` items, hand-written decode peeking the single `opaque` key (serde `untagged` cannot hold `RawValue`). Must not decide whether an item may be sent.
- **Opaque capability gate** (`src/sync/coordinator/serve.rs`): the one outbound function that, per link, emits opaque entries only if `envelope.opaque.v1` was negotiated, else cuts the batch before the first opaque entry and counts it withheld (no error, no close, no hot-loop re-serve). Reads `LinkEntry::opaque_cap()` beside `fork_cap()`; runs before `encode_halved`.
- **Chain-row reader** (`src/store/journal.rs`, behind `Journal::entries_from`): reads chain positions as `StoredEntry` from the union of `journal` and `journal_opaque`; replaces the `RawEnvelope::read` sites. Other readers stay known-only unless the spike (S1) finds otherwise.
- **Peer-wire recorder** (test-support only): decorator over `PeerIo` splitting u32 frames per direction into `{"dir","frame"}` lines verbatim; needs no production hook and runs unchanged against v2.1.1.
- Extended, not new: `JournalPort` (`serve`, `serve_branch`, `ingest`, `ForkEvidence` carry `StoredEntry`; SqliteStore and MemoryJournal both), `chain::check_next`, store ingest `ChainStep::Append` (inserts into `journal_opaque`), apply pass (log whose next position is opaque records `parked_unknown`/`blocked_on`), incarnations worker / resync salvage (distinct "opaque, cannot re-issue" report reason), `OPAQUE_CAP` pushed into hello caps beside `FORK_CAP`. New `OpaqueWire` fields are added to the lint baseline when the type is introduced (required fields, no `serde(default)`).

Digest:
- EntryPrefix: owns the 68-byte layout-agnostic prefix (write and parse); every envelope generation writes its first 68 bytes through it; callers never slice prefix offsets by hand.
- OpaqueEntry: owns shape-checking an unknown-layout entry against its header and verifying hash and signature over raw bytes; callers never parse `raw` past the prefix.
- StoredEntry: owns the layout-independent entry operations as one `match` each; a newly understood generation becomes a parse into `Known`, never a new variant.
- WireItem: owns the `entries`/`branch_entries` item alternative, its hand-written decode and StoredEntry conversion; callers never use serde `untagged` around a RawValue.
- Opaque capability gate: owns cutting served batches by the link's negotiated `envelope.opaque.v1` and counting withheld entries; serve paths never check capabilities themselves.
- Chain-row reader: owns reading chain positions from `journal` and `journal_opaque` as StoredEntry; serve, fork and branch code never query `journal_opaque` directly.
- Peer-wire recorder: owns verbatim per-direction frame capture by wrapping PeerIo in test-support; production code never gets a recording hook.

## Post-Implementation Notes

*As this design is implemented and iterated on — bug fixes, adjustments, anything that diverged from the assumptions above — append a dated note here, whether or not a formal debugging skill was used.*
