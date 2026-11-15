# Initial queue control implementation

This is ht-d2jq.3 only. It adds an optional adapter control port; production wake
selection, the scheduler, persistent wake state, hook bodies and receipts are unchanged.
The contract is the observed-idle amendment and authorized first slice in
[the design](design.md), including the accepted cooperative loadedness limit.

## Scope and targeting

The neutral port in `src/harness/wake.rs` carries canonical binding, native session,
execution/generation, process fingerprint and kernel boot. Its context uses the
caller's clock, budget, process reader and owned adapter state directory. Other
adapters default to no control support; registration delegates without a harness
branch in core. Linux Codex exposes the control implementation, but nothing selects it.

Capture reads bounded proc metadata from the exact live process, its executable,
current native-session writer lock and persistent queue database. FDs are deduplicated
by device/inode. Deleted connections, multiple databases, unsupported observed layouts,
missing locks, unreadable or oversized metadata, changed boot/start and replaced files
refuse the observation. Descriptors retain only three paths and file identities, never
an argv/environment dump. The observed queue layout is capability evidence, not a
Codex release-number prerequisite. Other platforms retain native waking.

A lock supports observed loadedness; it is not a query of the receiver's thread
manager. Canonical SessionEnd normally disconnects before writer cleanup. The accepted
combination of missed SessionEnd and failed writer cleanup can retain a marker until
resume. No new prompt suppression or lifecycle enforcement system is introduced.

## Control and external effects

Add revalidates the live receiver before and after helper readiness. The helper uses
the captured executable, a private empty HOME/CODEX_HOME/XDG/cwd and explicit actual
SQLite directory; inherited environment, credentials, hooks, profiles and receiver
command-line overrides are not copied. Actual helper DB device/inode must match the
frozen receiver DB. A bounded queue/list(limit1) preflight then checks current stored-thread addressability
and expected response shape, without treating queue contents as arrival or receipt
evidence. Missing metadata, unsupported/malformed response or too little remaining
time refuses before authorization/add. A 100ms add-request refusal margin is not a
round-trip latency promise. After qualification, the caller's final callback rechecks canonical
scope/owedness and host idle/UI conditions immediately before add bytes. The callback
must honor the caller's budget. The adapter has no host/UI policy.

Add accepts correlation, never a body: it constructs only the existing 22-byte wake
marker. Its outcomes distinguish definitive no-effect refusal, a known accepted ID,
and uncertain acceptance. Any positive add write makes transport failure uncertain;
there is no internal retry. A known ID survives cleanup failure through
`Accepted { id, cleanup_error }`. Unknown/refused errors retain cleanup diagnostics.
No outcome is message delivery, cognition or receipt evidence.

Saved list/delete intentionally do not require the old receiver, boot or lock. They
validate the frozen descriptor's current executable/database identities and helper
same-DB observation; core must authorize the saved intent and exact owned IDs. List
returns matching correlation IDs only, with explicit incomplete pagination. Delete
reports only whether a pending row was removed, without promising no later turn.

## Resource behavior

The accepted refusal caps remain: capture 100ms/1024 FDs, argv 32KiB, environment
64KiB, paths 4KiB, descriptor 16KiB, IDs 128 bytes; JSON line 256KiB, total stdout 1MiB,
stderr 8KiB; list 32 rows/page and at most four pages. A helper has a two-second ceiling
inside its caller budget, with startup/request bounds and 250ms reserved for cleanup;
less than 500ms remaining refuses startup. One operation per exact process/session
scope runs at a time, without waiting or a background owner.

Cleanup sends SIGKILL to the exclusively owned process group, polls direct-child reap
and captured-pipe EOF within its reserved window, and reports failure to confirm them.
There is no blocking wait. Drop is the bounded panic backstop. These are explicit
observations, not a claim about arbitrary detached descendants. Later recovery must
persist/quarantine cleanup uncertainty before retry or fallback, as the design owner
ruled; this slice does not implement that persistence.

## Evidence and validation

[Source passages and no-auth qualification](control-source-inspection.json) preserve
queue connection/pool lifetime, home-independent metadata lookup and SessionEnd order.
The private-home check used an isolated metadata fixture and separate helper, matched
actual DB identity, added/listed/deleted one marker with helper loaded/list empty
before/after, made zero model-turn requests, copied no authentication and removed all
owned processes/scratch. It is not a real-receiver matrix or release qualification.

Deterministic tests exercise real bounded proc fixtures and a protocol subprocess,
including duplicate descriptors, ambiguous/deleted storage, private environment,
missing/malformed/slow metadata preflight, late authorization refusal, uncertain replies, partial/zero writes, pagination,
saved control after receiver exit, accepted-ID cleanup reporting and descendant stop.
A combined test reaches capture through real registry delegation and owned proc FDs.

Final command results and scoped cleanup are recorded in `control-validation.json`.
Tests run through the existing build toolbox (GCC), using the shared heavy-job wrapper
and the worktree's seeded target; the host itself lacks a C linker. Loaded-host build
wall times are recorded separately from test execution; they are not latency or
incremental-build-budget claims. No new routine model-visible text is introduced.
