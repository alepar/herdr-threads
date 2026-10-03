## Goals

### ht-xoc (root)
A routine Claude Code or Codex upgrade never produces a degraded state or any Health noise. herdr-threads
reports a harness version as broken only when it has observed that version break the hook payload contract (or
the published canary has, for a version never seen locally), and then says exactly what to do. Marking a new
harness version as working or broken needs no herdr-threads build.

## Task tree

- ht-xoc · Harness version evidence: no noise on routine harness upgrades (epic) · Implements docs/superpowers/specs/2026-10-02-harness-version-evidence-design.md (branch harness-version-evidence). · deps: ht-p03.14, ht-p03.23, ht-p03.71
  - ht-xoc.1 · Payload contract declarations, contract_id and contract-id CLI · Per-harness contract as data (event kinds consumed, required fields, JSON types) derived from the parsers in src/harness/claude.rs and codex.rs; contract_id = f · deps: none
  - ht-xoc.2 · Seam contract: running-harness version attribution (ancestor walk, image check) · Seam contract: running-harness version attribution (spec Attribution steps 1-3). · deps: ht-xoc.8
      owns: attribution function and its reason strings. files: src/harness/attribution.rs (new), src/harness/mod.rs.
  - ht-xoc.3 · Manifest: schema 2, embedded copy, cache and fetch policy · Upgrade B6's in-repo harness-versions.json generator to schema_version 2 (canary-only fields null); embed at build (release: harness-manifest branch file, fallb · deps: none
  - ht-xoc.4 · Evidence store and verified-by-use recording · Migration adding harness_version_evidence(harness, version, contract_id, lifecycle_ok_at, tool_ok_at, violation_at, violation_event, violation_field, last_seen_ · deps: ht-xoc.1, ht-xoc.2
  - ht-xoc.5 · Contract-first state derivation and Health/doctor rendering · One pure function: local violation -> broken; ladder Refused -> broken; local verified -> working (manifest known_broken doctor-only); manifest known_broken (sa · deps: ht-xoc.1, ht-xoc.3, ht-xoc.4
  - ht-xoc.6 · Canary manifest writer and harness-manifest branch publishing · Extend B6 canary: write rows to the harness-manifest branch by direct bot commit (contents: write for that branch only); only payload-contract failures write kn · deps: ht-xoc.1, ht-xoc.3
  - ht-xoc.7 · Integration sweep: harness version evidence end to end · End to end with a stand-in harness: an unlisted new version -> no Health line -> first lifecycle + tool payloads -> working; a payload missing a required field  · deps: ht-xoc.1, ht-xoc.2, ht-xoc.3, ht-xoc.4, ht-xoc.5, ht-xoc.6, ht-xoc.8
  - ht-xoc.8 · Spike: real hook ancestry for Claude and Codex · Register a logging hook (via claude --settings / a temporary CODEX_HOME, never editing ~/.claude or ~/.codex) and record proc_pidpath(getppid()) plus the full a · deps: none
