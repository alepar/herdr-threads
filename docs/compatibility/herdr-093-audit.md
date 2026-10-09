# Official Herdr 0.9.3 compatibility audit

Audited 2026-10-05 against Threads baseline `f04676cb` (0.2.7), on macOS arm64. The bounded change admits exact Herdr `0.9.1` and `0.9.3`, both with protocol `22`, through one predicate shared by transport ping and snapshot normalization. It preserves existing framing, budget, cancellation, endpoint witness, launch readiness and authority checks. It does not qualify arbitrary protocol-22 releases or harness versions.

> **Superseded policy (2026-10-08).** The exact-release predicate described here was replaced by a version floor: the adapter admits Herdr 0.9.1 or newer, ignores `protocol` for admission and warns in Health for releases newer than 0.9.3. This audit's evidence (Threads consumes none of 0.9.2's removed routes) is what makes 0.9.2 admissible. See [Herdr host compatibility](herdr-host.md).

## Official identities

- [0.9.1 source](https://github.com/herdrdev/herdr/tree/065ef9d6a531c49fb8bee7e818ef837065b21ee9): peeled tag commit `065ef9d6a531c49fb8bee7e818ef837065b21ee9`.
- [0.9.3 source](https://github.com/herdrdev/herdr/tree/7b116c05bfda646af39d2524c54e70c751f57ee8): peeled tag commit `7b116c05bfda646af39d2524c54e70c751f57ee8`; clean detached source checkout, no source edits or local build.
- [0.9.3 release](https://github.com/herdrdev/herdr/releases/tag/v0.9.3): published 2026-09-29; official `herdr-macos-aarch64`, 22,210,016 bytes, `--version` reports `herdr 0.9.3`. Downloaded SHA-256 `5173a3e0ae42d5d1ab7ebfa5d5e6329f7c3d23f8e1a3677c7ce3231da2884157`, matching the GitHub release API asset digest.
- Official generated machine schemas at `docs/next/api/herdr-api.schema.json`: both document version 1 / protocol 22. SHA-256: 0.9.1 `226d4ecbd128d2e6bc84e4c8ddcec21ba9c7e51a0aafffcf087111ead3f1fa9a`; 0.9.3 `9e2af207e9aa8183d4aeca5fde9cc48e7909bb40cdbd7cf21608a6d3ea78075b`.

Fetched source, release API records, schemas, API/recognition/lifecycle diffs and test evidence are retained under the main checkout's `target/coordinator/herdr-093/`. This directory is task evidence, not an installed server or modified upstream dependency checkout.

## Consumed machine contract

Compared official 0.9.1 and 0.9.3 schemas and source producers/dispatch, rather than inferring API compatibility from protocol equality.

| Threads route | Audited result |
| --- | --- |
| `ping`, `session.snapshot` | Required version/protocol and snapshot collection shapes unchanged; version becomes 0.9.3. |
| `pane.get`, `pane.current` | Target/terminal/workspace/tab/revision/status/focused/session fields retained; optional `restore_error` added to PaneInfo. |
| `pane.read`, `agent.read` | Params and consumed text/read response retained, including `detection` and `recent_unwrapped`. |
| `pane.send_text`, `pane.send_keys` | Params and ordinary response dispatch retained. |
| `agent.start`, `agent.get` | Start params unchanged; optional `completion_seq` added to AgentInfo, required fields unchanged. |
| `agent.prompt`, `agent.send_keys` | Params and consumed response shapes retained; blocked and readiness guards remain. |

Source: [machine schema](https://github.com/herdrdev/herdr/blob/v0.9.3/docs/next/api/herdr-api.schema.json), [API server](https://github.com/herdrdev/herdr/blob/v0.9.3/src/api/server.rs), [agent operations](https://github.com/herdrdev/herdr/blob/v0.9.3/src/app/agents.rs), [prompt dispatch](https://github.com/herdrdev/herdr/blob/v0.9.3/src/api/wait.rs), [snapshot producer](https://github.com/herdrdev/herdr/blob/v0.9.3/src/app/api/session.rs).

Ordinary nonstreaming requests still produce one LF-terminated response and close their connection; new SSH registration and event subscriptions have different lifetimes and are outside this adapter. `agent.start` still returns after submission with launch pending, before named readiness. Threads must keep its bounded readiness polling and preserve unknown outcomes after possible submission. The added completion sequence does not establish model receipt, check-in, ACK, execution identity or seat continuity.

## Material release changes and limits

[0.9.2 release notes](https://github.com/herdrdev/herdr/releases/tag/v0.9.2) describe removal of `pane.graphics.*`, self-reported agent resume/idle-shell release, terminal environment changes, completion handling and Codex state detection changes. The [0.9.3 hotfix](https://github.com/herdrdev/herdr/releases/tag/v0.9.3) repairs Escape/Alt input decoding. Threads consumes no removed graphics routes, emits no self-reported resume command and retains its own conservative composer and wake checks. New pane environment cleanup does not remove Threads' explicit launch arguments or independently captured lifecycle evidence.

The new idle-shell release in `src/terminal/state.rs:1879` applies to custom labels that `parse_agent_label` cannot identify. It clears a self-reported claim when that pane's own shell is observed idle, respecting newer report timing. It is not evidence for ending a Threads binding. Known Claude/Codex labels retain the existing process recognition and cooperative continuity limits. No TRUST-POLICY invariant or accepted limit is weakened by this release admission.

Official 0.9.3 has neither `AgentStartParams.process_hint` nor `ServerCapabilities.agent_start_process_hint_v1`. Its Python runtime parsing still refuses opaque `-c` / `-m` payloads; `start_agent` still constructs native executable plus original arguments without child environment injection. Therefore Task29's external hint dependency remains necessary for its measured launcher seam. Task30 owns the separate exact-true capability negotiation: missing/false/malformed capability must produce prestart `NotSubmitted`, no start frame and no fallback. This compatibility change does not implement or bypass it. A private patched candidate based on 065ef9d6 remains honestly modified **0.9.1**, with source/build/binary identity recorded separately; accepting its release metadata does not attest its modifications.

## Validation and independent review

The regression first failed on the old exact snapshot gate with `Unsupported` for 0.9.3. Focused socket tests then cover both audited releases through ping and snapshot, unsupported release/protocol refusals before any operation connection, unchanged unknown execution/incarnation claims, framing/cancellation and endpoint witness behavior. The ignored `isolated_herdr_093_host_contract` test selects an explicitly supplied official binary on PATH and uses `IsolatedHerdr` for a private HOME/config/socket/workspace and automatic cleanup. A local recognizable script substitutes for Codex; it has no provider, credentials, model or Threads hook. Its start/readiness/prompt observations qualify the host contract only.

Independent native reviewer `herdr-093-review` found no source/design blockers after reviewing both machine schemas, official source and the normative trust policy. Final implementation verdict: **Ready; no Blocking or Should-fix findings**. The test-only bounded second-accept amendment was also reviewed. The transcript is retained as `target/coordinator/herdr-093/review-transcript.txt`.

Final verification on this branch:

- `nice cargo test --locked --all-features --lib host::`: 76 passed, no failures (1.13 s test execution).
- `nice cargo test --locked --all-features --test combined host_adapter::`: 26 passed, no failures, two explicit opt-in tests ignored (0.51 s).
- With the official downloaded binary directory prepended to PATH, `nice cargo test --locked --all-features --test combined isolated_herdr_093_host_contract -- --ignored --nocapture`: one passed (7.88 s). Verified snapshot/pane identity, canonical start argv, named readiness, prompt delivery/read, blocked prompt refusal, shell return and private server stop. The fixture explicitly creates its workspace on the fresh headless host, renders the source-required startup prompt, and uses the pane ID for Threads prompt correlation. Earlier fixture failures were corrected without product behavior changes.
- `nice cargo clippy --locked --all-targets --all-features -- -D warnings`, `nice scripts/check-default-features`, `cargo fmt --check`, `git diff --check`: passed. Final incremental clippy took 1.05 s.
- Scoped `scripts/check-no-leaked-processes --run-id "$HT_LEAK_RUN_ID"`: no leaked test processes. The run ID and cleanup output are preserved alongside the private-host log. All owned private servers/descendants stopped.

No shared-server upgrade/restart, real-config mutation, upstream source modification, full branch suite, push or release was performed. Coordinator w4:p1 owns main integration and integrated checks; w4:pCC owns Task29/Task30; w4:pD8 owns the version-independent harness design.

## Existing adapter follow-up

The audit found a pre-existing `agent.send_keys` response-handling omission in Threads `src/host/transport.rs`: the method is mapped by NativeCli and used by `send_submit_key`, but excluded from the transport's accepted-result match. Rejection occurs after the host exchange, so Enter may already have been sent when Threads reports `InvalidRequest`; the notification verification caller records `NotChecked` and skips its final composer read. This exists at baseline as well as this branch and is unchanged by release admission. Independent review classified it as a nonblocking existing follow-up. A later fix should accept the official `ok` response and test `send_submit_key` delivery and structured-error handling. The private qualification here exercises `pane.send_keys`, not that defective adapter path. The upstream API-shape comparison above does not claim that path was runtime-qualified.
