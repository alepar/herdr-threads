# Preliminary architecture review

Independent native subagent review of the research preview, 2026-10-03. This is design review only, not approval or implementation certification.

Two material findings were returned:

1. Durable storage closes the harness set as well as the Rust enums. `src/store/schema.rs` recognizes a literal Claude/Codex/Human constraint, and `src/store/queries.rs` restricts cooperative inbox display to Claude/Codex. The lead's follow-up also found closed checks in migration 0012's version evidence and unattributed tables. The preview now includes a one-time forward migration, old-schema reopening tests, and generic canonical agent eligibility. Historical migrations remain unchanged.
2. Hermes observers cannot establish the full registration/delivery path. The first callback with usable role information must perform attachment; subsequent turns use Current; explicit reset is deferred until a qualified Clear callback. Existing history is not Resume evidence. Observer-only callbacks do not consume offers. Callback deadlines can discard output, so flushed Rust stdout remains offered bridge context, not accepted context or receipt. The preview now includes this state machine and timeout/replay/lost-output validation.

The reviewer otherwise found the registry and typed optional capability direction suitable for the extensibility criterion. Remaining Hermes native/API qualification and user scope/compatibility choices are recorded in the report and dossier. A formal approved spec and implementation review are still required.
