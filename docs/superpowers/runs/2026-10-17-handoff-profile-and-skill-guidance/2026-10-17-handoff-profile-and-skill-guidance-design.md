# Handoff launch directories and concise collaboration guidance

## Goal

Agents can hand off to a fresh tab using an explicitly selected existing harness profile directory and working directory, with the same selected inputs preserved across retry. The shipped skill teaches readable names, timely priority-0 delivery, explicit receipt dependence and proactive recovery when requested receipts do not arrive.

## Problem description

Handoff already freezes instructions, native options, routing and a new tab's cwd, but has no first-class profile-directory input. Calling it from a Codex or Claude session can therefore select the caller's profile rather than the desired account/configuration. The skill documents delivery mechanics but leaves agents to infer when another worker needs an immediate wake and what to do when a receipt they depend on is missing.

## Main challenges

Profile selection must affect the launched child as well as hook inspection. Herdr's guarded `agent.start` accepts only native argv; `src/cli/launch.rs` deliberately rejects nonempty adapter environment overrides. The existing guarded tab-creation transport does accept cwd and env. Existing bootstrap identities require empty env and have exact retry/derived-child comparisons, so support must be additive without rewriting historical identities. Adapter-specific environment names must stay in adapters. Receipt recovery must preserve the distinction between receipt, adoption and completion, and must never become polling or repeated wake spam.

## Key decisions made

Expose `handoff --new-tab LABEL --profile-dir DIR [--cwd DIR]`. Keep cwd's existing new-tab behavior. Both explicit paths must be absolute existing directories; canonicalize once before effectful publication, reject missing/files/non-UTF-8/control-containing or transport-oversized paths, and recheck existence before each unpublished create/start boundary. New-tab profile transport uses the tab's declared child environment, with neutral adapter policy deciding the mapping. Existing-pane launches cannot safely change cwd/profile through the supported guarded host route: reject either option before durable/thread/native effects and point to new-tab. `--existing` delivery rejects both as launch-only. Plain `launch` is unchanged. Do not create profiles, install hooks, copy authentication, mutate caller env or write real user configuration.

## Decision points

### Public grammar and defaults

`--profile-dir PATH` means a directory, never a native named-profile argument. It is opt-in: omission retains today's environment/profile selection and serialization. `--cwd` remains an absolute existing directory defaulting to invocation cwd for new-tab. Supporting relative paths was considered but rejected to retain the existing cwd contract and make frozen paths unambiguous. Exposing arbitrary `--env` or changing HOME was rejected as unnecessary and overbroad. Parse launch-only conflicts before resolving recipients or acquiring publication effects; command help must state new-tab support and explain the guarded-start limitation for panes. Existing exact `--harness-binary` PATH identity guards remain intact.

### Adapter profile mapping

Extend the registered `LaunchPolicy` with a neutral default unsupported explicit-directory selector; the registry selects the adapter once. Codex maps the canonical directory to `CODEX_HOME`; Claude maps to `CLAUDE_CONFIG_DIR`. Hermes maps to `HERMES_HOME`, keeping its existing native named-profile resolution and source-bound prelaunch checks. An explicitly selected Hermes named profile may resolve below the supplied root; the retained effective setup scope must represent the actual selected profile, not confuse a root with a named profile. agy derives configuration/witnesses from HOME and currently has no independent profile-directory selector: reject explicit profile selection with a compact unsupported diagnostic, while new-tab cwd continues to work. Never manufacture an agy equivalent or change HOME. Any later supported agy mapping belongs solely in its adapter.

Explicit selection takes precedence over inherited caller or pane-shell profile hints for preflight and child tab env. Override only the adapter-declared variable in a captured `SetupEnvironment`; hook lookup, status, fingerprinting, startup composition and child launch must use this captured selection and the selected cwd. Do not re-probe an unrelated launcher shell and silently replace an explicit selection. Shell startup can still alter its declared environment: report this honestly as declared child input, not native consumption or authenticated-account proof. Preserve all current prelaunch and native executable rechecks. No version probe or exact-version gate is added.

### Frozen launch context and canonical compatibility

Introduce an optional, serde-defaulted and absent-omitted `LaunchDirectoryContext` in the saved new-tab bootstrap/derived launch inputs. It holds the canonical requested profile directory, the adapter-produced bounded environment projection, and selected working directory needed by preflight/retry; no credential contents. The existing cwd remains the tab creation cwd and must agree with context cwd. An absent context retains historical serialization, digests, empty-env validation and replay. A present context permits exactly its frozen env on the create request and reuses the same context in the exact derived child. Bound absolute paths and environment names/values, reject mismatched cwd/env/context, and preserve exact retained-report comparisons. The daemon judges the plan against its canonical identity; local selection is input, never seat authority.

Advertise and require additive daemon capability `handoff.launch_directories_v1` before publishing a context-bearing intent, creating a tab, resolving a recipient, or staging messages. Older daemons fail before effects. Legacy context-free operations continue to use their current capabilities. No migration or existing historical identity rewrite is intended; serialized optional fields and existing JSON retention suffice. If source investigation proves a new shape/version necessary, implement the smallest additive version while preserving old validators/replay and document it in a dated implementation note.

Retry consumes saved canonical directories/env/cwd and saved native argv, not current CLI defaults, caller profile variables, a renamed symlink, or newly configured options. Current existence/type is rechecked without re-resolving the original symlink; missing selected directories refuse before new native effects. Repair and retry remain available. Failed pre-start/new-tab creation retains existing certainty/unknown rules, and possible start never relaunches automatically. Runtime config fingerprints retain existing current-input revalidation: freezing selection does not freeze directory contents or bypass config-change guards.

### Guidance content and scope

In user-facing prose use known agent/thread/workspace names; use exact IDs for commands, disambiguation, receipts and evidence, respecting exact hook ready commands. Send priority 0 (`--nudge`) when the recipient may be actively waiting, blocked, or idle and must react now; this includes dependency-unblocking replies, urgent clarifications and task dispatch that needs immediate action. Routine nonblocking updates stay priority 1; default threshold and runtime wake policy do not change.

When progress depends on receipt, send `--require-ack` with an actionable request and requested response; explain once that ACK is receipt only. If a required ACK is missing/overdue, take useful action: inspect the recipient pane with the Herdr skill when available, perform the needed work, or hand off to a fresh session. Choose based on observed state; do not treat a missing ACK as proof of death, restart peers, resend blindly, repeatedly nudge, poll inbox, or use follow/sleep to wait. Waiting for a notified response means finishing the native turn. Automatic complete-display ACK behavior and top-level/subagent restrictions remain exact.

Edit `integrations/skill/SKILL.md` as source and regenerate `docs/agent-usage.md` and any other checked mirrors using the repository's established generator. Keep one compact guidance paragraph per topic and avoid repeating static framing per message. Record added/removed UTF-8 bytes for each model-visible artifact and check against the token-efficient communication rules; target total skill growth at most 1 KiB, including the profile example. Help/errors remain concise, bounded and do not print auth/config file contents.

### Verification and matrix

Use focused tests during implementation, with meaningful RED/GREEN for behavior. Contract and parser tests cover supported/new-tab, pane/existing refusal before effects, path validation, unset/default preservation, symlinks, spaces/quotes/control bytes and size bounds. Adapter tests cover Codex/Claude/Hermes mapping, explicit-over-inherited selection, Hermes named profile and agy unsupported behavior. Durable identity/transport tests prove matching env/cwd into actual CreateTabRequest and derived child, exact automatic and explicit-thread retry, changed caller env/cwd/symlink after freeze, removed directory refusal, lost replies and historical context-free identity bytes. Treat independent hooks/profile readiness as diagnostics under existing rules.

A dedicated Configuration smoke leaf invokes the public handoff entry point against owned helper executables and isolated Herdr: new-tab Codex and Claude receive distinct scratch profiles and cwd, Hermes mapping has a source-bound fixture (or supported isolated stand-in), agy refusal/cwd is covered, existing/pane options have zero effects, and no-option cases retain behavior. Extend `tests/herdr_release_launch_and_handoff` in the existing release matrix to check the real child's declared profile env and cwd across every listed Herdr release, with owned stand-ins and no live model quota. Record the shell-startup declaration limitation explicitly in evidence. No live credential profile or shared server is used.

Integration sweep verifies the public main flows, unwired fields, README/help/goldens/generated mirrors, neutral core check, fmt and locked all-feature clippy. Full suite and default-feature gate are caller-owned final integration checks under the existing budgets and scoped leak rules. Do not add flaky timing assertions or a routine repeated full suite.

## Release and follow-on

Document this as an unreleased minor feature, proposing 2.1.0 because it adds CLI behavior. Do not bump/tag/release, merge main or push. Caller owns code roast, green integration handoff, full suite and final short office update. Release-only authenticated wake e2e remains the releasing agent's prerequisite.

## Post-Implementation Notes

*As this design is implemented and iterated on — bug fixes, adjustments, anything that diverged from the assumptions above — append a dated note here, whether or not a formal debugging skill was used.*
