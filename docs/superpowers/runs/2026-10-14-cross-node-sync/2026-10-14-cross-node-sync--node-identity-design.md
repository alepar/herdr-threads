# Cross-node sync: node identity, layout and schema baseline — design

Parent: [root spec](2026-10-14-cross-node-sync-design.md) · Bead: ht-5fr.1

## Goal

A herdr-threads daemon starts as one node per `(machine, OS user, state dir)`. It is addressed by its state dir only, holds a persistent Ed25519 identity with a `node_uuid` and a 3-character tag, mints every public ID as `<prefix><tag><8>`, and runs on a fresh clean-start store whose schema separates node scope (`local_node`) from Herdr-server scope (`herdr_hosts`) and gives seats a home node. It keeps running with zero Herdr servers attached. The pre-node `instances/` data is left untouched, reported by `doctor` and removed only by an operator's `legacy purge`.

## Problem description

Today a daemon, its store and its identity are keyed by `sha256(host_endpoint)` under `<state_dir>/instances/<digest>/` (`src/daemon/paths.rs` `instance_dir`, `InstancePaths`). The instance UUID is a random `namespace` file (`src/daemon/ownership.rs`). `daemon run` needs `--host-endpoint` (`src/main.rs` `detached_child`, `src/daemon/lifecycle.rs` `spawn_detached`). Public IDs are `<prefix><8 base62>` (`src/protocol/ids.rs`, `src/store/public_ids.rs`) and are unique only within one store. The schema is a 46-step migration chain (`src/store/schema.rs`, `migrations/0001..0046`) whose `host_instances` row mixes node-wide revisions with one Herdr server's boot, epoch and observation sequences. Every later subepic of the cross-node tree needs three things this subepic provides. First, a stable, signing-capable node identity (journal core, peer links). Second, IDs that cannot collide across nodes (fact kinds). Third, a schema in which a node can have zero or many Herdr servers and in which seats can belong to other nodes (HostSessions, fact kinds).

## Main challenges

The key must be created exactly once and must never be silently replaced. A replacement would orphan the node's self log, and an accidental new identity next to an old store would look like a restore. Tags must be derivable by every node without coordination, and the re-tag rule must be deterministic. The rename from instance to node touches about 26 files that use `InstancePaths` and every query that reads `host_instances`. Moving the Herdr-scope columns must leave today's single-host daemon working, so HostSessions (ht-5fr.2) can generalize it without first repairing a broken build. Squashing 46 migrations into one baseline must keep every object today's code depends on, including inert backfill singletons that code still reads. The daemon must start, serve and stay up with no Herdr server, although today `run_elected_guarded` takes a mandatory `HostPort`. And the clean start must not destroy, open or migrate anything under `instances/`, while still telling the user that the data exists.

## Key decisions made

`src/node/` holds `NodeIdentity`, `NodeUuid`, `NodeTag` and the pure tag and retag functions. The key is a raw 32-byte Ed25519 seed in `node.key` (0600, created with a no-clobber link, never rewritten), and `ed25519-dalek` is the only new crypto dependency. The node refuses to start when a store exists but its key is missing, or when the key and the store disagree. `InstancePaths` becomes `NodePaths` rooted at `<state_dir>/node/`. `RuntimeContext.host_endpoint` becomes `Option` (the claim host). `daemon run` takes `--state-dir` plus an optional `--host-endpoint` attach hint. Until ht-5fr.2, the daemon attaches at most one Herdr server (the *primary host*) and otherwise runs store-only. The store is a new file `herdr-threads.db` with one squashed baseline, `migrations/node/0001_node_baseline.sql`. The baseline stays editable in place until the first release that ships it, and it is verified at open against an in-memory copy of itself. The legacy migration chain is deleted. `host_instances` becomes the singleton `local_node`. Its Herdr-scope columns move to `herdr_hosts`. The ~100 `instance_id` columns keep their name and now hold the local `node_uuid`. Herdr-scope child tables are re-keyed to `host_id` by ht-5fr.2. Seats gain `home_node`, `is_local`, `remote_kind`, `host_id` and the `remote` state, with trigger guards. Public IDs read the tag from `local_node` inside the minting transaction. The direct-thread ID function is pure, in `protocol::ids`.

## Decision points

### 1. Node identity and key custody (`src/node/identity.rs`)

**Recommended.** Add a new core module `src/node/` (`mod.rs`, `identity.rs`, `tag.rs`, `json.rs`), exported from `src/lib.rs` as `herdr_threads::node`:

- `pub struct NodeUuid([u8; 16])` holds the first 16 bytes of SHA-256 over the 32-byte Ed25519 public key. `Display`/`FromStr` use 32 lowercase hex characters with no hyphens. `Ord` compares bytes, which is the same as comparing the lowercase hex strings, so "higher `node_uuid`" means the same thing everywhere. Serde writes it as that string. It is never a `uuid::Uuid` in hyphenated form: every place that today prints `instance.to_string()` prints `NodeUuid`'s `Display`.
- `pub struct NodeIdentity { uuid: NodeUuid, signing: ed25519_dalek::SigningKey, public: [u8; 32], tag_k: AtomicU32 }`. It is loaded once by `NodeIdentity::load_or_create(paths: &NodePaths, store_exists: bool) -> Result<NodeIdentity, NodeKeyError>` and held as `Arc<NodeIdentity>`. Methods: `uuid()`, `public_key() -> [u8; 32]`, `sign(hash: &[u8; 32]) -> [u8; 64]` (infallible), `tag() -> NodeTag` and `tag_k() -> u32` (both from the atomic), and `set_tag_k(k)`. Only `set_tag_k` changes the tag. The daemon calls it at start with the value read from `local_node.tag_k`, and the `node.retag` apply handler (ht-5fr.4) calls it after the row commits.
- **Key file.** `node.key` contains exactly 32 raw bytes (the Ed25519 seed). It is mode 0600, owned by the effective uid and a regular file, not a symlink. Creation: 32 bytes from `getrandom`, written to `node.key.tmp.<pid>` (O_CREAT|O_EXCL, 0600), fsynced, then `link()`ed to `node.key`. `EEXIST` means another creator won, so re-read that key. Then unlink the temp file and fsync the directory. The key is never rewritten, truncated or regenerated.
- **Start rules** (`NodeKeyError`, a closed enum that names the path and is different from store errors):
  - `Missing` → create only when `store_exists == false`. Else `KeyMissingForStore`: the store `herdr-threads.db` exists but the key does not. The daemon refuses to start with the remedy "restore node.key, or move herdr-threads.db away to start as a new node".
  - `Unreadable(io)`, `Permissions{mode, uid}` (any group/other bit, or another owner), `NotRegularFile`, `Malformed{len}` (length ≠ 32) → refuse, never repair.
  - After the store opens, `local_node.id ≠ identity.uuid()` → `IdentityMismatch{store, key}`, refuse (§5).
- Dependencies: `ed25519-dalek` pinned exactly to the current 2.x release in the repo's `=x.y.z` style, `default-features = false`, features `std` and `zeroize` (no `rand_core`), and `getrandom` as a direct dependency pinned to the version already in `Cargo.lock`. `NodeIdentity` depends on nothing in `store`, `sync` or `daemon` except the `NodePaths` value.

**Considered.**
- A PEM or OpenSSH key format: it adds a parser for no benefit, since the file is private to one program.
- Creating a new key whenever the old one is missing: this silently forks identity next to an existing store (root §10 treats key loss as a *new node*, which requires a new store as well).
- Storing the key inside SQLite: a `VACUUM INTO` backup or a copied store would then carry the secret, and the root assigns the key to its own file.
- Generating the seed with `rand_core`/`OsRng`: one more feature surface, while `getrandom` is already in the tree.

### 2. Tags and the retag rule (`src/node/tag.rs`)

**Recommended.**
- `pub struct NodeTag([u8; 3])`, base62 in the alphabet `0-9A-Za-z` (the same constant as `protocol::ids::BASE62`, moved to a shared `pub(crate) const`).
- `pub fn tag_for(uuid: NodeUuid, k: u32) -> NodeTag`:
  - digest = SHA-256(`uuid bytes (16)` ‖ `k as u32 big-endian (4)`);
  - n = the first 16 digest bytes as a big-endian `u128`;
  - emit 3 digits by repeating `BASE62[n % 62]; n /= 62`, the first emitted digit first.

  The shared helper `pub(crate) fn base62_digits(n: u128, len: usize) -> String` (in `protocol::ids`) is also used by direct-thread IDs (§7). Golden vectors are frozen in tests: three fixed seeds → `node_uuid` → `tag_for(_, 0)` and `tag_for(_, 1)`.
- `pub enum RetagDecision { Keep, MoveTo { k: u32, tag: NodeTag } }`.
- `pub fn on_collision(self_uuid, self_k, other_uuid, other_tag) -> RetagDecision` returns `MoveTo{k: self_k+1, …}` only when `other_tag == tag_for(self_uuid, self_k)`, `other_uuid != self_uuid` and `self_uuid > other_uuid`; otherwise `Keep`. It is pure. The higher uuid moves to the next k, as the root rule says, and a new collision at k+1 simply triggers the rule again.
- Who calls it: ht-5fr.4's `node.genesis`/`node.retag` apply handlers. When this node must move, the handler authors `node.retag{k, tag}`, and applying that fact updates `local_node.tag`/`tag_k` and then calls `NodeIdentity::set_tag_k`.
- This subepic ships the function, the `node_directory` table (§5) and the store helper `node_directory::colliding(conn, tag) -> Vec<NodeUuid>`.
- IDs minted under an old tag stay valid: nothing parses a tag back out of an ID to route anything.

**Considered.**
- Random tags drawn at mint time (research 05): the root fixed derivation from `node_uuid` and k, so every node can recompute any tag.
- Skipping tags already known to collide: the outcome would depend on what each node has seen, while the strict k+1 rule converges with no extra state.
- An operator `node retag` command: not needed, since the rule is automatic.

### 3. `node.json`

**Recommended.** `node.json` is `{"format":1,"node_uuid":"<32 hex>","tag":"K3q","tag_k":0,"created_at":<ms>}`.
- It is derived data. The elected daemon regenerates it (atomic write via the existing `atomic_write`, mode 0600) at start, after the store opens, whenever the content differs or the file is missing or unparsable, and again after a retag applies. `created_at` is `local_node.created_at`.
- Readers that have no daemon use it as a hint and never as authority:
  - descriptor validation (it replaces the `namespace` file, §4);
  - `doctor`;
  - later `peer --stdio` diagnostics.

  A missing or corrupt `node.json` means "identity unknown", which today's missing `namespace` file already means (`lifecycle.rs` `instance_uuid` → `None`).

**Considered.** Making `node.json` authoritative for the tag: two writers (the file and the store) could disagree. The store's `local_node` row is the tag record, as the root's NodeIdentity lifecycle requires.

### 4. NodePaths and how the CLI, hooks and daemon resolve a node

**Recommended.**
- **Rename `InstancePaths` → `NodePaths`** (`src/daemon/paths.rs`; mechanical in the ~26 files that use it).

  | Field | Path |
  |---|---|
  | `node_dir` | `<state_dir>/node` (0700, via `ensure_private_dir` under `ensure_owned_state_root`) |
  | `key_path` | `node/node.key` |
  | `node_json_path` | `node/node.json` |
  | `database_path` | `node/herdr-threads.db` (`DATABASE_FILE` changes) |
  | `lock_path` | `node/owner.lock` |
  | `descriptor_path` | `node/endpoint.json` |
  | `socket_path` | `node/daemon.sock` when it fits `SOCKET_PATH_LIMIT`, else the existing runtime-dir fallback `stable_socket_path(node_dir)` |

  - Settings move to `node/settings.json` (`daemon::settings::SETTINGS_FILE`, read from `node_dir`). Logs move to `node/logs/` and `node/daemon.log`. The harness manifest cache moves to `manifest::cache_dir(node_dir)`.
  - `NodePaths::resolve(state_dir)` takes no host input. `locator`, `locator_path` and `namespace_path` are removed.
  - `legacy_boot_socket_path` and `relocated_endpoint_matches` are removed for the node dir: no older build ever bound a socket for `node/`. `accepts_descriptor_endpoint` becomes `endpoint == socket_path`.
  - New `pub fn legacy_instances_dir(state_dir) -> PathBuf` (`<state>/instances`), used only by doctor and purge.
  - `holds_store(state_dir)` is true when `node/herdr-threads.db` exists **or** any legacy `instances/*/threads.sqlite3` exists. `cli::instance::choose_state_dir` keeps choosing the directory that already has user data, so the node lands next to the legacy data it replaces.
- **`RuntimeContext`** becomes `{ state_dir, host_endpoint: Option<PathBuf>, herdr_bin }`.
  - `explicit` canonicalizes the state dir as today and the host only when it is present.
  - New `fn require_host(&self) -> io::Result<&Path>` returns today's "host endpoint missing: use --host-endpoint or HERDR_SOCKET_PATH" error. Every consumer that needs a pane or Herdr server (hook claim, `me`, `launch`, handoff, `panes`, setup) calls it instead of reading the field.
  - `cli::instance::resolve_context` resolves the state dir exactly as today (flag, `HERDR_PLUGIN_STATE_DIR`, default) and the host as today but optionally. Its source JSON reports `"host_endpoint": "none"` when no host resolves.
- **Daemon argv.**
  - `daemon run --state-dir <dir> [--host-endpoint <path>]`: `main.rs` `detached_child` accepts 5 or 7 arguments in that order and nothing else.
  - `lifecycle::spawn_detached` passes `--host-endpoint` only when the starting client resolved a claim host. It is an *attach hint*, never part of the daemon's address: the socket, lock and descriptor depend only on the state dir.
- **Hooks.**
  - `cli::hook::parse_hook_argv` keeps accepting `--state-dir` and `--host-endpoint` in old installed argv. The state dir selects the node, and the host endpoint becomes the claim host.
  - `setup`'s `installed_argv` is unchanged in this subepic: it already writes `--host-endpoint` only when it was explicit.
- **Descriptor.**
  - `EndpointDescriptor.instance_uuid: Uuid` becomes `node_uuid: NodeUuid`.
  - A new field `primary_host: Option<PathBuf>` holds the attached host's canonical endpoint.
  - `ownership.rs` checks the descriptor's `node_uuid` against `node.json` where it checked the `namespace` file before. `OwnerLock` no longer creates `namespace`.
- **Interim single-host guard (removed by ht-5fr.2).**
  - A CLI call that has a claim host and finds a daemon whose `primary_host` is a different endpoint fails client-side with `ErrorCode::HostUnavailable`. Detail: `node daemon serves Herdr server <a>; <b> is not attached` (one line; the remedy names `herdr-threads daemon stop`).
  - A store-only daemon (`primary_host: None`) reached by a call that needs a host fails the same way.

  This prevents a pane on server B from being matched against server A's targets before claims carry a host locator.

**Considered.**
- Keeping `RuntimeContext.host_endpoint: PathBuf` with a dummy value for store-only: it hides the absence and invites misuse.
- A separate `NodeContext` type plus a CLI `CallerContext`: cleaner, but it churns ~40 files that ht-5fr.2 rewrites anyway.
- Dropping `--host-endpoint` from `daemon run` entirely: the daemon would then have no way to attach a host before ht-5fr.2's lazy attach, and the single-host behaviour would regress in between.
- Keeping the per-boot and relocated socket acceptance: dead code for a directory that no old build used.

### 5. Clean-start schema baseline

**Recommended.**
- **File and versioning.**
  - The new store file is `node/herdr-threads.db`.
  - Schema: `migrations/node/0001_node_baseline.sql`, included by `src/store/schema.rs` as `NODE_BASELINE`.
  - `PRAGMA application_id = 0x48544E44` ('HTND'), `user_version = 1`, `schema_identity.marker = 'herdr-threads-node-v1'`.
- **`schema::initialize(conn, now, node: &NodeSeed)`**, where `NodeSeed { uuid: NodeUuid, tag: NodeTag, tag_k: 0 }`:
  - an empty database → in one `BEGIN IMMEDIATE`: apply the baseline, set the pragmas, insert the `local_node` row (`id`, `created_at`, `tag`, `tag_k = 0`) and the self row of `node_directory`;
  - `user_version = 1` with the right `application_id` → `verify_baseline`;
  - anything else (nonzero `user_version`, wrong `application_id`, unversioned nonempty) → `IncompatibleSchema`.
- **`verify_baseline(conn)`** applies `NODE_BASELINE` once to a `:memory:` connection, caches the normalized `(type, name, tbl_name, sql)` set from `sqlite_master` in a `OnceLock`, and compares. Any difference → `IncompatibleSchema` with the first differing object named. This replaces every hand-written `verify_existing_v*` audit. It also catches a development store built from an earlier draft of the baseline: the remedy is to move the file away.
- **Editable until released.** Until the first `v*` release that ships the node store, later subepics (ht-5fr.2..6) change the schema by editing `0001_node_baseline.sql` in place, with no new migration files. After that release, changes are additive `migrations/node/000N_*.sql` with `user_version` bumps and per-step verification. This rule is written as a header comment in the baseline file.
- **How the baseline is produced (one task).**
  1. Run today's `initialize` on an empty database.
  2. Dump `sqlite_master` in `rowid` order (tables, indexes, triggers, views) plus `INSERT` statements for every row present after initialization (the `schema_identity` marker and the backfill singletons, `completed = 1`).
  3. Apply the node delta below by hand.

  A transient test in that task's commit asserts that the dump minus the delta equals the v46 schema object for object. It is removed together with the chain in the next task.
- **Delete the legacy chain.** Remove `migrations/0001..0046_*.sql`, every `migrate_v*`/`verify_existing_v*`/`verify_v*` function and `LATEST_VERSION` from `schema.rs`, and the upgrade-path tests. These are `tests/store/schema.rs` upgrade cases, `lazy26_prerequisite.rs`, `topology_handoff_schema.rs` upgrade cases and the `user_version` fixtures in the store, integration and service suites listed by `grep -l "user_version\|migrate_v\|migrations/00" tests src`. Tests that only used the chain to build a fresh store switch to `initialize`. Rows in `scripts/harness-neutral-allowlist.txt` that name deleted migration files are removed.
- **Node delta applied to the v46 shape:**
  - **`host_instances` → `local_node`** (table renamed; every `REFERENCES host_instances(id)` becomes `REFERENCES local_node(id)`; every Rust query is renamed mechanically). Columns:
    - `id TEXT PRIMARY KEY CHECK(length(id)=32 AND id NOT GLOB '*[^0-9a-f]*')` (the `node_uuid`);
    - `singleton INTEGER NOT NULL DEFAULT 1 UNIQUE CHECK(singleton=1)`;
    - `created_at`;
    - `tag TEXT NOT NULL CHECK(length(tag)=3 AND tag NOT GLOB '*[^0-9A-Za-z]*')`;
    - `tag_k INTEGER NOT NULL DEFAULT 0 CHECK(tag_k>=0)`;
    - the node-scope columns `decision_seq`, `send_eligibility_revision`, `duration_config_revision`.
  - **`herdr_hosts`** (new). Columns:
    - `host_id INTEGER PRIMARY KEY AUTOINCREMENT` (node-local, never replicated, never reused);
    - `node_id TEXT NOT NULL REFERENCES local_node(id)`;
    - `endpoint TEXT NOT NULL UNIQUE CHECK(length(CAST(endpoint AS BLOB)) BETWEEN 1 AND 4096)` (the canonical path, as `RuntimeContext::explicit` canonicalizes it);
    - `created_at`;
    - `last_attached_at INTEGER`;
    - every Herdr-scope column moved from `host_instances`: `host_boot`, `host_epoch`, `observation_sequence`, `recovery_boot`, `recovery_epoch`, `lifecycle_revision`, `active_snapshot_id`, `recovery_baseline_generation_id`, `baseline_hold_unclaimed`, `invalidation_revision`, `observation_admission_sequence`, `observation_decided_sequence`, `reconciled_boot`, `reconciled_epoch`, with their CHECKs unchanged.

    The triggers `host_active_snapshot_published`, `host_baseline_snapshot_published` and `archival_host_invalidation` move to `herdr_hosts` and keep their bodies. Reachability stays in memory (`HostReachability`), with no column.
  - **`instance_id` columns** (~100, in node-scope and Herdr-scope tables alike) keep their name and their FK to `local_node(id)`. Their value is the local `node_uuid`, a constant per store. Renaming them would churn every query for no behaviour change.
  - **Herdr-scope child tables** (`snapshot_generations`, `observed_targets`, `recovery_baseline_releases`, `recovery_baseline_targets`, `recovery_holds`, `allocation_decisions`, `archival_instances`) stay keyed by `instance_id` in this subepic. With one primary host they are equivalent. ht-5fr.2 re-keys them to `host_id` by editing the baseline when it makes observation per host. That boundary is recorded in the baseline header.
  - **`node_directory`** (new): `node_uuid TEXT PRIMARY KEY` (hex32 CHECK), `tag` (base62-3 CHECK), `tag_k INTEGER NOT NULL CHECK(tag_k>=0)`, `host_label TEXT CHECK(length(CAST(host_label AS BLOB))<=255)`, `first_seen_at INTEGER NOT NULL`, `tag_hlc INTEGER` (the HLC of the fact that set the tag; NULL for the self row at init). Index `node_directory_tag ON node_directory(tag)`. This subepic writes only the self row. ht-5fr.3/.4 genesis and retag handlers upsert the rest.
  - **seats** (§6).
- **Store wiring.**
  - `SqliteStore::new(context, instance, …)` receives `identity.uuid().to_string()` as `instance`.
  - The lazy `INSERT INTO host_instances … ON CONFLICT DO NOTHING` in `store/seats.rs` (~L118) is removed, because `initialize` creates the row.
  - Queries that read or write the moved Herdr-scope columns are redirected to `herdr_hosts WHERE host_id = ?primary`. That covers the observation and admission sequences in `seats.rs` ~L118-130 and the snapshot upsert ~L2486, plus `materialization.rs`, `archival.rs`, `bindings.rs`, `effective.rs`, `wake.rs` and `queries.rs`, as `grep -n "host_instances" src` lists them per column. `?primary` comes from a new `StoreContext.primary_host: Option<i64>` field, set once after attach (§8). A host-scope write with `None` returns `ErrorCode::HostUnavailable`.
  - The snapshot upsert's `INSERT … ON CONFLICT(id)` becomes an `UPDATE herdr_hosts … WHERE host_id=?`, because the row exists from the moment of attach.
- **Daemon start order** (`app.rs` `run_elected_impl`, after election):
  1. `NodeIdentity::load_or_create(paths, database_path.exists())`;
  2. open the store with `NodeSeed{uuid, tag_for(uuid,0), 0}`;
  3. read `local_node.(id, tag_k)`, refuse on `id` mismatch (`IdentityMismatch`), `identity.set_tag_k(tag_k)`;
  4. regenerate `node.json`;
  5. attach the primary host (§8).

  Any of these failing is a start failure that goes through the existing `startup_failure_text` remedy path, with the `NodeKeyError` variant named.

**Considered.**
- Keeping the 46-step chain plus a v47: it carries ~2,500 lines of upgrade code for stores the node never opens.
- A single combined migration that also re-keys every Herdr-scope child table now: it touches `seats.rs`/`archival.rs` observation code that ht-5fr.2 rewrites for per-host sessions anyway, so the work would be done twice.
- Renaming all `instance_id` columns to `node_id`: churn with no behaviour.
- A checked-in golden schema dump: it duplicates the baseline file, which is itself the reviewed artifact, while `verify_baseline` catches drift at open.
- Keeping the `host_instances` name: misleading once the Herdr-scope columns have moved out.

### 6. Seats: home node, host and remote state

**Recommended.** The baseline's `seats` table changes as follows:
- `state` CHECK becomes `IN ('resolved','unresolved','retired','remote')`.
- New `home_node TEXT NOT NULL CHECK(length(home_node)=32 AND home_node NOT GLOB '*[^0-9a-f]*')`.
- New `is_local INTEGER NOT NULL CHECK(is_local IN (0,1))`.
- New `remote_kind TEXT CHECK(remote_kind IN ('agent','human'))` with `CHECK((is_local=0) = (remote_kind IS NOT NULL))`. A remote seat's agent/human role comes from `seat.created`. `role` keeps its local meaning and is `'native'` on remote rows.
- New `host_id INTEGER REFERENCES herdr_hosts(host_id)` with `CHECK(target_id IS NULL OR host_id IS NOT NULL)`.
- `CHECK(is_local=1 OR (host_id IS NULL AND target_id IS NULL AND structural_terminal_id IS NULL AND state IN ('remote','retired')))` and `CHECK(state<>'remote' OR is_local=0)`. The structural columns are already all-or-nothing, so one NULL test covers them.
- `seats_live_target` becomes `UNIQUE INDEX ON seats(host_id, target_id) WHERE state='resolved' AND target_id IS NOT NULL`.
- New index `seats_home_node ON seats(home_node, state)`.
- Triggers `seats_home_node_insert` and `seats_home_node_update` (BEFORE INSERT and BEFORE UPDATE OF `home_node`, `is_local` and `state`) `RAISE(ABORT, 'seat home mismatch')` unless `NEW.is_local = (NEW.home_node = (SELECT id FROM local_node))`. Two consequences: `is_local` can never disagree with `home_node`, and a seat's home never changes (UPDATE of `home_node` to a different value aborts). This enforces TRUST-POLICY's "never move a seat" at the row level.

The local-seat predicate is `is_local = 1`. It is published once as `pub(crate) const LOCAL_SEAT: &str = "is_local=1"` in `src/store/seats.rs`, plus `fn is_local_seat(conn, &SeatId) -> Result<bool, ApiError>`. Callers never compare `home_node` inline (root Key abstractions). Both seat inserts in this subepic (`seats.rs` ~L2609 allocation and ~L3406 rebind/fresh) write `home_node = instance`, `is_local = 1` and `host_id = primary`. No code path creates remote rows yet: ht-5fr.4's `seat.created` handler does.

**Considered.**
- Deriving locality from `home_node` with a subquery in every predicate: it cannot be indexed and is easy to get wrong.
- A generated column: SQLite generated columns cannot run subqueries.
- `home_node` NULL for local seats: this contradicts the root's "equal to self for local seats", and replicated payloads would need a special case.

### 7. Public IDs with tags and the direct-thread ID

**Recommended.**
- `protocol::ids`:
  - `pub const NODE_TAG_LEN: usize = 3`.
  - `generate_public_id(prefix, tag: &NodeTag) -> String` = `prefix + tag + public_id_suffix()`.
  - `is_short_public_id(prefix, value)` is renamed `is_public_id`: true when `value` strips `prefix` and the remainder is exactly 11 base62 characters. Its sibling `public_id_parts(prefix, value) -> Option<(&str /*tag*/, &str /*suffix*/)>`.
  - The prefixes `n`/`np` stay unambiguous by length: an `n` ID is 12 characters, an `np` ID 13.
  - `derived_message_id` keeps the whole remainder (tag plus suffix), so `p<tag><8>` ↔ `m<tag><8>` and `np<tag><8>` ↔ `n<tag><8>`. The legacy long-prefix branches (`prep-`, `msg-`, `notify-`) are deleted: a clean store has none.
  - `ThreadId`/`SeatId` parsing stays opaque (it already accepts `d…`). The legacy-UUID test is removed.
- `store::public_ids::fresh(conn, prefix, slots)` keeps its signature. It reads the tag with `SELECT tag FROM local_node` in the caller's transaction, so all call sites stay unchanged (`control.rs`, `seats.rs`, `messages.rs`, `service_send.rs`, `service_controls.rs`, `service_events.rs`, `schema.rs`). A retag commits before any later mint and is seen at once. `fresh_with` tests keep their injected generator.
- `ports::short_seat` uses the last 8 characters of a public seat ID (the random suffix), so Herdr agent names stay `s<8>` and do not grow by the tag. `launch_agent_names` uses `is_public_id`.
- **Direct-thread ID**, in `protocol::ids`: `pub fn direct_thread_id(a: &SeatId, b: &SeatId) -> Result<ThreadId, DirectIdError>`.
  - `DirectIdError::SameSeat` when `a == b`.
  - `(lo, hi)` = byte order of `as_str()` (Rust `str` `Ord`, which matches SQLite BINARY and today's `direct_threads CHECK(seat_a < seat_b)`).
  - input = `b"herdr-threads/direct/v1"` ‖ `u32 BE len(lo)` ‖ `lo` ‖ `u32 BE len(hi)` ‖ `hi`.
  - id = `"d"` + `base62_digits(u128::from_be_bytes(sha256(input)[0..16]), 10)`.
  - `pub const DIRECT_THREAD_PREFIX: &str = "d"` joins `prefix`. `pub fn is_direct_thread_id(&str) -> bool` checks `d` plus exactly 10 base62 characters.

  The function is pure and symmetric. It never hashes node IDs (root §2). This subepic wires it nowhere: `resolve_direct_pair` and the `thread.direct` handler adopt it in ht-5fr.4.
- Model-visible size: seat, thread, invitation and message IDs grow from 9 to 12 characters (the root accepted this). Direct-thread IDs are 11. No other agent-facing text changes. Golden CLI surfaces (`tests/cli/golden/`, `tests/golden_surfaces.rs`) that pin ID lengths or shapes are regenerated in the same task.

**Considered.**
- Passing the tag explicitly into `fresh` at ~20 call sites: churn, and a stale tag could be passed in from outside the transaction.
- Keeping `short_seat` on the first 8 characters after the prefix: it would mix the tag into agent names and shorten their entropy.
- Keeping the legacy-prefix derivation: dead code once the store is clean.

### 8. Store-only daemon and the primary host

**Recommended.**
- `app::run_elected_guarded` and `run_elected_impl` take `host: HostChoice` in place of `host: Arc<dyn HostPort>, observer: Arc<dyn BootstrapObserver>`:
  - `HostChoice::Hint(PathBuf)`: from `--host-endpoint`.
  - `HostChoice::Remembered`: no flag.
  - `HostChoice::Given(Arc<dyn HostPort>, Arc<dyn BootstrapObserver>, PathBuf)`: tests with `FakeHost`.
- `HostChoice` comes with `factory: Arc<dyn Fn(&Path) -> (Arc<dyn HostPort>, Arc<dyn BootstrapObserver>) + Send + Sync>`. `main.rs` passes a `NativeCli` factory.
- After the store opens:
  - `Hint` upserts `herdr_hosts(endpoint)` and attaches it.
  - `Remembered` attaches the row with the greatest `last_attached_at` whose endpoint is an existing socket (`FileTypeExt::is_socket`), if any.
  - Attaching sets `last_attached_at`, `StoreContext.primary_host` and the descriptor's `primary_host`.
- With no attached host the daemon is **store-only**:
  - it binds, publishes its descriptor and serves `health`/`stop`/`doctor` probes;
  - the host-bound workers (observation, wake dispatcher, archival sampler, handoff and bootstrap observer, the `HostGuardedStart` path) are not spawned;
  - a request that needs a host returns `HostUnavailable` ("no Herdr server attached");
  - health reports `hosts: 0` and a `store-only` daemon state label in `doctor` (`daemon_state_label`).

  Nothing in this subepic makes the daemon exit because no host exists. The idle-exit rule (no clients, no host, no peer links, no dial targets) belongs to ht-5fr.5. Until then a store-only daemon keeps today's lifetime (explicit stop).
- `host_id` values are never shown to agents.

**Considered.**
- Starting the host workers against a stub `HostPort` that fails every call: the workers would spin on errors and report a degraded health that is really an expected state.
- Resolving the host before election in `main.rs`: the remembered host lives in the store, which opens only after election.

### 9. Legacy `instances/`: doctor report and `legacy purge`

**Recommended.**
- **Doctor.** `doctor` (`src/cli/doctor.rs`, `collect_local`) adds a `legacy` object when `<state>/instances` exists: `{path, instances: <dir count>, bytes: <recursive size>, settings: [<instance settings.json paths>], live: [<pid>…], remedy: "herdr-threads legacy purge"}`.
  - `live` lists instance dirs whose `owner.lock` is currently held. It is probed with a non-blocking `flock` try that is released at once, the only access doctor makes. Doctor never opens a legacy database.
  - Text rendering is one line: `legacy data: <path> (<n> instances, <size>) not used; remove: herdr-threads legacy purge`, plus `legacy daemon running: pid <p>` per live owner, plus `legacy settings not carried over: <path>` per settings file.
  - The doctor exit status and `writes_safe` are unchanged (informational).
  - Unsafe permissions on `instances/` are no longer an `unsafe_private_dir` finding. `unsafe_private_dir` checks `node/` only.
- **`herdr-threads legacy purge [--confirm]`** is a new CLI subcommand, local only, with no daemon call:
  - Without `--confirm` it prints what it would remove (the doctor line) and exits 0.
  - With `--confirm` it refuses, with distinct one-line errors and exit 2, when:
    - agent evidence is present (the existing `agent_evidence` best-effort guard, TRUST-POLICY A4; the refusal names `--operator`, which overrides it the same way `me init --operator` does);
    - `instances` is a symlink, not a directory, or not owned by the euid;
    - any legacy `owner.lock` is held (the error names the pid from that instance's `endpoint.json`, if readable).
  - Otherwise it calls `std::fs::remove_dir_all(<state>/instances)` and prints `purged <path> (<n> instances, <size>)`.
  - It never touches `node/` or anything else in the state dir. Mutating no store, it records no store provenance. The audit label `operator:local-user:<uid>` appears in its output line.

**Considered.**
- Auto-removing after a grace period: destructive, as the root already rejected.
- Purging through the daemon: the daemon must never open `instances/`.
- Copying legacy `settings.json` into `node/`: reading `instances/` violates the root's "never opened"; doctor lists those files instead.

## Testing

- **`src/node/identity.rs` unit tests:**
  - create then reload gives the same uuid;
  - the key file is mode 0600 and exactly 32 bytes;
  - refusals for 0644 (`Permissions`), length 31 or 33 (`Malformed`), a directory or symlink at `node.key` (`NotRegularFile`), and a missing key with an existing store (`KeyMissingForStore`);
  - a race where a pre-placed `node.key` appears between temp write and link keeps the existing key;
  - a fixed-seed golden `node_uuid`;
  - `sign` output verifies with `ed25519_dalek::VerifyingKey`.
- **`src/node/tag.rs` unit tests:**
  - golden `tag_for` vectors for k=0 and 1 on three fixed uuids;
  - every tag is 3 base62 characters;
  - `on_collision` returns `Keep` for the lower uuid, for an equal uuid and for a different tag, and `MoveTo{k+1}` only for the higher uuid.
- **`src/node/json.rs` unit tests:** regenerated when missing, corrupt or with a stale tag; never authoritative (a reader of a corrupt file gets `None`).
- **`src/daemon/paths.rs` unit tests:**
  - `NodePaths::resolve` is independent of `HERDR_SOCKET_PATH` and `--host-endpoint`;
  - socket fallback for a long state dir;
  - `holds_store` for node-only, legacy-only and empty dirs;
  - `require_host` error text.
- **`src/protocol/ids.rs` unit tests:**
  - `generate_public_id` shape `<prefix><tag><8>` for every prefix;
  - `n`/`np` disambiguation;
  - preparation ↔ message derivation with a tag;
  - `direct_thread_id` golden vector, symmetry, `SameSeat`, 11-character shape, and different pairs giving different IDs;
  - `short_seat` uses the suffix.
- **`src/store/public_ids.rs`:** `fresh` reads the tag from `local_node`, and after an `UPDATE local_node SET tag=…` the next mint carries the new tag.
- **`tests/store/schema.rs`** (rewritten; a module of `tests/combined.rs`):
  - a fresh `initialize` sets `application_id`/`user_version` and inserts the `local_node` and self `node_directory` rows;
  - a second open verifies;
  - drift (an extra index, or an altered table) → `IncompatibleSchema` naming the object;
  - a wrong `application_id` and a nonzero unknown version are refused;
  - a second `local_node` row aborts;
  - seat triggers: a remote seat with a local home aborts, a resolved seat with a foreign home aborts, a `home_node` update aborts, and a valid remote row (`state='remote'`, `is_local=0`, `remote_kind='agent'`, host and target NULL) inserts;
  - `seats_live_target` allows the same `target_id` on two `host_id`s and refuses a duplicate on one.
- **Transient baseline-equivalence test** (squash task only): the baseline minus the node delta equals the v46 chain's objects.
- **`tests/daemon/node_layout.rs`** (module of `tests/integration.rs`, test-support, daemons spawned via `test_support::spawn` and stopped at the end):
  - `daemon run --state-dir` alone starts a store-only daemon (health `hosts: 0`), creates `node/` 0700 with `node.key` 0600, `node.json` and `herdr-threads.db`, and stays up;
  - a pre-seeded `instances/` tree is byte-for-byte unchanged afterwards (no file in it opened for write; the mtimes compared);
  - two CLI calls with different `HERDR_SOCKET_PATH` reach the same descriptor;
  - with a `FakeHost` hint, a second claim host gets the interim `HostUnavailable` line;
  - a restart keeps `node_uuid` and tag;
  - deleting `node.key` with the store present makes the restart fail with the `KeyMissingForStore` remedy.
- **`tests/cli/doctor_json.rs` / `doctor_labels.rs`:** the `legacy` object and text lines; a held legacy `owner.lock` shows as live; `store-only` label.
- **`tests/cli/legacy_purge.rs`** (new, module of `tests/combined.rs`; it uses scrubbed env):
  - a dry run lists and removes nothing;
  - `--confirm` removes only `instances/` and leaves `node/` intact;
  - refusals for agent evidence, a symlinked `instances`, and a held lock.
- **`tests/cli/hook.rs`:** old installed argv `--state-dir S --host-endpoint H hook claude` resolves the node at `S/node` and uses `H` as the claim host.
- **Checks:** `nice cargo clippy --locked --all-targets --all-features -- -D warnings`, `scripts/check-harness-neutral` (`src/node/` is core and names no harness), `nice scripts/check-default-features`, and focused `nice cargo test --locked --all-features` filters per task. No leaked processes (`scripts/check-no-leaked-processes`) after the daemon-layout tests.

Suggested task cut (each with its files):
1. `src/node/` identity, tag and json, plus the `Cargo.toml` deps.
2. `NodePaths`/`RuntimeContext`/descriptor/argv: `src/daemon/paths.rs`, `ownership.rs`, `lifecycle.rs`, `src/main.rs`, `src/cli/instance.rs`, `src/cli/hook.rs`, and the `InstancePaths` users.
3. The baseline squash: `migrations/node/`, `src/store/schema.rs`.
4. The `local_node`/`herdr_hosts`/seats delta and query redirection: the `src/store/*` files that `grep host_instances` lists.
5. Public IDs and the direct-thread ID: `src/protocol/ids.rs`, `src/store/public_ids.rs`, `src/ports.rs`, the goldens.
6. Store-only daemon and the primary host: `src/app.rs`, `src/daemon/health.rs`, `src/cli/doctor.rs` label.
7. Deleting the legacy chain and its tests: `migrations/00*`, `src/store/schema.rs`, `tests/store/*`.
8. Doctor legacy report and `legacy purge`: `src/cli/doctor.rs`, `src/cli/commands.rs`, `src/cli/mod.rs`, a new `src/cli/legacy.rs`.

Order: 1 → 2 → 3 → 4 → (5, 6, 8) → 7.

## Key abstractions

**NodeUuid / NodeTag**: new value structs (`src/node/`). These are the node's two public names, each with its canonical text form and its ordering.
- Owns: the 32-lowercase-hex form of `NodeUuid` (Display/FromStr/serde) and its byte `Ord`, which defines "higher uuid" everywhere. The 3-character base62 `NodeTag`. The pure `tag_for(uuid, k)` and `on_collision(...) -> RetagDecision`.
- Hides: SHA-256 over the public key and over `uuid ‖ k`, the digit emission order, and which side moves on a collision.
- Must not: be built from or printed as a hyphenated `uuid::Uuid`. Must not read the store or keep any state.
- Relationships: leaf types. `NodeIdentity`, the schema seed, `node_directory`, `protocol::ids` and the descriptor depend on them. They depend only on the shared `base62_digits` and `BASE62` in `protocol::ids`.
- Errors: `FromStr` rejects bad length or bad characters. `RetagDecision` is a closed enum (`Keep` / `MoveTo{k, tag}`).

**NodeJson**: new module (`src/node/json.rs`). It is the derived `node.json` hint file.
- Owns: the `format:1` shape, the regenerate-if-differs atomic write (0600), and a reader that returns `Option` (missing or corrupt means "identity unknown").
- Hides: the file format, and that the file is never authoritative.
- Must not: be read by the elected daemon as a source of uuid or tag. Only the store row (`local_node`) and the key hold those. Must not create the file anywhere except in the daemon after the store opens or after a retag applies.
- Relationships: the daemon start sequence and the retag apply handler (ht-5fr.4) write it. `ownership.rs` descriptor validation (it replaces `namespace`), `doctor` and later `peer --stdio` diagnostics read it.

**NodeDirectory**: new store module (`store::node_directory`). It is the only owner of the `node_directory` table.
- Owns: inserting the self row at `initialize`, the upserts by genesis and retag handlers (ht-5fr.3/.4), and `colliding(conn, tag)`.
- Hides: the table shape and the `tag` index.
- Must not: decide a retag. That stays in `on_collision`, and the apply handler acts on the result.
- Relationships: `schema::initialize` uses it for the self row. Fact handlers use it later.

**HostChoice**: new closed enum (`Hint` / `Remembered` / `Given`) plus a factory closure. It describes how the elected daemon gets its zero or one Herdr server.
- Owns: the attach decision after the store opens. `Hint` upserts `herdr_hosts` and attaches. `Remembered` picks the most recent row whose endpoint is a live socket. `Given` is a test `FakeHost`. Attaching stamps `last_attached_at`.
- Hides: `NativeCli` construction (the closure from `main.rs`) and how a socket's liveness is checked.
- Must not: be resolved before election (the remembered host lives in the store). Must not appear in the daemon address (socket, lock and descriptor path come from the state dir only).
- Relationships: consumed once by `run_elected_impl`. It produces an `Option` of the attached host (see the HostSession entry). The factory closure is the seed of HostSessions' lazy attach in ht-5fr.2. It is a closure, not a trait: there is one production implementation (`NativeCli`), and tests use `Given`.
- Lifecycle: consumed at start. Nothing keeps it afterwards.

**LegacyInventory**: new module (`src/cli/legacy.rs`). It is a read-only census of `<state>/instances`, shared by `doctor` and `legacy purge`.
- Owns: the path, instance count, recursive byte size, settings paths and live owners. A live owner is found by a non-blocking `flock` try that is released at once, with the pid taken from that instance's `endpoint.json`. It also owns the one-line text and the purge preconditions (symlink or not-a-directory, owner, held lock, agent evidence and `--operator`).
- Hides: every access to `instances/`. It is the only code that touches the directory, and the only removal goes through `remove_dir_all` in purge.
- Must not: open, migrate or copy any legacy database or settings file. Must not touch `node/`. Must not be reachable from the daemon.
- Relationships: `doctor` (`collect_local`) renders it. `legacy purge` checks it, then removes. `paths::legacy_instances_dir` is its only path source.
- Errors: purge refusals are distinct (agent evidence, unsafe directory, held lock with pid), each one line with exit 2. The dry run and the doctor report never fail on legacy contents.

**NodeIdentity**: extended. It gains the concrete key custody: `load_or_create(paths, store_exists)` with the no-clobber `link()`, `NodeKeyError` (a closed enum: `KeyMissingForStore`, `Unreadable`, `Permissions`, `NotRegularFile`, `Malformed`, `IdentityMismatch`, each naming its path and kept separate from store errors), and an `AtomicU32` tag cache that changes only through `set_tag_k`. That call happens at start from `local_node.tag_k` and from the `node.retag` handler. The `local_node` row is the tag record. Per spec §7, minting reads the tag from the row inside the transaction and never from this cache, so PublicIds does not use `NodeIdentity` for the tag. This narrows the ancestor's "PublicIds use it (tag)". `NodeIdentity` is held as `Arc` and created in `run_elected_impl`. It is never a global static and has no trait (one implementation).

**InstancePaths → NodePaths**: extended. It resolves from the state dir only (`node/`, `node.key`, `node.json`, `herdr-threads.db`, socket fallback). It gains `legacy_instances_dir`, and `holds_store` also counts legacy data. `locator`, `namespace` and the per-boot and relocated socket acceptance are gone.

**RuntimeContext / EndpointDescriptor**: extended. `host_endpoint` becomes `Option` (the claim host), and `require_host()` is the one place that produces the "host endpoint missing" error. The descriptor carries `node_uuid: NodeUuid` and `primary_host: Option<PathBuf>`. The interim single-host guard compares the claim host with `primary_host` (different, or `None`, gives `HostUnavailable`). It is one client-side function, so ht-5fr.2 deletes it in one place.

**HostSession**: extended (introduced early in single-host form). The attached primary host is one value holding `host_id`, the canonical endpoint, `Arc<dyn HostPort>` and the bootstrap observer. The daemon keeps it as `Option`. The host-bound workers (observation, wake dispatcher, archival sampler, handoff and bootstrap observer, `HostGuardedStart`) are spawned only from `Some`. `None` is the store-only daemon (health `hosts: 0`, doctor label `store-only`). ht-5fr.2 turns the `Option` into HostSessions without changing what one session owns.

**StoreContext**: extended. A set-once `primary_host` (`OnceLock<i64>`) and a single accessor that returns the id or `HostUnavailable`. Every query that moved to `herdr_hosts WHERE host_id=?` goes through that accessor and never reads the field directly. That makes it the one seam ht-5fr.2 replaces with per-seat `host_id` routing.

**Schema (`store::schema`)**: extended. `initialize(conn, now, &NodeSeed)` either creates the baseline, the `local_node` row and the self `node_directory` row in one `BEGIN IMMEDIATE`, or runs `verify_baseline`. `verify_baseline` compares against an in-memory copy of `NODE_BASELINE` cached in a `OnceLock`. `IncompatibleSchema` names the first object that differs, or reports a wrong `application_id` or version. All the `migrate_v*` and `verify_*` functions are deleted. Ordering hazard: the `local_node` row must be inserted before anything in `initialize` mints an ID, because `fresh` now reads the tag from that row.

**`host_instances` → `local_node` + `herdr_hosts`**: extended, as the ancestor describes. Concretely, the node row gains `tag`/`tag_k`, `herdr_hosts` gains `endpoint` and `last_attached_at` plus the moved columns and triggers, and the snapshot upsert becomes an `UPDATE` keyed by `host_id`.

**seats**: extended. It gains `home_node`, `is_local`, `remote_kind`, `host_id` and state `remote`, enforced by triggers (home never changes). There is one published predicate `LOCAL_SEAT` / `is_local_seat`, and both local insert paths write `home_node = self` and `host_id = primary` through the StoreContext accessor.

**PublicIds (`protocol::ids`, `store::public_ids`)**: extended. IDs are `<prefix><tag><8>`, checked with `is_public_id` / `public_id_parts`. The shared `base62_digits` lives here. `derived_message_id` carries the tag along. There is a pure `direct_thread_id` with `DirectIdError::SameSeat`. `fresh` keeps its signature and reads the tag in the caller's transaction. `short_seat` uses the random suffix.

Variation: very little varies in this subepic, so it adds no trait.
- How a daemon gets its host varies across three fixed cases, so it is the closed enum `HostChoice`. `NativeCli` construction is a closure, because only one production implementation exists.
- The number of attached hosts is 0 or 1 now and many in ht-5fr.2. `Option<HostSession>` plus the StoreContext accessor are the two seams that ht-5fr.2 widens.
- Key-start failures, retag outcomes, direct-ID errors, seat states and purge refusals are fixed sets, so each is a closed enum or CHECK, handled with `match`.
- Schema changes are edits to the one baseline file until release, and `verify_baseline` is the only check, so no per-version code exists.

Walk-through: an agent in a pane on Herdr server H runs `send`, and no daemon is running yet.
1. `resolve_context` resolves state dir S and claim host H.
2. `spawn_detached` runs `daemon run --state-dir S --host-endpoint H`.
3. After election, `NodePaths::resolve(S)` resolves the paths. `NodeIdentity::load_or_create(paths, false)` writes `node.key` with the no-clobber link.
4. `schema::initialize` gets `NodeSeed{uuid, tag_for(uuid,0), 0}` and creates the baseline, `local_node` and the self `node_directory` row.
5. The daemon reads `local_node.(id, tag_k)`, checks the id against the key, and calls `set_tag_k`. NodeJson regenerates `node.json`.
6. `HostChoice::Hint(H)` upserts and attaches `herdr_hosts`. `StoreContext.primary_host` is set, the `HostSession` is `Some`, and the host workers start. The descriptor publishes `node_uuid` and `primary_host = H`.
7. The client's interim guard sees that H matches and sends the request.
8. The seat insert writes `home_node = self`, `is_local = 1` and `host_id` from the accessor. `fresh` mints `s<tag><8>`, reading the tag from `local_node` in the same transaction.
9. Later, a pane on server H2 gets `HostUnavailable` from the guard on the client, and nothing reaches the store. A restart with the key deleted fails at step 3 with `KeyMissingForStore`. `instances/` is never touched, and doctor reports it through LegacyInventory.

Likely future change: ht-5fr.4 adds retag.
- One new FactKind module, `node.retag`, plus one registry entry.
- Its genesis handler upserts NodeDirectory, calls `colliding` and then `on_collision`. On `MoveTo` it authors `node.retag`.
- The retag apply handler updates `local_node.tag`/`tag_k`, then calls `NodeIdentity::set_tag_k` and NodeJson regenerate.
- `fresh`, NodePaths and the schema check do not change: the next mint sees the new row.

Digest:
- NodeUuid/NodeTag: owns the canonical hex and base62 forms, uuid ordering, `tag_for` and `on_collision`. A change to tag derivation or the retag rule goes in `src/node/tag.rs`. Callers never hash keys or compare uuid strings themselves.
- NodeJson: owns the `node.json` format, regenerate-if-differs, and the hint-only reader. New diagnostic fields go here. Callers never treat it as authority or write it outside the daemon start and retag paths.
- NodeDirectory: owns the `node_directory` rows and collision lookup. Peer-node directory writes go here. Callers never write the table directly or decide retags in it.
- HostChoice: owns the start-time choice (hint, remembered or given) and the attach stamp. A new way to choose the start host is a variant here. Callers never build a `HostPort` before election or put the host into the daemon address.
- LegacyInventory: owns every read of `instances/` and the purge preconditions. New legacy findings go here. Nothing else, and never the daemon, opens or removes `instances/`.

## Post-Implementation Notes

> *As this design is implemented and iterated on — bug fixes, adjustments, anything that diverged from the assumptions above — append a dated note here, whether or not a formal debugging skill was used.*
