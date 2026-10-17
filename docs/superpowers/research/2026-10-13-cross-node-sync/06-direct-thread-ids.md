# Topic 6: Deterministic 1:1 (direct) thread IDs

Research for the vision spec's research question 6 (`docs/superpowers/specs/2026-10-13-cross-node-sync-vision-design.md`, "IDs" and "Research questions"). Standard mode, 2026-10-13.

## Recommendations

1. **Use a fixed-length hash, not raw concatenation.** `id = "d" + base62(SHA-256(input))[..10]`, where `input = "herdr-threads/direct/v1\0" || len(a) || a || len(b) || b` and `a < b` are the two **canonical** seat IDs under bytewise order. The order is Rust `String` `Ord`, which matches SQLite's BINARY collation and today's `CHECK(seat_a < seat_b)`. The ID is 11 characters, the same as `t` + a 2-char node ID + 8, so it is no longer than node-embedded random thread IDs.
2. **Leave node IDs out of the hash input.** Seat IDs already embed their home node, so adding node IDs adds no uniqueness. It would also make the thread ID depend on a second identifier that can change (a host-key rotation re-derives the node ID).
3. **Use a distinct prefix (`d`).** Direct IDs then never collide with random `t…` IDs. The prefix also shows that the ID has no single origin node, which is an exception to "IDs embed the origin node".
4. **Treat a hash collision as an error you detect and report, never a merge.** Unlike random IDs (`store::public_ids` regenerates on collision), a deterministic ID cannot be re-rolled: both nodes must compute the same value. Every create fact carries its `(seat_a, seat_b)` tuple. On ingest, a thread ID that already exists with a different tuple is refused for the later fact, ordered by `(hlc, node)`, and raises an operator warning. At 10 base62 characters (59.5 bits) the odds are about 6e-9 for 10^5 pairs ever created cluster-wide (table below).
5. **Look the pair up through the replicated `direct_threads` mapping, and compute the hash only when minting.** Legacy pairs keep their random `t…` IDs. They are safe because before linking every pair is intra-node (both seats on one node). Only a pair member's home node can author the pair's creation, so that node is the only one that can ever create it, and no split can duplicate it. Backfill emits one `direct_pair(a', b') → t_legacy` fact with the re-keyed seat IDs.
6. **Hash canonical (re-keyed) seat IDs only, never aliases.** Do the re-key before the node's journal starts (that is, before backfill), so no replicated fact ever names an old ID. Old IDs stay node-local aliases that are resolved before lookup and before hashing.
7. **Replacement and retirement follow existing seat continuity.** `rebind --replace` keeps OLD's ID, so OLD keeps its pair thread. A genuinely new seat is a new identity and gets a new pair thread. The retired seat's thread stays as history.
8. **Make concurrent creation idempotent. Every field of the create fact is a pure function of the ID.** Those fields are the kind, the pair and the default label `1:1 <a> <b>`.
   - **Creator and creation time:** the canonical creator and `created_at` come from the earliest create fact by `(hlc, node)`. The other create fact stays in the journal but adds no second "created" timeline event.
   - **Default topic:** the create fact must **not** write the topic/name LWW register. The default label is the register's bottom value, so a later-HLC duplicate create cannot revert a topic already set on the other side.
   - **Crossed invitations:** each side's invitation to the peer arrives for a seat that has already joined. The invitee's home node settles it automatically (`already_joined`, no nudge, including a `require_ack` invite) by authoring the settle fact, which the authoring model allows.

## Current code (baseline)

- `resolve_direct_pair` (`src/store/control.rs` ~L623-765) sorts the two seats, looks up `direct_threads(instance_id, seat_a, seat_b)`, and on a miss mints a **random** `t…` ID through `public_ids::fresh`. In the same transaction it then inserts the thread with topic/goal `1:1 <a> <b>`, the caller's `joined` membership, a `create_thread` event, the recipient's `invited` membership with an invitation, and the `direct_threads` row.
- `migrations/0029_direct_threads.sql`: `CHECK(seat_a < seat_b)`, `UNIQUE(instance_id, seat_a, seat_b)`, `thread_id UNIQUE`. The `instance_id` scope has to become cluster-wide for replicated pairs.
- `src/protocol/ids.rs`: compact IDs are `<prefix><8 base62>` (about 2^47.6 values) and parse as opaque, at most 128 bytes, printable ASCII. Legacy IDs with long prefixes persist unchanged.
- `sha2 = "=0.10.9"` is already a dependency (`Cargo.toml`; used in `src/ports.rs`, `src/archival_legacy.rs`).
- TRUST-POLICY: seats are never merged. `rebind --replace` retires NEW and OLD keeps its target ("nothing moves from NEW to OLD") (TRUST-POLICY.md, C3). The same rule holds in `src/store/seats.rs` (~L3358: "NEW retires and OLD claims the target").

## Prior art

**Matrix: random room IDs cause duplicate DMs (glare).** MSC2199 (Canonical DMs, still open [S2]) needs a "glare" procedure because two users who DM each other at the same time each create a room: "There is a potential for two users to start a DM with each other at the same time, preventing either server from short-circuiting the `/createRoom` call" [S1 §Glare]. The fix runs after the fact: order the conflicting rooms' creation events by `origin_server_ts`, keep the oldest as canonical, and tombstone the rest. The worst case is "a split DAG on the tombstone with two tombstone state events" [S1]. The proposal also requires "Servers MUST return an existing room ID if the server already knows about a DM between the important users" [S1], which is the same look-up-before-create rule our `resolve_direct_pair` follows. Deterministic IDs remove the duplicate itself, so no tombstone or union step is needed.

**Matrix room v12 (MSC4291): hashed IDs alone do not deduplicate.** The room ID becomes the hash of the `m.room.create` event: "`!` + 43x unpadded urlsafe base64" [S3]. That rules out ID spoofing, but the create event contains per-creation content, so two concurrent creations still produce two rooms. To deduplicate, the hash input must be only the identity tuple (the pair), not the create event. MSC4291 also points out the cost of not embedding the origin: "It is not immediately obvious which server created the room" [S3]. Our earliest-create-fact rule (rec. 8) covers this.

**Telegram, WhatsApp and Slack: the peer *is* the conversation key.** Telegram's `Peer` for a private chat is `peerUser { user_id }` [S4], so the chat ID is the other party's ID (fine on a central server, which sees one viewpoint). WhatsApp keys chats by the peer's JID (`user@s.whatsapp.net`, groups `@g.us`) [S6]. Slack's `conversations.open` with a single user "creates a 1:1 DM", and its `prevent_creation` flag checks "if there is an existing dm" [S5], so there is at most one DM per pair. In a symmetric peer-to-peer system the viewpoint-free equivalent is the sorted pair.

**WhatsApp LID migration: re-keying a participant ID duplicates conversations.** Once WhatsApp moved contacts from phone JIDs to `@lid` JIDs, a client that did not consult the LID↔phone map ended up with "Two separate chat entries … for the same contact — one under `@s.whatsapp.net`, one under `@lid`", and lost messages from queries [S7]. The same failure would hit us if one node hashed an old seat ID and another the re-keyed one. Hence rec. 6: hash only canonical IDs, resolve aliases before lookup, and finish the re-key before the journal exists.

## Analysis

### Raw concatenation vs. hash

| Option | Length (rekeyed seats `s`+2..4+8) | Collision | Notes |
|---|---|---|---|
| Raw `d<a>.<b>` | 23-27 chars | none (injective with a separator) | Easy to debug. Too long for IDs agents read constantly. A legacy long-prefix ID could push it past the 128-byte `ThreadId` limit. Leaks both seat IDs (harmless here). |
| SHA-256, base62, 8 chars | 9 | 2.3e-5 at 10^5 pairs | Matches today's random suffix length, but a collision cannot be re-rolled. |
| **SHA-256, base62, 10 chars** | **11** | **6.0e-9 at 10^5 pairs** | Recommended. |
| SHA-256, base62, 12 chars | 13 | 1.5e-12 at 10^5 pairs | Use if 2 extra characters are acceptable. |

Collision odds use the birthday bound p ≈ 1 − exp(−N²/(2·62^k)), computed for N pairs ever minted cluster-wide. Only direct IDs compete, because the `d` prefix separates them from random IDs:

| k | bits | N=10^4 | N=10^5 | N=10^6 |
|---|---|---|---|---|
| 8 | 47.6 | 2.3e-7 | 2.3e-5 | 2.3e-3 |
| 10 | 59.5 | 6.0e-11 | 6.0e-9 | 6.0e-7 |
| 12 | 71.5 | 1.5e-14 | 1.5e-12 | 1.5e-10 |

A realistic upper scale is 10 nodes with a few thousand seats each over their lifetime, so about 10^4-10^5 pairs. NIST notes that truncating a digest reduces its collision resistance to λ/2 bits for λ retained bits [S8, search snippet], which is the bound used above. The trust model is cooperative, so no adversarial collision search applies.

**Hash choice.** SHA-256 is already a dependency, fully specified and stable across versions. BLAKE3 would add a dependency for no benefit at this volume. Never use `std` `DefaultHasher`: "its hashes should not be relied upon over releases" [S9]. A mixed-version cluster (versionless rule) would then compute different IDs. Encode it like `public_id_suffix`: read the first 16 digest bytes as a `u128` and take k base62 digits. The modulo bias (62^10/2^128) is negligible. Use length-prefixed fields and a version tag in the input. If the derivation ever changes, it takes a new prefix and the lookup-first rule (rec. 5) keeps old pairs reachable.

### Node IDs redundant?

Yes. A seat's home node is fixed (seat existence is authored by its home node) and is embedded in the seat ID, so `(nodes, seats)` carries no more information than `(seats)`. Node IDs come from SSH host keys (spec "Node"). A host-key rotation would re-derive the node ID and fork every pair that node is part of. The re-key is one-time, so seat IDs do not have this problem.

### Aliasing and re-keying

- The deterministic rule only needs to hold for pairs that can be created on two nodes, which means cross-node pairs. Those exist only after linking, and so after the re-key.
- Legacy intra-node pairs keep their random thread IDs, and their backfilled `direct_pair` fact names the canonical seat IDs. Recomputing the hash for them would rename existing threads, and agents hold those thread IDs in context.
- All lookups go `alias → canonical seat → direct_threads`, and the hash is computed only on a miss.

### Retirement and replacement

- `rebind --replace` keeps the surviving seat's ID, so the pair thread continues.
- A retired seat cannot be a direct recipient (`eligible_delivery_recipient`). Its pair thread is archived history.
- A new seat for the same role is a new identity with a new pair thread. This matches "never merge seats" and the Matrix soft-tombstone stance ("create a new room for the DM and consider the existing one as soft-tombstoned" when rejoin fails [S1]).

### Concurrent creation merge (worked case)

Split: seat `a` on X and seat `b` on Y both ensure the pair. X authors `create(d…, a, b)`, `join(a)` and `invite(b by a)`. Y authors `create(d…, a, b)`, `join(b)` and `invite(a by b)`. After the heal:
- The thread exists once. Its canonical creator and `created_at` come from the earlier create fact, and the duplicate create is a no-op.
- Both seats are `joined`. Each crossed invitation is settled `already_joined` by the invitee's home node, with no nudge. This keeps Delivery-contract rule 2: nothing wakes an idle seat for a moot invite.
- Messages from both sides interleave by local ingest order (spec "Ordering").
- The topic/name register is untouched by either create fact, so any explicit topic set wins.

## Limitations

- The scale estimates (number of pairs) are assumptions, not measurements.
- The NIST truncation claim is cited from a search snippet [S8]. The full PDF was not retrieved.
- The membership and episode merge semantics belong to the sync/materialization research topics. Only the direct-thread consequences are covered here.
- Signal's 1:1 identifiers were not checked (only its group-ID derivation surfaced). Telegram, WhatsApp and Slack cover the peer-keyed pattern.

## Methodology

Tools: the `search` CLI (Brave provider; preflight `--help` OK), `curl`, and `gh api`, plus reading the repo. There were 8 search queries, about 43 result entries were considered, and 8 original documents were retrieved. One retrieval failure: the MSC2199 `old_master` raw path returned 404 and was fetched from the `matrix-doc` branch instead. Ledgers are in the session scratchpad (`t6/`).

## Bibliography

- [S1] MSC2199 Canonical DMs, rendered proposal: https://github.com/matrix-org/matrix-doc/blob/travis/msc/immutable-dms/proposals/2199-canonical-dms.md
- [S2] MSC2199 PR (state: open): https://github.com/matrix-org/matrix-spec-proposals/pull/2199
- [S3] MSC4291 Room IDs as hashes of the create event: https://github.com/matrix-org/matrix-spec-proposals/blob/matthew/msc4291/proposals/4291-room-ids-as-hashes.md
- [S4] Telegram API, type Peer: https://core.telegram.org/type/Peer
- [S5] Slack `conversations.open`: https://docs.slack.dev/reference/methods/conversations.open
- [S6] whatsmeow `types` (JID servers): https://pkg.go.dev/go.mau.fi/whatsmeow/types
- [S7] whatsapp-cli issue #15 (LID migration duplicates chats): https://github.com/vicentereig/whatsapp-cli/issues/15
- [S8] NIST SP 800-107r1: https://nvlpubs.nist.gov/nistpubs/Legacy/SP/nistspecialpublication800-107r1.pdf
- [S9] Rust `DefaultHasher`: https://doc.rust-lang.org/std/collections/hash_map/struct.DefaultHasher.html
