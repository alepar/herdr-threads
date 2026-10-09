ht-uwd.10 Configuration smoke: Claude/Codex, bare/ht/absolute/pinned, root/human
Exercise isolated fixture entry points for both backend renderer/install/status/remove, all four spelling categories, supported pinned/output forms, quoted body and compounds. Run parse_argv fixtures for root/human, legacy/operator/inferred-Human refusal and --human formatting. Early verification is not terminal readiness only. owns: cross-configuration focused smoke fixtures. consumes: native backends; namespace grammar; runtime actor boundary. Acceptance: named command/fixture per configuration, actual nonzero tests, model-free evidence labels, relevant RED/GREEN. Files: tests/permissions_smoke.rs (combined module), tests/combined.rs registration; test-support required if spawning.
blocked-by ht-uwd.3: consumes Runtime honest actor routing and human lifecycle
blocked-by ht-uwd.5: consumes Claude narrow permissions and durable legacy ownership migration
blocked-by ht-uwd.6: consumes Codex owned execpolicy backend
Acceptance deliverables above consume required predecessors (needs: ht-uwd.3).
Deps: ht-uwd.6,ht-uwd.3,ht-uwd.5

ht-uwd.9 Human command guidance and immutable continuation rendering
Update owned guidance, generated pending/retry/ready/continuation commands and TRUST-POLICY spelling/native limits. Human output ready argv preserves immediate human; no new strict OutputSpec/ContinuationContext wire field, digest rewrite, or blind replacement of peer/body text. Supply exact shared SKILL/handoff help wording to w4:pD0 via parent and consume only coordinated source. owns: lawful namespace suggestions and documentation. consumes: InvocationActor route; frozen retry origin classifier; runtime actor boundary. Acceptance: valid legacy wire fixtures and generated human/agent continuation argv; precise suggestions and no peer/body rewriting. Files: src/cli/output.rs, src/cli/journal.rs rendering, src/cli/me.rs help, TRUST-POLICY.md, docs/install.md; shared integrations/skill/SKILL.md and handoff help owner-coordinated.
blocked-by ht-uwd.2: consumes Immutable original actor classification and retry preflight
blocked-by ht-uwd.3: consumes Runtime honest actor routing and human lifecycle
Acceptance deliverables above consume required predecessors (needs: ht-uwd.2).
Deps: ht-uwd.2,ht-uwd.3

ht-uwd.8 Installer validated owned paths and permission gateway
Pass canonical installed binary, owned herdr-threads link and ht alias plus pinned routing into component reconciliation. Reuse exact alias/foreign/force/PATH protection; update consent/exit/next steps for independent permissions, no unrelated PATH grant. owns: installer executable inventory transport. consumes: independent component lifecycle and consent. Acceptance: actual installer shell fixture gateway plus Rust consumer, bare/absolute/custom-prefix paths and foreign/declined/update/uninstall preservation; installed path set verified. Files: scripts/install.sh, tests/release/install_test.sh, src/cli/commands.rs internal installer arguments, src/cli/installer.rs input plumbing.
blocked-by ht-uwd.7: consumes Independent setup lifecycle and permission consent
Acceptance deliverables above consume required predecessors (needs: ht-uwd.7).
Deps: ht-uwd.7

ht-uwd.7 Independent setup lifecycle and permission consent
Wire permissions beside hooks/skill for explicit setup/status/unsetup, installer-integrations reconciliation and requested doctor fix. Explicit permission consent names ordinary reads/writes and human/operator exclusion; hooks ownership alone never grants. Sequential shared-settings plans re-read exact inspected state after consent. owns: component lifecycle and consent. consumes: Claude permission backend; Codex permission backend; permission component API. Acceptance: real Rust synthetic component path install/status/remove + absent/declined/noninteractive consent and mixed hook status, no clobber; isolated roots only. Files: src/cli/setup.rs, src/cli/installer.rs, src/cli/doctor.rs, src/cli/commands.rs setup-specific declarations, tests/installer_integrations.rs.
blocked-by ht-uwd.5: consumes Claude narrow permissions and durable legacy ownership migration
blocked-by ht-uwd.6: consumes Codex owned execpolicy backend
Acceptance deliverables above consume required predecessors (needs: ht-uwd.5).
Deps: ht-uwd.6,ht-uwd.5

ht-uwd.6 Codex owned execpolicy backend
Generate exact executable union allow plus matching immediate human prompt, install/status/remove owned rules file under active CODEX_HOME/rules. Retain stronger policy and separate existing codex_config network/socket/writable-root allowances. owns: Codex permission backend. consumes: PermissionInputs, PermissionPlan/Inspection and component manifest schema. Acceptance: isolated literal/union checker contract, precedence, paths, quoted/compound evidence scope, edited/foreign/partial/symlink/race removal refusal; no shell prefixes or modes. Files: src/harness/permissions/codex.rs, tests/permissions_codex.rs (combined module).
blocked-by ht-uwd.4: consumes boundary contract
Acceptance deliverables above consume required predecessors (needs: ht-uwd.4).
boundary contract: ht-uwd.4
Deps: ht-uwd.4

ht-uwd.5 Claude narrow permissions and durable legacy ownership migration
Implement independent Claude positive-family renderer plus exact lifecycle, decouple hook auto-grant/completeness, durable transfer from historical owned broad/retired rules. Design transfer interruption and shared settings conflict behavior before code. owns: Claude permission backend and historical hook transfer. consumes: PermissionInputs, PermissionPlan/Inspection and component manifest schema; ordinary command catalog. Acceptance: historical/pre-existing/edited/partial/symlink/race fixtures; no broad restore or foreign adoption; preserve unrelated/deny/ask, native limits disclosed. Files: src/harness/setup.rs, src/harness/claude.rs, src/harness/permissions/claude.rs, tests/installer_integrations.rs.
blocked-by ht-uwd.4: consumes boundary contract
Acceptance deliverables above consume required predecessors (needs: ht-uwd.4).
boundary contract: ht-uwd.4
Deps: ht-uwd.4

ht-uwd.4 Seam contract: owned permission component and validated executable inputs
Deliver compilable inert-by-default permission module/API: bounded validated executable spellings and routing, inspect/plan/install/remove backend types, separate manifest/status/consent fields, module wiring. Only boundary code and its acceptance; no backend implementation/migration/polish. owns: PermissionInputs, PermissionPlan/Inspection and component manifest schema. consumes: ordinary command catalog. Acceptance: compile, no silent grant, validated union rejects foreign/control/relative paths; stubs wired to consumer boundaries. Files: src/harness/permissions.rs or permissions/mod.rs, src/harness/mod.rs, src/cli/setup.rs boundary declarations, src/cli/installer.rs boundary declarations.
blocked-by ht-uwd.1: consumes Namespace grammar and ordinary command catalog
Acceptance deliverables above consume required predecessors (needs: ht-uwd.1).
Deps: ht-uwd.1

ht-uwd.3 Runtime honest actor routing and human lifecycle
Propagate route through caller/selection/cooperative/operator paths and me init; root accountable actions refuse inferred Human before intent/context retirement/effects; Human route lawful communication/lifecycle with existing A2/A3/A4, own inbox no display ACK. Native hooks unchanged. owns: runtime actor boundary. consumes: InvocationActor route. Acceptance: synthetic/private fixture RED/GREEN inferred Human root refusal, explicit agent selection, person communication/check-in/operator repair and no pre-refusal effects. Files: src/cli/mod.rs dispatch/caller paths, src/cli/me.rs, tests/cli/human.rs, tests/cli/cooperative.rs.
blocked-by ht-uwd.1: consumes Namespace grammar and ordinary command catalog
Acceptance deliverables above consume required predecessors (needs: ht-uwd.1).
Deps: ht-uwd.1

ht-uwd.2 Immutable original actor classification and retry preflight
Classify validated original SemanticMutation/frozen CallerClaim/IntentScope, including Native lifecycle and contradictions, before completed handoff shortcut output/cleanup. Existing agent completed replay after binding change remains root-valid; retained human/operator only human retry. Read absent journal without mkdir; no schema/claim/scope/digest/key rewrite. owns: frozen retry origin classifier and preflight. consumes: InvocationActor route. Acceptance: RED/GREEN legacy person/operator, completed agent/person, Native and malformed; byte-identical replay invariance. Files: src/cli/journal.rs, src/cli/retry.rs, src/cli/mod.rs preflight only, tests/cli/journal.rs, tests/cli/follow_retry.rs; coordinate handoff.rs owner through parent.
blocked-by ht-uwd.1: consumes Namespace grammar and ordinary command catalog
Acceptance deliverables above consume required predecessors (needs: ht-uwd.1).
Deps: ht-uwd.1

ht-uwd.1 Namespace grammar and ordinary command catalog
Implement immediate raw argv[1] human route separate from output; reject legacy person/operator aliases/globals with correct replacement. Own commands.rs and new actor_route.rs (human.rs is output rendering). Export positive ordinary-family catalog for renderers, supported pinned/output spellings. owns: InvocationActor route and ordinary command catalog. consumes: none. Acceptance: meaningful parse RED/GREEN bare/ht/absolute, pinned globals, output --human, body data, all legacy operator/person aliases. Files: src/cli/commands.rs, src/cli/actor_route.rs, tests/cli/commands.rs.
Deps: 