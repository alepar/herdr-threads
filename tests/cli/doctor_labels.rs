//! ht-p03.27: doctor's `hooks.claude.observed` label and the daemon log path.
//! Each test names the mutation it kills.

use super::*;

#[test]
fn default_doctor_pins_result_and_issues_without_debug_inventory() {
    let report = json!({
        "result": "degraded",
        "context": {"ok": true, "state_dir": "/private/state", "host_endpoint": "/private/socket"},
        "state_dir": {"exists": true, "safe": true},
        "daemon": {"state": "healthy", "boot_id": "secret-debug-id"},
        "hooks": {
            "claude": {"setup": {"installed": false}, "installed": {"admission": "listed"}},
            "codex": {"setup": {"installed": true}, "installed": {"admission": "listed"}}
        },
        "limitations": ["claude hooks are missing: run `herdr-threads setup claude`"]
    });
    let text = render_text(&report);
    assert!(text.starts_with("doctor: degraded\n"), "{text}");
    assert!(text.contains("daemon: healthy\n"), "{text}");
    assert!(text.contains("issue: claude hooks are missing"), "{text}");
    assert!(!text.contains("secret-debug-id"), "{text}");
    assert!(!text.contains("state_dir:"), "{text}");
    assert!(render_debug_text(&report).contains("secret-debug-id"));
}

#[test]
fn repair_plan_refuses_unsafe_state_and_limits_setup_to_missing_hooks() {
    let mut report = json!({
        "context": {"ok": true}, "state_dir": {"safe": true},
        "daemon": {"state": "not_running"},
        "hooks": {
            "claude": {"setup": {"installed": false}, "installed": {"admission": "listed", "binary": "/bin/claude"}},
            "codex": {"setup": {"installed": true}, "installed": {"admission": "listed", "binary": "/bin/codex"}}
        }
    });
    assert_eq!(
        repair_plan(&report),
        vec![Repair::EnsureDaemon, Repair::SetupClaude]
    );
    report["state_dir"]["safe"] = json!(false);
    assert!(repair_plan(&report).is_empty());
    report["state_dir"]["safe"] = json!(true);
    report["hooks"]["claude"]["error"] = json!("ownership unknown");
    assert_eq!(repair_plan(&report), vec![Repair::EnsureDaemon]);
    report["hooks"]["claude"]["error"] = Value::Null;
    report["daemon"]["state"] = json!("unreachable");
    assert!(repair_plan(&report).is_empty());
    report["daemon"]["state"] = json!("healthy");
    report["daemon"]["version_matches"] = json!(false);
    assert!(repair_plan(&report).is_empty());
    report["daemon"]["version_matches"] = json!(true);
    report["daemon"]["state"] = json!("unavailable");
    assert!(repair_plan(&report).is_empty());
}

#[test]
fn doctor_fix_keeps_codex_global_sandbox_setup_explicit() {
    let report = json!({
        "context": {"ok": true}, "state_dir": {"safe": true},
        "daemon": {"state": "healthy"},
        "hooks": {"codex": {
            "setup": {"installed": false},
            "installed": {"admission": "listed", "binary": "/bin/codex"},
            "trust": {"status": "not_installed"}
        }}
    });
    assert!(repair_plan(&report).is_empty());
    let mut outcomes = Vec::new();
    add_manual_repairs(&report, &mut outcomes);
    assert!(
        outcomes.iter().any(|row| row["action"] == "setup codex"
            && row["outcome"] == "manual"
            && row["detail"]
                .as_str()
                .is_some_and(|text| text.contains("hooks are not installed"))),
        "{outcomes:?}"
    );

    let mut installed = report;
    installed["hooks"]["codex"]["setup"]["installed"] = json!(true);
    installed["hooks"]["codex"]["installed"]["admission"] =
        json!("schema-matched, live-unverified");
    installed["hooks"]["codex"]["installed"]["version"] = json!("0.160.0");
    let mut outcomes = Vec::new();
    add_manual_repairs(&installed, &mut outcomes);
    assert!(
        outcomes
            .iter()
            .all(|row| row["action"] != "Codex socket policy"),
        "{outcomes:?}"
    );
}

#[test]
fn compact_doctor_does_not_offer_fix_for_leftover_state() {
    let report = json!({
        "result": "unavailable",
        "context": {"ok": true, "source": {"state_dir_leftover": "uninstalled plugin"}},
        "state_dir": {"safe": true}, "daemon": {"state": "not_running"}
    });
    let text = render_text(&report);
    assert!(!text.contains("fix: herdr-threads doctor fix"), "{text}");
    assert!(repair_plan(&report).is_empty());
}

#[test]
fn compact_doctor_offers_fix_for_inspectable_missing_claude_hooks() {
    let report = json!({
        "result": "degraded", "context": {"ok": true},
        "state_dir": {"safe": true}, "daemon": {"state": "healthy"},
        "hooks": {"claude": {
            "setup": {"installed": false},
            "installed": {"admission": "listed", "binary": "/bin/claude"}
        }}
    });
    assert!(render_text(&report).contains("fix: herdr-threads doctor fix"));
}

#[test]
fn compact_doctor_shows_newly_detected_version_over_older_session() {
    let report = json!({
        "result": "degraded", "context": {"ok": true},
        "state_dir": {"safe": true}, "daemon": {"state": "healthy"},
        "harness_states": {"harnesses": [{
            "harness": "codex",
            "detected": {"version": "0.160.0", "state": "new"},
            "versions": [{"version": "0.159.3", "state": "working"}]
        }]}
    });
    let text = render_text(&report);
    assert!(
        text.contains("codex advisory history 0.160.0: new"),
        "{text}"
    );
}

#[test]
fn fix_outcomes_name_manual_trust_and_refuse_unsafe_state() {
    let report = json!({
        "context": {"ok": true}, "state_dir": {"safe": false},
        "daemon": {"state": "not_running"},
        "hooks": {"codex": {"setup": {"installed": true}, "trust": {"status": "review_required"}}}
    });
    let mut outcomes = Vec::new();
    add_manual_repairs(&report, &mut outcomes);
    assert!(
        outcomes
            .iter()
            .any(|row| row["action"] == "state directory" && row["outcome"] == "refused")
    );
    assert!(
        outcomes
            .iter()
            .any(|row| row["action"] == "Codex hook trust" && row["outcome"] == "manual")
    );
}

#[test]
fn codex_trust_labels_distinguish_review_required_recorded_and_unknown() {
    let mut report = json!({
        "result": "ok", "context": {"ok": true},
        "state_dir": {"safe": true}, "daemon": {"state": "healthy"},
        "hooks": {"codex": {
            "setup": {"installed": true}, "installed": {"admission": "listed"},
            "trust": {"status": "review_required"}
        }}
    });
    let required = render_text(&report);
    assert!(
        required.contains("Codex hook review required"),
        "{required}"
    );
    assert!(required.contains("/hooks"), "{required}");
    report["hooks"]["codex"]["trust"]["status"] = json!("recorded_unverified");
    let recorded = render_text(&report);
    assert!(
        recorded.contains("Codex hook trust recorded (current hashes unverified)"),
        "{recorded}"
    );
    assert!(!recorded.contains("review required"), "{recorded}");
    report["hooks"]["codex"]["trust"]["status"] = json!("unknown");
    let unknown = render_text(&report);
    assert!(unknown.contains("Codex hook trust unknown"), "{unknown}");
    assert!(!unknown.contains("review required"), "{unknown}");
}

/// Kills: labelling every non-Supported harness "(cooperative mode)" (the
/// Wave 26 bug), or dropping the label from the one state it describes.
#[test]
fn cooperative_mode_label_only_for_cooperative() {
    let none: [String; 0] = [];
    assert_eq!(
        claude_observed_text(HarnessState::Cooperative, &none, &none, None),
        CLAUDE_NOT_OBSERVABLE
    );
    assert!(CLAUDE_NOT_OBSERVABLE.contains("(cooperative mode)"));
    for state in [
        HarnessState::Unsupported,
        HarnessState::Supported,
        HarnessState::Unknown,
    ] {
        let text = claude_observed_text(state, &none, &none, None);
        assert!(!text.contains("cooperative"), "{state:?}: {text}");
    }
    assert_eq!(
        claude_observed_text(HarnessState::Supported, &none, &none, None),
        "observed"
    );
    assert_eq!(
        claude_observed_text(HarnessState::Unknown, &none, &none, None),
        "unknown"
    );
}

/// Kills: an Unsupported (refused) claude printing a generic or cooperative
/// line instead of the daemon's own refusal reason, and a not-installed claude
/// losing its reason (it sits in notes, not limitations).
#[test]
fn unsupported_prints_its_own_reason() {
    let refused = vec![
        "host unavailable: server not running".to_owned(),
        "harness claude unsupported: claude 9.9.9: no recipe admits it".to_owned(),
    ];
    assert_eq!(
        claude_observed_text(HarnessState::Unsupported, &refused, &[], None),
        "not observable: claude 9.9.9: no recipe admits it"
    );
    let absent = vec![
        "harness claude not installed: no executable `claude` on the daemon's PATH".to_owned(),
    ];
    assert_eq!(
        claude_observed_text(HarnessState::Unsupported, &[], &absent, None),
        "not observable: no executable `claude` on the daemon's PATH"
    );
    // The codex line must not be mistaken for claude's reason.
    let codex_only = vec!["harness codex unsupported: codex 1.0: refused".to_owned()];
    assert_eq!(
        claude_observed_text(HarnessState::Unsupported, &codex_only, &[], None),
        "not observable: the daemon refused the installed claude version (see its version verdict below)"
    );
}

/// Kills: a VersionRefused claude (no Health line) that does not name the
/// refusal from the version verdict, and a verdict that outranks a Health line.
#[test]
fn version_refused_names_the_refusal() {
    let verdict = "claude 2.1.100 is below the supported floor 2.1.283; upgrade claude";
    assert_eq!(
        claude_observed_text(HarnessState::Unsupported, &[], &[], Some(verdict)),
        format!("not observable: {verdict}")
    );
    let refused = vec!["harness claude unsupported: claude 9.9.9: no recipe admits it".to_owned()];
    assert_eq!(
        claude_observed_text(HarnessState::Unsupported, &refused, &[], Some(verdict)),
        "not observable: claude 9.9.9: no recipe admits it"
    );
}

/// Kills: doctor's text form omitting the daemon log it already knows (the
/// path was only in the JSON report).
#[test]
fn doctor_text_prints_the_daemon_log_path() {
    let report = json!({
        "version": "0",
        "protocol_version": 1,
        "context": {"ok": true, "state_dir": "/s", "source": {}, "host_endpoint": "/h"},
        "state_dir": {"exists": true, "safe": true},
        "instance_dir": "/s/instances/i",
        "daemon_log": "/s/instances/i/logs/daemon.log",
        "daemon": {"state": "degraded"},
        "hooks": {"claude": {"setup": {}, "installed": {}}, "codex": {"setup": {}, "installed": {}}},
        "result": "degraded",
    });
    let text = render_debug_text(&report);
    assert!(
        text.contains("daemon_log: /s/instances/i/logs/daemon.log\n"),
        "{text}"
    );
    let mut no_log = report.clone();
    no_log["daemon_log"] = Value::Null;
    assert!(!render_debug_text(&no_log).contains("daemon_log:"));
}

// -- ht-xoc.5: the per-harness verdict block -----------------------------------

fn states_report(harnesses: serde_json::Value) -> Value {
    json!({
        "version": "0",
        "protocol_version": 1,
        "context": {"ok": true, "state_dir": "/s", "source": {}, "host_endpoint": "/h"},
        "state_dir": {"exists": true, "safe": true},
        "daemon": {"state": "healthy"},
        "harness_manifest": {
            "policy": "auto", "source": "default", "settings_error": null,
            "cache_fetched_at": "2026-10-02T10:00:00Z", "cache_etag": "e1",
            "embedded_schema_version": 2,
        },
        "harness_states": {"harnesses": harnesses},
        "hooks": {"claude": {"setup": {}, "installed": {}}, "codex": {"setup": {}, "installed": {}}},
        "result": "ok",
    })
}

fn row(version: &str, state: &str, source: &str, line: &str, last_seen: u64) -> Value {
    json!({
        "version": version, "state": state, "source": source, "line": line,
        "notes": [], "issue_url": null, "last_seen_at": last_seen, "in_health_window": true,
    })
}

/// The block lines of `text`: everything between the first `harness <h>:`
/// line and the manifest policy line.
fn block(text: &str) -> Vec<&str> {
    let start = text
        .lines()
        .position(|l| l.starts_with("harness ") && !l.starts_with("harness manifest"));
    let end = text.lines().position(|l| l.starts_with("harness manifest"));
    match (start, end) {
        (Some(start), Some(end)) if start < end => {
            text.lines().skip(start).take(end - start).collect()
        }
        _ => Vec::new(),
    }
}

/// Kills: a block that omits the verdict source for any state (local
/// evidence, canary, manual, recipe, below floor, no evidence), the other
/// rows in the window, doctor notes and the issue URL.
#[test]
fn doctor_text_pins_the_verdict_block_for_every_state() {
    let mut newest = row(
        "2.1.286",
        "working",
        "local evidence (lifecycle + tool payloads)",
        "claude 2.1.286: working",
        2_000,
    );
    newest["notes"] =
        json!(["known broken in >= 2.1.290 per the recipe tables; it has worked here"]);
    let mut broken = row(
        "2.1.290",
        "broken",
        "canary manifest row",
        "harness claude 2.1.290 broken: the canary manifest row reports SessionStart payload field source; report: https://example.test/i",
        1_500,
    );
    broken["issue_url"] = json!("https://example.test/i");
    let stale = {
        let mut stale = row("2.1.200", "working", "recipe tables", "x", 10);
        stale["in_health_window"] = json!(false);
        stale
    };
    let report = states_report(json!([
        {"harness": "claude", "contract_id": "0123456789abcdef", "detected": null,
         "versions": [newest, broken, row("2.1.288", "working", "canary manifest row (live)", "x", 1_400),
                      row("2.1.287", "working", "manual manifest row", "x", 1_300),
                      row("2.0.1", "broken", "below the recipe floor",
                          "claude 2.0.1 is below the supported floor 2.1.283; upgrade claude", 1_200),
                      stale],
         "unattributed": null, "hook_parse_failures": 0},
        {"harness": "codex", "contract_id": null,
         "detected": {"version": "0.999.0", "state": "new",
                      "line": "new version, not yet seen working; verified on first use"},
         "versions": [], "unattributed": null, "hook_parse_failures": 0},
    ]));
    let text = render_debug_text(&report);
    assert_eq!(
        block(&text),
        [
            "harness claude advisory history: working 2.1.286 \u{2014} local evidence (lifecycle + tool payloads)",
            "  note: known broken in >= 2.1.290 per the recipe tables; it has worked here",
            "  claude 2.1.290: broken \u{2014} canary manifest row",
            "  harness claude 2.1.290 broken: the canary manifest row reports SessionStart payload field source; report: https://example.test/i",
            "  issue: https://example.test/i",
            "  claude 2.1.288: working \u{2014} canary manifest row (live)",
            "  claude 2.1.287: working \u{2014} manual manifest row",
            "  claude 2.0.1: broken \u{2014} below the recipe floor",
            "  claude 2.0.1 is below the supported floor 2.1.283; upgrade claude",
            "harness codex: new 0.999.0 (on PATH, no session yet) \u{2014} new version, not yet seen working; verified on first use",
        ],
        "{text}"
    );
    // The stale row (outside the Health window) is not printed; the manifest
    // policy and cache lines follow the block.
    assert!(!text.contains("2.1.200"), "{text}");
    assert!(
        text.contains("harness manifest: auto\nmanifest cache: fetched 2026-10-02T10:00:00Z"),
        "{text}"
    );
}

/// Kills: the unattributed reason shown although a newer row exists (it is
/// stale then), hidden when it is newer, a wrong time format, and a
/// parse-failure line that is printed for zero.
#[test]
fn doctor_text_shows_unattributed_reason_and_parse_failures() {
    let mut harness = json!({
        "harness": "claude", "contract_id": "0123456789abcdef", "detected": null,
        "versions": [row("2.1.286", "working", "recipe tables", "x", 5_000)],
        "unattributed": {"reason": "resume before first entry", "at": 4_000},
        "hook_parse_failures": 0,
    });
    let older = render_debug_text(&states_report(json!([harness.clone()])));
    assert!(!older.contains("version evidence unavailable"), "{older}");
    assert!(!older.contains("hook payloads not understood"), "{older}");
    harness["unattributed"]["at"] = json!(1_791_000_000_000_u64);
    harness["hook_parse_failures"] = json!(3);
    let newer = render_debug_text(&states_report(json!([harness])));
    assert!(
        newer.contains(
            "version evidence unavailable: resume before first entry (2026-10-03T04:00:00Z)\n"
        ),
        "{newer}"
    );
    assert!(
        newer.contains("hook payloads not understood: 3\n"),
        "{newer}"
    );
}

/// Kills: a daemon that cannot answer printing nothing (the operator cannot
/// tell "no versions" from "not asked"), and terminal escapes in a daemon
/// string reaching the terminal.
#[test]
fn doctor_text_says_why_there_are_no_states_and_escapes_daemon_text() {
    let mut report = states_report(json!([]));
    report["harness_states"] = Value::Null;
    report["harness_states_unavailable"] = json!("the daemon is not running");
    let text = render_debug_text(&report);
    assert!(
        text.contains("harness states unavailable: the daemon is not running\n"),
        "{text}"
    );
    let hostile = states_report(json!([
        {"harness": "claude", "contract_id": null, "detected": null,
         "versions": [row("2.1.286", "working", "src\u{1b}[31mred", "x", 1)],
         "unattributed": null, "hook_parse_failures": 0}
    ]));
    assert!(!render_debug_text(&hostile).contains('\u{1b}'));
}

// Metadata absence alone offers no fix; real missing configuration does.
#[test]
fn task3_versionless_doctor_metadata_unknown_is_informational_not_repairable() {
    let mut report = json!({"context": {"ok": true}, "state_dir": {"safe": true},
    "daemon": {"state": "healthy"}, "hooks": {"claude": {
        "setup": {"installed": true}, "installed": {"binary": "/bin/wrapper", "version": null, "admission": "contract_declared"}
    }}});
    assert!(repair_plan(&report).is_empty());
    let mut repairs = Vec::new();
    add_manual_repairs(&report, &mut repairs);
    assert!(
        !repairs
            .iter()
            .any(|repair| repair["action"] == "claude version")
    );
    report["hooks"]["claude"]["setup"]["installed"] = json!(false);
    assert_eq!(repair_plan(&report), vec![Repair::SetupClaude]);
}
