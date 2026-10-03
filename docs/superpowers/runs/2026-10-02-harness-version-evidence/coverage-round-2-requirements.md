R1 A routine harness upgrade (a version with no evidence yet) produces no Health line; doctor shows one informational line
R2 A version that delivers one valid lifecycle payload and one valid tool payload becomes working (persisted, silent)
R3 A contract violation (well-formed payload, required field missing or wrong type) makes the version broken: one degraded Health line naming harness, version, event and field
R4 The broken line carries an action from the manifest: upgrade herdr-threads to X, else pin the harness to the last working version, else an issue link
R5 Payloads are attributed to the running harness process's version (ancestor walk, replaced-image check); unattributable payloads record nothing and doctor says why
R6 The payload contract is declared as data with a stable contract_id exposed by a contract-id CLI
R7 Manifest schema 2 with per-contract rows, an embedded copy, a cache, and fetches only on need (at most once a day per harness), with an opt-out and offline tolerance
R8 Local evidence wins for "works"; a manifest known_broken applies only to versions never verified here; an unsupported schema means no manifest
R9 The canary publishes rows to the harness-manifest branch without a build; only payload-contract failures become known_broken; retention and size cap hold
R10 The release build embeds the manifest branch file
R11 Malformed or truncated payloads never degrade
R12 State derivation is one pure function covering every input combination
R13 Every pre-existing version-related Health/doctor emitter is retired or routed through the state function, so an unlisted version produces no Health line from any check
R14 A canary known_broken row reaches a client end to end (branch file → fetch → broken line with action) with no herdr-threads build
R15 A maintainer can mark or correct a manifest row by hand, with no build, and the canary does not overwrite it
R16 An unattributable payload leaves a durable reason that doctor shows
