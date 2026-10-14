# 05: Node ID derivation and length

Research question 5 of the cross-node vision spec (`docs/superpowers/specs/2026-10-13-cross-node-sync-vision-design.md`, "Node" and "IDs").

## Recommendations

1. **Mint the node identity at random and do not derive it from the SSH host key.** Each store gets a 128-bit `node_uuid` the first time it runs, and that UUID is the journal's `origin_node`. The node ID should not depend on anything outside the store. The host key cannot tell apart several instances on one machine. It is often missing on the dialing laptop, and it changes when the OS is reinstalled or cloud-init runs again. It also gives back the same ID after the store is wiped, which is the most dangerous case (see "Why derivation hurts"). The vision text should be amended to match.
2. **Keep the identity apart from the short tag.** Add `node_tag`, 3 base62 characters drawn at random when the node is minted. The tag is only a namespace for minting public IDs. It is never used as the identity of a fact. A fact is identified by `(node_uuid, counter)`.
3. **Use exactly 3 characters, always.** With 10 nodes the chance of a tag collision is 1.9e-4, and with 100 nodes it is 2.1%. The handshake catches every collision, so these odds are only about how often an operator has to step in. A fixed length keeps parsing positional and makes old and new IDs easy to tell apart by length.
4. **Handle a collision by re-tagging, not by refusing to link forever.** At the handshake each side sends `(node_uuid, node_tag)`. If two different UUIDs share a tag, the node with the higher UUID draws a new tag before it mints anything else, and it avoids every tag it already knows. IDs it minted earlier keep the old tag and stay valid. They are already unique in practice, because the 8-character suffix still gives about 2^47.6 values for each tag. Literal ID clashes are still caught at ingest by the local UNIQUE check. Provide `node retag` for an operator.
5. **Use the handshake to detect clones.** If one `node_uuid` turns up with diverging counters, or from two places at the same time, the store has been cloned or restored. Refuse the link and point the operator to research question 9 (epochs). Record the host key fingerprint and hostname of each peer only as diagnostic metadata.
6. **Format public IDs as kind prefix, then node tag, then suffix:** `s` + `Ab3` + `k3Fq9a2B` gives `sAb3k3Fq9a2B` (12 characters, 9 today). Keep the 8-character suffix for v1.

## Which host key? (if derivation were kept)

- **A host has several keys, and the set changes.** OpenSSH's default `HostKey` list is now ECDSA, Ed25519, `mldsa44_ed25519` and RSA, and more than one key file is allowed [S3]. `ssh-keygen -A` creates every default type that is missing [S4]. So the answer to "the host's fingerprint" depends on which key type is picked, and the default set itself changes between releases.
- **Keys are expected to rotate.** OpenSSH has rotated host keys gracefully since 6.8. Keys the server no longer offers are removed automatically from clients' `known_hosts` [S1], and the IETF is standardizing the same mechanism [S2]. A node ID based on a key that the ecosystem intends to rotate becomes an identity change after every rotation.
- **Reinstalls and first-boot tools create new keys.** cloud-init deletes the keys baked into an image (`ssh_deletekeys`) and creates new ones [S6]. Lima VMs created new host keys on every stop/start, because cloud-init treated each boot as the first [S5]. Each of these events would silently produce a new node and orphan the old ID's seats and 1:1 thread IDs.
- **The dialing laptop may have no host key at all.** This macOS 26.6.2 machine has no `ssh_host_*` files in `/etc/ssh` (Remote Login is off) [L1]. Yet the spec's laptop that "links box1 and box2" has to be a node. Containers usually do not run sshd either.
- **The private key cannot be used.** It is root-only. sshd refuses group- or world-readable key files [S3]. herdr-threads runs as the user, so it could only hash the public `.pub` file. Anyone can read that file, so it proves nothing.
- **The fingerprint does not authenticate anything here.** On the dialed side, ssh checks the host key against `known_hosts`. The dialed side never sees the dialer's host key, only its user key. The trust model is cooperative and has the same user on every node (spec, "Trust"), so there is nothing for a fingerprint to prove.

## Several instances on one machine

A machine's host key is shared by every instance on it. The spec says a node is an instance, and `instance_dir = sha256(host_endpoint)` [R4]. A derived ID would therefore need an instance discriminator anyway, for example `H(fingerprint ‖ instance_uuid)`. herdr-threads already mints that UUID: the first time an instance is locked, it writes `Uuid::new_v4()` to `<instance_dir>/namespace` and keeps it [R1]. Once this random input is in the hash, the result is effectively random. The fingerprint then adds only the drawbacks described above. The only thing it would add is "the same machine gets the same ID again after a store wipe", and that is a hazard, not a benefit.

Recommendation: keep `node_uuid` in the store itself, for example in `host_instances` or a new single-row `node_identity` table. The `namespace` file is not enough on its own, because identity must move with the journal: a copied database file must carry its identity with it, and a new database must get a new one. If the namespace UUID is reused as the seed, store it in the database as well when it is first minted.

## Why derivation hurts: re-derivation after data loss

Peers hold a per-origin high-water mark (spec, "Sync"). Suppose a machine loses its store, gets a fresh install, and derives the same node ID again. Its counter starts at 1. Peers already hold that origin up to some higher counter N, so they silently ignore its new entries up to N, or treat them as duplicates of different facts. A random identity minted with the store avoids this completely: a new store is a new origin. A store restored from backup or cloned into an image still reuses its UUID and an old counter. Neither approach prevents that case. It needs the epoch and fork detection of research question 9, which the handshake can carry (recommendation 5). The systemd authors give the same advice about machine identity: keep the machine identifier confidential, and derive application identifiers from it with a keyed hash rather than reusing it [S7]. They also document that cloned images repeat machine-ids, which is the same failure as cloned host keys [S6][S7].

## Length

Exact birthday probabilities, computed as 1 − ∏(N−k)/N [S8]:

| alphabet | len | space | P(collide), 10 nodes | P(collide), 100 nodes |
|---|---|---|---|---|
| base62 | 2 | 3,844 | 1.2% | 73% |
| base62 | **3** | 238,328 | **1.9e-4** | **2.1%** |
| base62 | 4 | 14.8M | 3.1e-6 | 3.4e-4 |
| base36 (lower) | 3 | 46,656 | 9.6e-4 | 10% |
| Crockford32 | 3 | 32,768 | 1.4e-3 | 14% |
| Crockford32 | 4 | 1.05M | 4.3e-5 | 4.7e-3 |

- **Two characters are too few.** A 1% chance for a 10-node cluster is real, and every reinstall adds to the population of nodes ever minted.
- **Three base62 characters fit the 2-10 node target.** Collisions are detected anyway (recommendation 4).
- **Variable length (2-4) brings no benefit.** It makes `<kind><tag><suffix>` ambiguous to parse, and it forces every parser to know the tag length.

## Case sensitivity

Today's suffixes are already base62 and case-sensitive (`BASE62` in [R2]). A case-insensitive alphabet for the tag alone would not make whole IDs case-insensitive, and it would make the tag space 5-7 times smaller (see the table). Keep base62. One risk to note: if an ID ever becomes a filename, a default case-insensitive APFS volume would merge `sAb3…` and `saB3…`. Public IDs should therefore never be used directly as path components; hash them first, as `instance_dir` already does [R4].

## Placement and token cost

- **`<kind><tag><suffix>`** is the recommended layout:
  - Kind letters stay first, so the existing `is_short_public_id(prefix, …)` scheme carries over. Its length check changes from 8 to 3 + 8 [R2].
  - Suffix-sharing derivation still works unchanged: `p<tag><sfx>` maps to `m<tag><sfx>`, and `np…` maps to `n…` [R2].
  - Old 9-character IDs (`sk3Fq9a2B`) and new 12-character IDs differ by length. This allows the planned upgrade aliases (spec, "IDs") without a separate marker.
  - The same tag appears in every ID that node mints, so agents can easily see where an ID came from.
- **The tag should not go at the end.** A tag after the suffix would keep the kind letter in front, but the random part would come first. A reader then cannot see the origin at a glance, and longest-prefix completion is harder.
- **The suffix could be shorter, but should not be changed yet.** Within a tag, uniqueness is enforced by construction and by the local collision check [R3]. The suffix only has to be large enough not to run out and to keep retired-ID reuse rare (the TRUST-POLICY "Probabilistic identifiers" accepted limit, cited in [R3]). Cutting it to 7 characters (62^7 ≈ 2^41.7) would win back one character. Keep 8 for v1 and revisit after the token cost is measured.
- **Token cost was not measured.** No tokenizer was available offline. A plausible estimate is about +1 to +2 tokens for each ID occurrence for 3 extra random alphanumeric characters, but this is unverified. A follow-up should count tokens for a sample of `sXXXXXXXX` and `sTAGXXXXXXXX` IDs with the harnesses' tokenizers.

## Implications for other topics

- **Topic 6 (deterministic 1:1 thread ID):** derive it from `node_uuid` or from seat IDs, never from the mutable `node_tag`. Otherwise re-tagging after a collision would split pair threads.
- **Topic 9 (restore and clone):** handle these through the handshake check in recommendation 5.

## Limitations

- Token cost is not measured.
- Container and Docker host-key behaviour is reasoned, not sourced.
- The claim that macOS has no host keys rests on one local observation [L1].

## Methodology

- **Tools:** `search` CLI 0.9.0 (several providers; perplexity and serper failed with quota errors, so the remaining providers were used). Original documents were fetched through the r.jina.ai reader.
- **Search volume:** 7 query attempts and about 40 result entries considered. 7 original documents were retrieved, plus 4 repository files and one local system check.
- **Retrieval failures:** tiktoken is not installed, which left the token-cost gap.
- **When retrieval stopped:** once every facet except token cost had primary sources.

## Bibliography

- [S1] D. Miller, "Key rotation in OpenSSH 6.8+", https://blog.djm.net.au/2015/02/key-rotation-in-openssh-68.html
- [S2] IETF draft-ietf-sshm-hostkey-update-00, https://www.ietf.org/archive/id/draft-ietf-sshm-hostkey-update-00.html
- [S3] sshd_config(5), HostKey, https://man.openbsd.org/sshd_config
- [S4] ssh-keygen(1), `-A`, `-E`, https://man.openbsd.org/ssh-keygen
- [S5] lima-vm/lima #678, "SSH host keys are regenerated each time a VM does a stop/start", https://github.com/lima-vm/lima/issues/678
- [S6] cloud-init module reference (ssh: `ssh_deletekeys`, `ssh_genkeytypes`), https://cloudinit.readthedocs.io/en/latest/reference/modules.html
- [S7] machine-id(5), https://www.freedesktop.org/software/systemd/man/latest/machine-id.html
- [S8] "Birthday problem", Wikipedia, https://en.wikipedia.org/wiki/Birthday_problem
- [R1] `src/daemon/ownership.rs` (`OwnerLock::acquire`, namespace UUID)
- [R2] `src/protocol/ids.rs` (prefixes, `PUBLIC_ID_SUFFIX_LEN`, BASE62, derivations)
- [R3] `src/store/public_ids.rs` (local collision check)
- [R4] `src/daemon/paths.rs` (`instance_dir`)
- [L1] Local check: `ls /etc/ssh` on macOS 26.6.2 (no `ssh_host_*` files)
