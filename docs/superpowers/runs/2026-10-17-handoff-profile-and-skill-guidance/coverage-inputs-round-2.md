## Goals

### ht-1urq (root)
Agents can hand off to a fresh tab using an explicitly selected existing harness profile directory and working directory, with the same selected inputs preserved across retry. The shipped skill teaches readable names, timely priority-0 delivery, explicit receipt dependence and proactive recovery when requested receipts do not arrive.

## Task tree

- ht-1urq · Handoff profile directories and collaboration skill guidance (epic) · Deliver the committed handoff launch-directory and collaboration-guidance design to merge-ready code. · deps: none
  - ht-1urq.1 · Seam contract: frozen launch directory context · Land the inert shared directory-context type, defaulted optional retained fields and adapter policy seam so later leaves compile against one contract. · deps: none
      owns: LaunchDirectoryContext
      owns: optional launch-context transport fields and LaunchPolicy explicit-directory seam.
  - ht-1urq.2 · Parse and validate new-tab profile directory inputs · Expose new-tab --profile-dir and validate explicit profile/cwd paths before any recipient resolution, durable publication or native effects. · deps: ht-1urq.1
      owns: public handoff directory grammar and path normalization into LaunchDirectoryContext capture inputs.
      consumes: optional launch-context transport fields and LaunchPolicy explicit-directory seam.
      owns: pane/existing directory-option refusal before effects and focused parser/no-option/plain-launch regression tests.
  - ht-1urq.3 · Map explicit Codex and Claude profile directories · Make native config-root adapters select explicit profile directories ahead of inherited or shell hints for all launch inspection and child inputs. · deps: ht-1urq.1
      owns: explicit native config-root profile projection for Codex and Claude.
      consumes: LaunchDirectoryContext
      consumes: LaunchPolicy explicit-directory seam.
      owns: credential-safe native launch diagnostics and focused explicit-over-inherited adapter tests.
  - ht-1urq.4 · Map explicit Hermes roots and retain unsupported adapter behavior · Let Hermes explicit directory selection seed HERMES_HOME without changing named-profile resolution; retain agy unsupported explicit selection. · deps: ht-1urq.1
      owns: Hermes explicit root to native profile resolution and declared projection.
      consumes: LaunchDirectoryContext
      consumes: LaunchPolicy explicit-directory seam.
      owns: focused Hermes named-profile and agy unsupported/cwd regression tests.
  - ht-1urq.5 · Validate and retain context-bearing bootstrap identities · Admit additive context-bearing identities canonically while preserving exact historical context-free validation and replay. · deps: ht-1urq.1
      owns: canonical launch-context admission, capability and exact retained identity/child/report validation.
      consumes: LaunchDirectoryContext
      consumes: optional launch-context transport fields.
      owns: focused canonical capability admission, historical identity and exact replay regression tests.
  - ht-1urq.6 · Apply frozen profile environment through new-tab handoff · Wire selected directories end to end through preflight, tab creation, derived launch and exact retry for automatic and explicit-thread new-tab modes. · deps: ht-1urq.2, ht-1urq.3, ht-1urq.4, ht-1urq.5
      owns: selected directory capture-to-create-to-derived-launch wiring and retry filesystem checks.
      consumes: LaunchDirectoryContext
      consumes: normalized public directory inputs
      consumes: canonical launch-context admission
      consumes: native config-root and Hermes profile projections.
      owns: guarded executable/PATH preservation and focused frozen retry/lost-reply/disappeared-directory tests.
  - ht-1urq.7 · Teach names, immediate delivery and receipt recovery in shipped skill · Update the shipped collaboration guide with name-first prose, timely p0 dispatch and useful responses to missing receipt confirmations. · deps: none
      owns: static model-facing collaboration guidance and its matching documentation.
      owns: measured baseline/final skill UTF-8 bytes and at most 1 KiB growth accounting.
  - ht-1urq.8 · Document launch directory support and proposed minor version · Align help, user documentation and changelog with supported new-tab directories and deliberate pane/agy limitations. · deps: ht-1urq.2
      owns: public handoff option reference, help/goldens and unreleased change summary.
      consumes: public handoff directory grammar.
      owns: proposed unreleased minor 2.1.0 with no package version bump, tag or release.
  - ht-1urq.9 · Configuration smoke: selected profile and cwd in release matrix · Exercise public handoff configuration and real child env/cwd with isolated stand-ins across the existing Herdr release matrix. · deps: ht-1urq.6
      owns: executable configuration smoke and release-matrix directory evidence.
      consumes: end-to-end frozen new-tab launch wiring.
      owns: credential-free owned-fixture evidence, scoped process/temp cleanup and no real config or shared-server changes.
