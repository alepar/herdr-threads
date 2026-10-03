## Goals

### ht-xoc (root)
A routine Claude Code or Codex upgrade never produces a degraded state or any Health noise. herdr-threads
reports a harness version as broken only when it has observed that version break the hook payload contract (or
the published canary has, for a version never seen locally), and then says exactly what to do. Marking a new
harness version as working or broken needs no herdr-threads build.

## Task tree

- ht-xoc · Harness version evidence: no noise on routine harness upgrades (epic) · Implements docs/superpowers/specs/2026-10-02-harness-version-evidence-design.md (branch harness-version-evidence). · deps: ht-p03.14, ht-p03.23, ht-p03.71
  - ht-xoc.1 · Payload contract declarations, contract_id and contract-id CLI · Per-harness contract as data (event kinds consumed, required fields, JSON types) derived from the parsers in src/harness/claude.rs and codex.rs; contract_id = f · deps: none
      owns: the contract boundary consumed by ht-xoc.4, .5, .6: contract_id = first 16 hex of SHA-256 over canonical JSON (sorted keys, no whitespace, fields sorted p
  - ht-xoc.2 · Seam contract: running-harness version attribution (ancestor walk, image check) · Seam contract: running-harness version attribution (spec Attribution steps 1-3). · deps: ht-xoc.8
      owns: attribution function and its reason strings. files: src/harness/attribution.rs (new), src/harness/mod.rs.
      owns: result type Attributed{harness, version} | Unattributable{reason}; the recorder (ht-xoc.4) records nothing on Unattributable but persists its reason.
  - ht-xoc.3 · Manifest: schema 2, embedded copy, cache and fetch policy · Upgrade B6's in-repo harness-versions.json generator to schema_version 2 (canary-only fields null); embed at build (release: harness-manifest branch file, fallb · deps: none
      owns: manifest schema 2 row format (rows keyed by harness+version+contract_id; status, evidence, supported_since, last_working, issue_url, source canary|manual)
  - ht-xoc.4 · Evidence store and verified-by-use recording · Migration adding harness_version_evidence(harness, version, contract_id, lifecycle_ok_at, tool_ok_at, violation_at, violation_event, violation_field, last_seen_ · deps: ht-xoc.1, ht-xoc.2, ht-xoc.3
      owns: harness_version_evidence schema and read API (verified = lifecycle_ok_at and tool_ok_at both set for the same harness, version, contract_id) and a per-har
  - ht-xoc.5 · Contract-first state derivation and Health/doctor rendering · One pure function: local violation -> broken; ladder Refused -> broken; local verified -> working (manifest known_broken doctor-only); manifest known_broken (sa · deps: ht-xoc.1, ht-xoc.3, ht-xoc.4
  - ht-xoc.6 · Canary manifest writer and harness-manifest branch publishing · Extend B6 canary: write rows to the harness-manifest branch by direct bot commit (contents: write for that branch only); only payload-contract failures write kn · deps: ht-xoc.1, ht-xoc.3
  - ht-xoc.7 · Integration sweep: harness version evidence end to end · End to end with a stand-in harness: an unlisted new version -> no Health line -> first lifecycle + tool payloads -> working; a payload missing a required field  · deps: ht-xoc.1, ht-xoc.2, ht-xoc.3, ht-xoc.4, ht-xoc.5, ht-xoc.6, ht-xoc.8
  - ht-xoc.8 · Spike: real hook ancestry for Claude and Codex · Register a logging hook (via claude --settings / a temporary CODEX_HOME, never editing ~/.claude or ~/.codex) and record proc_pidpath(getppid()) plus the full a · deps: none

## Changes since the previous round
- ht-xoc.1 amended — C2 (classifier result Ok|Violation|Malformed, truncated test), C6 (owns contract boundary, CLI json)
- ht-xoc.2 amended — C11 (result type Attributed|Unattributable{reason})
- ht-xoc.3 amended — C7 (owns manifest schema 2, branch path/URL, source field), C10 (ensure_manifest entry point; state function reads only)
- ht-xoc.4 amended — C8 (owns evidence schema, verified definition), C9 (last_unattributed record), C10 (calls ensure_manifest; new edge ←ht-xoc.3)
- ht-xoc.5 amended — C1 (action selection rule), C3 (retire existing version emitters), C9 (render unattributed reason)
- ht-xoc.6 amended — C1 (supported_since from passing release contract), C4 (release embed test), C5 (manual edit path, source=manual)
- ht-xoc.7 amended — C2, C4, C9 scenarios
