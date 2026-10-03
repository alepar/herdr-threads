# Scoped Codex socket policy: implementation plan

**Goal:** A managed Codex launch may use a future binary after the exact executable and effective session policy have passed a bounded capability probe, while default-deny networking and daemon authority remain intact.

**Current review checkpoint:** 0.160.0 native exec captured the full measured private-policy result, and the interactive TUI's function-call output captured by the private mock provider returned the same result. The product still lacks a target-pane executable witness. The independent preparatory change in this branch adds an explicit `-C` parser and rejects caller config/profile/sandbox overrides when an owned scoped policy is present. It does not authorize a new launch by itself. See [report](REPORT.md).

## Task 1: target shell witness

Files: `src/host/native.rs`, `src/host/observation.rs`, `src/ports.rs`, `src/cli/launch.rs`, focused host/launch tests.

1. Add a bounded target-pane shell operation that asks the **same shell Herdr will use** to resolve `codex` to an absolute regular executable, read its version, and identify its file content. Do not substitute local `PATH` or the coordinator's cwd. Herdr 0.9.1 `pane run` may supply a diagnostic witness, but a separate `pane run` followed by canonical `agent start --kind codex` cannot pin the executable: prompt hooks, PATH changes or shell replacement can change resolution between calls. The required guarded Herdr facility is `agent.start` with an absolute executable and an expected content identity checked at submission, while retaining its existing empty-pane and agent lifecycle checks. Until that exists (or an equivalent atomic identity binding is proved), report `target executable unverified`.
2. Bind the witness to the target pane ID, terminal, host incarnation, generation and observation sequence. Recheck those values immediately before guarded start. A pane move, shell replacement, changed PATH or unreadable executable invalidates the witness.
3. Compare the target executable content digest and version with the binary passed to the probe. An optimistic hook admission never substitutes for this equality.
4. Add tests for same binary, different binary with same `--version`, pane move, stale generation and unanswering host. Use an isolated named Herdr test server for native checks.

## Task 2: exact cwd and policy

Files: `src/harness/launch.rs`, `src/cli/launch.rs`, `src/cli/setup.rs`, focused launch tests.

1. Require one explicit absolute `-C` in the initial slice; canonicalize it and reject missing, duplicate, relative or nonexistent paths. The parser in this branch is a preliminary guard, not the whole check.
2. Refuse caller `-c`, profile, enable/disable, sandbox and additional-directory options while Herdr's owned scoped policy is active. This prevents caller arguments placed after the owned overrides from replacing the measured policy. Preserve existing behavior when no scoped policy is active.
3. Build exact owned session overrides for workspace-write, `network_access=true`, proxy enabled, one allowed daemon Unix socket, no allowed domains, no broad Unix sockets/local bindings/upstream proxy, and the two canonical client journal writable roots. Never write these scoped keys into the global user config.
4. Reproduce the selected cwd, project config layers and managed requirements in the probe. If the effective inputs cannot be read or matched, report `policy validation inconclusive` and do not launch. Reject a real policy failure separately from an inconclusive probe.
5. Validate that the same override placement applies to interactive Codex and `exec` after Codex parses their subcommand arguments. `exec resume` remains under its existing captured-hook gate.

## Task 3: bounded capability probe

Files: new `src/cli/codex_sandbox_probe.rs` and a hidden first-party network/filesystem probe entry point, plus focused tests.

1. Use the witnessed absolute Codex binary and exact policy from Task 2. Create a private temporary home, two owned Unix listeners, an owned loopback listener, and isolated writable-root fixtures. Spawn `codex sandbox` without a model request under one deadline. Stop/reap all children and close/remove all owned resources on every exit path.
2. Require unsandboxed positive controls for the owned listeners. Require the allowed socket to connect in the sandbox, other owned Unix and loopback targets to return recognizable policy denial, and scoped writable roots to write while a sibling/daemon-state path is denied. Direct external TCP must produce a policy denial; online HTTP can supplement, never replace, that result.
3. Return typed `passed`, `failed` and `inconclusive` observations with measured executable digest, platform, cwd/config/requirements/policy digests, probe protocol version, and stages. Cache only a conclusive result for the identical identity. A real later `transport_denied` invalidates its use for that invocation. Never create a known-broken hook assertion from a sandbox probe.
4. Test newer unlisted pass, same version/different bytes, changed config, proxy ignored, false-negative network outage, missing Python/helper, timeout cleanup, known-broken hook refusal, and stale cache. The probe must run offline.

## Task 4: user-facing states and integration

Files: `src/cli/launch.rs`, doctor-redesign's `src/cli/doctor.rs`, cli-help-research's command/help files, `docs/install.md` and compatibility docs.

1. Make setup keep user-level/global allowance constrained. A scoped pass must not make `doctor fix` promise global repair.
2. Render the actual state: target executable witness missing, `-C` missing, policy probe failed/inconclusive, or true socket `transport_denied`. Unknown version alone is not an unsupported condition. Keep hook admission and known-broken evidence separate.
3. Run focused all-feature tests, clippy with `-D warnings`, fmt, default-feature check, and scoped leak checks. Request the coordinator's exclusive full-suite and squash-merge slot, then verify the landed main SHA. Do not run a full suite concurrently with other tabs.

## Transport follow-on

Prototype a read-only stdio MCP bridge against a private daemon only after the near-term socket path is stable. Measure whether Codex starts the server outside its command sandbox and whether its lifecycle/cleanup and explicit caller claims can preserve daemon A2 decisions. Loopback TCP and filesystem mailboxes remain research alternatives; neither is a drop-in launch bypass.

## Tracked remaining optimistic-launch scope

- Obtain the guarded absolute-executable Herdr start facility or prove an equivalent atomic target execution witness. Match the actual target executable to the probed digest before allowing a session-scoped policy.
- Bind the exact target cwd, `CODEX_HOME`, project/profile/requirements layers, socket path, journal roots and native argv to the capability evidence. The preparatory `-C` parser and override guard do not perform this binding.
- Add bounded positive and recognizable policy-denial controls, typed conclusive/inconclusive results and cache invalidation for changed identity/configuration; test native exec and TUI under the effective policy.
- Authorize a managed launch only for a matched conclusive result, preserving known-broken hook evidence and daemon authority. Keep global user-level setup on its independent measured policy gate until broader coverage exists.
