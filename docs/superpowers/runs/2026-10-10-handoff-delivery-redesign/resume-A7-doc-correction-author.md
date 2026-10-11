# A7 documentation correction author handoff

Status: **COMMIT QUIESCENT**. Documentation source is frozen for parent transfer and independent whole-spec review. No Cargo, build, test, Herdr CLI, daemon/helper background process or subagent action was performed. No runtime, whole-spec pass or promotion claim is made.

- Author checkout: `/Users/alepar/AleCode/herdr-threads/.worktrees/handoff-delivery-redesign/.worktrees/finish-composed-hook`
- Branch: `helper/handoff-composed-public-ensure`
- Immediate base: `29b5786dda9c9c8a5d7e89a6f2807b3ba22038a2`; base tree: `8ae57ac7fff8474aaf80c202c39245bb2f43f0ec`
- Documentation correction commit: `cc686d29416522d1405e79df5c857c95438e819d`
- Commit tree: `bd278a7c987e13b6a0436495b2e2d0a9cc089060`
- Only path changed in this commit: `TRUST-POLICY.md` (5 added lines, 9 removed lines).
- Original policy blob: `09292a8d83bd05bbcd62ecd1491d19479f8f8389`; original SHA256: `1ed976f8abbf661c7923800f2901655b72babd1000ef15ade90ad2d9d841452f`
- Corrected policy blob: `09cd4b20a83cf9915654dd9930cdcc49c34863e7`; corrected SHA256: `49441e2e6eee407e6be9b205af3afb81efe9c87eb4d6bf807518cd99413a48a9`
- Changed paragraph: current lines519–523; original changed region519–527.
- Trailer: `Outside-adapter: TRUST-POLICY.md`
- Production model-visible text added: **0 bytes**; normative policy prose only.

Read the normative policy and the complete A7 section before editing. The preceding A7 text explicitly retains original native inviter wake addresses for still-open invitation deadline conditions and canonical native author wake addresses for exactly associated receipt deadline conditions. The later duplicated historical paragraph incorrectly said only the affected seat wakes and every transition about another seat waits, contradicting those retained exceptions. The first duplicate passive-observer paragraph also ended with a disconnected `When`.

Replaced those duplicate paragraphs with one paragraph: clears remain passive; transitions about another seat remain passive for other observers apart from the already specified canonical origin wakes; the accepted idle-member limit applies to a member who is only a passive observer. Preserved both next-check-in and hook-pull/turn-start opportunities. The preceding exact origin exceptions, bounded-window conservative fallback, and live Claude mod channel attention fingerprint/informational wording are unchanged. The retained `When` now directly introduces the following bounded-window sentence with no intervening duplicate paragraph.

Scope is strictly documentation reconciliation: no new policy, invariant, accepted limit, wake behavior, production text, test, schema or unrelated prose rewrite. No ambiguity required a policy choice. Parent should transfer only `cc686d29416522d1405e79df5c857c95438e819d` onto the composed candidate; prior C helper commits have separate transfer/execution provenance.

Static author evidence: precommit `git diff --check` exit0/empty output; committed `git diff HEAD^ HEAD --check` exit0/empty output; postcommit `git status --short` exit0/empty output. Inspected the exact14-line diff and final A7 paragraph. Parent retains matrix, review, execution and promotion ownership; this doc-only helper did not mutate the frozen composition.
