//! Canary capture-directory reader (nested spec §D5). The gated test runs only when
//! HT_CANARY_CAPTURE_DIR is set; it parses the captured payloads and reports the Codex schema
//! fingerprint and launch-table drift into canary-rust.json (reported, never asserted).
//! ht-p03.14.8 delivers the contract types. The ungated flag-table test always runs.
use crate::harness::contract::{Classification, Malformed, classify, contract_for, contract_id};
use serde::Deserialize;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, serde::Serialize)]
#[serde(rename_all = "kebab-case")]
enum ExpectedAdmission {
    Listed,
    Optimistic,
    SchemaMatchedOrOptimistic,
    Refused,
    Unasserted,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
struct CanaryProbe {
    harness: String,
    version: String,
    binary: PathBuf,
    expected_admission: ExpectedAdmission,
}

fn read_probe(dir: &Path) -> std::io::Result<CanaryProbe> {
    let bytes = std::fs::read(dir.join("canary-probe.json"))?;
    serde_json::from_slice(&bytes)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))
}

fn read_help(dir: &Path, name: &str) -> Option<String> {
    std::fs::read_to_string(dir.join("help").join(format!("{name}.txt"))).ok()
}

/// A historical recipe refusal fails its diagnostic when the probe did not expect it.
fn historical_refusal_is_failure(expected: ExpectedAdmission) -> bool {
    expected != ExpectedAdmission::Refused
}

#[test]
fn canary_probe_sample_decodes() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("scripts/canary/testdata");
    let probe: CanaryProbe =
        serde_json::from_slice(&std::fs::read(dir.join("canary-probe-sample.json")).unwrap())
            .unwrap();
    assert_eq!(
        probe.expected_admission,
        ExpectedAdmission::SchemaMatchedOrOptimistic
    );
    assert!(historical_refusal_is_failure(probe.expected_admission));
    assert!(!historical_refusal_is_failure(ExpectedAdmission::Refused));
    for e in [
        ExpectedAdmission::Listed,
        ExpectedAdmission::Optimistic,
        ExpectedAdmission::Unasserted,
    ] {
        assert!(historical_refusal_is_failure(e));
    }
}

/// Result of replaying one capture directory (§D5): per-payload parse results and the failures.
struct CaptureRun {
    report: serde_json::Value,
    failures: Vec<String>,
}

fn harness_of(name: &str) -> Option<crate::harness::context::Harness> {
    use crate::harness::context::Harness;
    match name {
        "claude" => Some(Harness::Claude),
        "codex" => Some(Harness::Codex),
        _ => None,
    }
}

/// Whether the witness recorded `<stem>.argv` for a hook invocation of `harness`: its arguments
/// (one per line) end `hook <harness>` or `hook <harness> --event NAME` (setup registers each hook
/// with its event). Returns `Some(registered event)` for a hook capture (`Some(None)` for the
/// legacy form without `--event`), `None` otherwise. A capture without that `.argv` is some other
/// herdr-threads invocation (or not a witness capture) and is never parsed as a payload.
fn hook_argv_event(stdin_path: &Path, harness: &str) -> Option<Option<String>> {
    let argv = std::fs::read_to_string(stdin_path.with_extension("argv")).ok()?;
    let args: Vec<&str> = argv.lines().collect();
    match args.as_slice() {
        [.., "hook", h, "--event", name] if *h == harness => Some(Some((*name).to_owned())),
        [.., "hook", h] if *h == harness => Some(None),
        _ => None,
    }
}

/// One captured payload: display name, event label, the event its hook was registered for, bytes.
struct Captured {
    name: String,
    event: String,
    registered: Option<String>,
    bytes: Result<Vec<u8>, String>,
}

/// `capture/tier0/*.stdin` (raw hook stdin, only those whose `.argv` ends `hook <harness>`) and
/// `capture/tier1/*.json` (`{"event","stdin","stdout"}`), each sorted by name, as
/// `(display name, event label, stdin bytes)`.
fn read_payloads(dir: &Path, harness: &str) -> Vec<Captured> {
    let mut out = Vec::new();
    let list = |sub: &str, ext: &str| -> Vec<PathBuf> {
        let mut files: Vec<PathBuf> = std::fs::read_dir(dir.join("capture").join(sub))
            .into_iter()
            .flatten()
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| p.extension().is_some_and(|e| e == ext))
            .collect();
        files.sort();
        files
    };
    for path in list("tier0", "stdin") {
        let Some(registered) = hook_argv_event(&path, harness) else {
            continue;
        };
        let name = format!(
            "capture/tier0/{}",
            path.file_name().unwrap().to_string_lossy()
        );
        let bytes = std::fs::read(&path).map_err(|e| e.to_string());
        let event = bytes
            .as_ref()
            .ok()
            .map_or_else(String::new, |b| event_name(b));
        out.push(Captured {
            name,
            event,
            registered,
            bytes,
        });
    }
    for path in list("tier1", "json") {
        let name = format!(
            "capture/tier1/{}",
            path.file_name().unwrap().to_string_lossy()
        );
        let doc = std::fs::read(&path)
            .map_err(|e| e.to_string())
            .and_then(|raw| {
                serde_json::from_slice::<serde_json::Value>(&raw).map_err(|e| e.to_string())
            });
        let registered = doc
            .as_ref()
            .ok()
            .and_then(|d| d.get("event"))
            .and_then(|e| e.as_str())
            .map(str::to_owned);
        let bytes = doc.and_then(|doc| {
            doc.get("stdin")
                .and_then(|s| s.as_str())
                .map(|s| s.as_bytes().to_vec())
                .ok_or_else(|| "tier1 capture has no string \"stdin\"".to_string())
        });
        let event = bytes
            .as_ref()
            .ok()
            .map_or_else(String::new, |b| event_name(b));
        out.push(Captured {
            name,
            event,
            registered,
            bytes,
        });
    }
    out
}

fn event_name(stdin: &[u8]) -> String {
    serde_json::from_slice::<serde_json::Value>(stdin)
        .ok()
        .and_then(|v| {
            v.get("hook_event_name")
                .and_then(|e| e.as_str().map(str::to_owned))
        })
        .unwrap_or_default()
}

/// `{"kind": ok|violation|malformed, "event", "field"}` for one payload's contract classification.
fn classification_json(c: Classification) -> serde_json::Value {
    match c {
        Classification::Ok { event } => {
            serde_json::json!({"kind": "ok", "event": event, "field": null})
        }
        Classification::Violation { event, field } => {
            serde_json::json!({"kind": "violation", "event": event, "field": field})
        }
        Classification::Malformed(_) => {
            serde_json::json!({"kind": "malformed", "event": null, "field": null})
        }
    }
}

/// Replay core payloads under the operational contract. A separate historical
/// diagnostic classifies explicit probe metadata; it never supplies runtime
/// attribution or excuses a core observation/payload failure.
fn run_capture(dir: &Path) -> CaptureRun {
    let dir = dir.canonicalize().expect("capture dir exists");
    let probe = read_probe(&dir).expect("canary-probe.json");
    let help = ["codex", "codex-exec"].map(|n| read_help(&dir, n).is_some());
    let mut failures = Vec::new();
    let mut payloads = Vec::new();
    let mut observation = "ok".to_string();
    let binary = if probe.binary.is_absolute() {
        probe.binary.clone()
    } else {
        dir.join(&probe.binary)
    };
    let harness = harness_of(&probe.harness);
    let bin_dir = binary.parent().map(|p| p.as_os_str().to_owned());
    match harness {
        None => failures.push(format!("unknown harness {:?}", probe.harness)),
        Some(harness) => {
            match crate::cli::hook::observe_harness_in(
                harness,
                bin_dir.as_deref(),
                std::time::Duration::from_secs(15),
                None,
            ) {
                Err(error) => {
                    observation = format!("refused: {error}");
                    failures.push(format!("operational observation refused: {error}"));
                }
                Ok(installed) => {
                    for captured in read_payloads(&dir, &probe.harness) {
                        let Captured {
                            name: file,
                            event,
                            registered,
                            bytes,
                        } = captured;
                        let contract = match (&bytes, contract_for(&probe.harness)) {
                            (Ok(b), Some(c)) => {
                                classification_json(classify(c, registered.as_deref(), b))
                            }
                            _ => classification_json(Classification::Malformed(Malformed::NotJson)),
                        };
                        let result = bytes.and_then(|b| {
                            crate::cli::hook::parse_event(&installed, &b)
                                .map(|_| ())
                                .map_err(|e| format!("{e:?}"))
                        });
                        if let Err(error) = &result {
                            failures.push(format!("{file}: {error}"));
                        }
                        payloads.push(serde_json::json!({
                            "file": file, "event": event,
                            "ok": result.is_ok(), "error": result.err(),
                            "contract": contract}));
                    }
                }
            }
        }
    }
    let (schema, launch_tables) = if probe.harness == "codex" {
        (
            Some(codex_schema_report(&binary)),
            launch_tables_report(
                read_help(&dir, "codex").as_deref(),
                read_help(&dir, "codex-exec").as_deref(),
            ),
        )
    } else {
        (None, None)
    };
    let historical = match harness {
        Some(crate::harness::context::Harness::Claude) => Some(recipe_history(
            crate::harness::claude::admission_table(),
            &probe,
            None,
        )),
        Some(crate::harness::context::Harness::Codex) => Some(recipe_history(
            crate::harness::codex::admission_table(),
            &probe,
            schema.as_ref().and_then(|schema| {
                (schema["result"] == "match")
                    .then(|| schema["recipe"].as_str())
                    .flatten()
            }),
        )),
        _ => None,
    };
    if let Some(diagnostic) = &historical
        && diagnostic["classification"] == "refused"
        && historical_refusal_is_failure(probe.expected_admission)
    {
        failures.push(format!(
            "historical recipe diagnostic refused: {} (explicit probe version {})",
            diagnostic["reason"], probe.version
        ));
    }
    let contract_id = harness.and_then(|_| contract_for(&probe.harness).map(contract_id));
    let report = serde_json::json!({
        "probe": {"harness": probe.harness, "version": probe.version, "binary": binary},
        "contract_id": contract_id,
        "observation": observation, "observation_scope": "operational_contract",
        "runtime_version": null, "historical_diagnostic": historical, "payloads": payloads,
        "schema": schema, "launch_tables": launch_tables, "help_present": help});
    CaptureRun { report, failures }
}

/// Source-declared recipe history for the explicitly captured probe version.
/// This never observes an executable or qualifies a current runtime/native
/// capability. Schema matching uses only the separate already-captured report.
fn recipe_history<P: 'static>(
    table: &[crate::harness::recipe::Recipe<P>],
    probe: &CanaryProbe,
    schema_recipe: Option<&str>,
) -> serde_json::Value {
    use crate::harness::admission::{Row, classify};
    let (classification, reason) = match classify(table, &probe.version, || {
        table.iter().find(|recipe| Some(recipe.id) == schema_recipe)
    }) {
        Row::Refused(reason) => ("refused", Some(format!("{reason:?}"))),
        Row::Listed(_) => ("listed", None),
        Row::SchemaMatched(_) => ("schema-matched, live-unverified", None),
        Row::Optimistic { .. } => ("optimistic", None),
    };
    serde_json::json!({
        "scope": "recipe_history_only", "version_source": "canary-probe.json.version",
        "version": probe.version, "classification": classification,
        "expected_admission": probe.expected_admission, "reason": reason,
        "runtime_qualified": false, "native_qualified": false,
    })
}

/// The production `codex_schema` fingerprint of `binary` against every recipe's
/// `schema_fingerprint` (§D3 `t0.schema`): `match` (equals a recipe's), `drift` (differs: the
/// leading signal that an adapter is needed) or `unextractable`. Reported, never asserted.
fn codex_schema_report(binary: &Path) -> serde_json::Value {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(120);
    match crate::harness::codex_schema::fingerprint_binary(binary, deadline, None) {
        Ok(measured) => {
            let recipe = crate::harness::codex::admission_table()
                .iter()
                .find(|recipe| recipe.profile.schema_fingerprint == measured.fingerprint);
            serde_json::json!({
                "result": if recipe.is_some() { "match" } else { "drift" },
                "fingerprint": measured.fingerprint,
                "recipe": recipe.map(|recipe| recipe.id),
                "recipe_fingerprints": crate::harness::codex::admission_table()
                    .iter()
                    .map(|recipe| recipe.profile.schema_fingerprint)
                    .collect::<Vec<_>>(),
            })
        }
        Err(reason) => {
            serde_json::json!({"result": "unextractable", "reason": format!("{reason:?}")})
        }
    }
}

/// Names (and `[aliases: ...]`) listed in a clap help text's `Commands:` section. An entry starts
/// at two spaces of indent; deeper lines continue its description.
fn help_commands(help: &str) -> Vec<String> {
    let mut names = Vec::new();
    let mut entry = String::new();
    let flush = |entry: &mut String, names: &mut Vec<String>| {
        let mut words = entry.split_whitespace();
        if let Some(name) = words.next() {
            names.push(name.to_owned());
        }
        if let Some((_, rest)) = entry.split_once("[aliases:")
            && let Some((aliases, _)) = rest.split_once(']')
        {
            names.extend(aliases.split(',').map(|a| a.trim().to_owned()));
        }
        entry.clear();
    };
    let mut in_section = false;
    for line in help.lines() {
        if line.trim_end() == "Commands:" {
            in_section = true;
        } else if in_section && line.trim().is_empty() {
            break;
        } else if in_section && line.starts_with("  ") && !line.starts_with("   ") {
            flush(&mut entry, &mut names);
            entry.push_str(line.trim());
        } else if in_section {
            entry.push(' ');
            entry.push_str(line.trim());
        }
    }
    flush(&mut entry, &mut names);
    names
}

/// Every name (short and long) of a value-taking option in a clap help text: an option line
/// (indent of at most six, starting with `-`) whose spec before the description carries `<...>`.
fn help_value_options(help: &str) -> Vec<String> {
    let mut names = Vec::new();
    for line in help.lines() {
        let indent = line.len() - line.trim_start().len();
        let spec = line.trim();
        if indent == 0 || indent > 6 || !spec.starts_with('-') {
            continue;
        }
        let spec = spec.split("  ").next().unwrap_or(spec);
        if !spec.contains('<') {
            continue;
        }
        names.extend(
            spec.split(", ")
                .filter_map(|part| part.split_whitespace().next())
                .filter(|name| name.starts_with('-'))
                .map(str::to_owned),
        );
    }
    names
}

/// `launch_tables` (§D3 `t0.launch-tables`, E10): top-level subcommands/aliases of `codex --help`
/// and `exec` subcommands of `codex exec --help` absent from the tables `launch.rs` knows, and
/// value-taking options of either help absent from `CODEX_VALUE_OPTIONS`. `None` when no help text
/// was captured. Reported, never asserted.
fn launch_tables_report(help: Option<&str>, exec_help: Option<&str>) -> Option<serde_json::Value> {
    use crate::harness::launch::{
        CODEX_EXEC_UNSUPPORTED_SUBCOMMANDS, CODEX_UNSUPPORTED_SUBCOMMANDS, CODEX_VALUE_OPTIONS,
    };
    if help.is_none() && exec_help.is_none() {
        return None;
    }
    let unknown = |names: Vec<String>, known: &[&str], handled: &[&str]| -> Vec<String> {
        let mut out: Vec<String> = names
            .into_iter()
            .filter(|n| !known.contains(&n.as_str()) && !handled.contains(&n.as_str()))
            .collect();
        out.sort();
        out.dedup();
        out
    };
    let subcommands = unknown(
        help.map(help_commands).unwrap_or_default(),
        CODEX_UNSUPPORTED_SUBCOMMANDS,
        &["exec", "resume"],
    );
    let exec_subcommands = unknown(
        exec_help.map(help_commands).unwrap_or_default(),
        CODEX_EXEC_UNSUPPORTED_SUBCOMMANDS,
        &["resume"],
    );
    let options = unknown(
        help.into_iter()
            .chain(exec_help)
            .flat_map(help_value_options)
            .collect(),
        CODEX_VALUE_OPTIONS,
        &[],
    );
    Some(serde_json::json!({
        "drift": !(subcommands.is_empty() && exec_subcommands.is_empty() && options.is_empty()),
        "unknown_subcommands": subcommands,
        "unknown_exec_subcommands": exec_subcommands,
        "unknown_value_options": options,
    }))
}

const HELP_SAMPLE: &str = "Codex CLI

Usage: codex [OPTIONS] [PROMPT]

Commands:
  exec     Run Codex non-interactively [aliases: e]
  apply    Apply the latest diff produced by Codex agent as a `git apply` to your local
           working tree [aliases: a]
  teleport Brand new subcommand [aliases: tp]
  help     Print this message

Options:
  -c, --config <key=value>
          Override a configuration value.

      --strict-config
          Error out when config.toml contains fields that are not recognized

  -i, --image <FILE>...
          Optional image(s)

      --new-value <THING>
          Not in CODEX_VALUE_OPTIONS
  -n, --nick <X>
          Short and long both new
";

/// The help parsers pick up exactly the commands, aliases and value options launch.rs does not
/// know (and none of those it does), including an alias on a wrapped description line.
#[test]
fn launch_tables_report_names_only_the_drift() {
    assert_eq!(
        help_commands(HELP_SAMPLE),
        ["exec", "e", "apply", "a", "teleport", "tp", "help"]
    );
    assert_eq!(
        help_value_options(HELP_SAMPLE),
        [
            "-c",
            "--config",
            "-i",
            "--image",
            "--new-value",
            "-n",
            "--nick"
        ]
    );
    let report = launch_tables_report(
        Some(HELP_SAMPLE),
        Some("Commands:\n  resume  r\n  bogus  b\n"),
    )
    .expect("help present");
    assert_eq!(report["drift"], true);
    assert_eq!(
        report["unknown_subcommands"],
        serde_json::json!(["teleport", "tp"])
    );
    assert_eq!(
        report["unknown_exec_subcommands"],
        serde_json::json!(["bogus"])
    );
    assert_eq!(
        report["unknown_value_options"],
        serde_json::json!(["--new-value", "--nick", "-n"])
    );
    let clean =
        launch_tables_report(Some("Commands:\n  exec  x [aliases: e]\n  help  h\n"), None).unwrap();
    assert_eq!(clean["drift"], false);
    assert!(launch_tables_report(None, None).is_none());
}

/// Every flag the Rust launch/setup code emits has a row in `scripts/canary/launch-flags.tsv`
/// (§D4), so the list cannot silently lag the code. Composed through the production launch rule
/// and the production setup plans, not from a hand-kept list. `-c` and `--config` are one option.
#[test]
fn every_emitted_launch_flag_has_a_tsv_row() {
    use crate::harness::{codex::InstalledVersion, setup};
    let tsv = std::fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("scripts/canary/launch-flags.tsv"),
    )
    .expect("launch-flags.tsv");
    let rows: Vec<(&str, &str)> = tsv
        .lines()
        .filter(|l| !l.trim().is_empty() && !l.starts_with('#'))
        .map(|l| {
            let cols: Vec<&str> = l.split('\t').collect();
            assert_eq!(cols.len(), 4, "harness, help-command, flag, source: {l:?}");
            (cols[0], cols[2])
        })
        .collect();
    let has_row = |harness: &str, flag: &str| {
        let wanted: &[&str] = if flag == "-c" {
            &["-c", "--config"]
        } else {
            &[flag]
        };
        rows.iter()
            .any(|(h, f)| *h == harness && wanted.contains(f))
    };

    let codex_plan = setup::plan_codex_for_version(
        &[],
        &["/canary/herdr-threads".to_owned()],
        &InstalledVersion::pinned_for_test(),
    )
    .unwrap();
    let mut emitted: Vec<(&str, String)> = Vec::new();
    for caller in [
        &[][..],
        &["prompt"],
        &["exec", "prompt"],
        &["exec", "resume", "id", "prompt"],
        // The top-level `resume` form is refused at launch until a live
        // capture shows it loading the owned hooks (main, ht-zp0 follow-up),
        // so it emits no flags.
    ] {
        let caller = caller.iter().map(|a| (*a).to_owned()).collect();
        let argv = codex_plan.launch_argv_for(caller, false).unwrap();
        emitted.extend(
            argv.into_iter()
                .filter(|a| a.starts_with('-'))
                .map(|a| ("codex", a)),
        );
    }
    let claude_plan = setup::plan_claude(b"{}", &["/canary/herdr-threads".to_owned()]).unwrap();
    emitted.extend(
        claude_plan
            .launch_argv("/canary/settings.json")
            .unwrap()
            .into_iter()
            .filter(|a| a.starts_with('-'))
            .map(|a| ("claude", a)),
    );

    let flags: std::collections::BTreeSet<_> = emitted.iter().map(|(_, f)| f.as_str()).collect();
    assert!(
        ["--no-daemon", "-c", "--settings"]
            .iter()
            .all(|f| flags.contains(f)),
        "the launch/setup code no longer emits one of --no-daemon, -c, --settings: {flags:?}"
    );
    let missing: Vec<_> = emitted
        .iter()
        .filter(|(harness, flag)| !has_row(harness, flag))
        .collect();
    assert!(
        missing.is_empty(),
        "emitted flags with no launch-flags.tsv row: {missing:?}"
    );
}

#[test]
fn gated_canary_payloads() {
    let Some(dir) = std::env::var_os("HT_CANARY_CAPTURE_DIR") else {
        return;
    };
    let dir = PathBuf::from(dir);
    let run = run_capture(&dir);
    std::fs::write(
        dir.join("canary-rust.json"),
        serde_json::to_vec_pretty(&run.report).unwrap(),
    )
    .unwrap();
    assert!(
        run.failures.is_empty(),
        "canary payload failures: {:#?}",
        run.failures
    );
}

/// The committed fixture capture directory parses cleanly (ungated, writes nothing).
#[test]
fn committed_canary_fixture_parses() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/harness/testdata/canary");
    let run = run_capture(&dir);
    assert!(run.failures.is_empty(), "{:#?}", run.failures);
    let payloads = run.report["payloads"].as_array().unwrap();
    assert_eq!(payloads.len(), 2, "tier0 + tier1 payload: {payloads:#?}");
    assert!(
        payloads
            .iter()
            .all(|p| p["event"] == "SessionStart" && p["ok"] == true)
    );
}

/// A payload the adapter rejects is reported per payload and fails the run.
#[test]
fn unparsable_payload_fails_the_run() {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/harness/testdata/canary");
    let tmp = std::env::temp_dir().join(format!("canary-bad-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(tmp.join("capture/tier0")).unwrap();
    std::fs::create_dir_all(tmp.join("bin")).unwrap();
    std::fs::copy(src.join("canary-probe.json"), tmp.join("canary-probe.json")).unwrap();
    std::fs::copy(src.join("bin/claude"), tmp.join("bin/claude")).unwrap();
    let bogus = b"{\"hook_event_name\":\"Bogus\"}";
    std::fs::write(tmp.join("capture/tier0/1.stdin"), bogus).unwrap();
    std::fs::write(
        tmp.join("capture/tier0/1.argv"),
        "--state-dir\n/s\nhook\nclaude\n",
    )
    .unwrap();
    // The same unparsable stdin recorded for other invocations: no .argv, another subcommand,
    // another harness. None of them is a hook payload of this probe, so none is parsed.
    std::fs::write(tmp.join("capture/tier0/2.stdin"), bogus).unwrap();
    std::fs::write(tmp.join("capture/tier0/3.stdin"), bogus).unwrap();
    std::fs::write(tmp.join("capture/tier0/3.argv"), "unsetup\nclaude\n").unwrap();
    std::fs::write(tmp.join("capture/tier0/4.stdin"), bogus).unwrap();
    std::fs::write(tmp.join("capture/tier0/4.argv"), "hook\ncodex\n").unwrap();
    let run = run_capture(&tmp);
    let _ = std::fs::remove_dir_all(&tmp);
    assert_eq!(run.failures.len(), 1, "{:#?}", run.failures);
    assert_eq!(run.report["payloads"].as_array().unwrap().len(), 1);
    assert_eq!(run.report["payloads"][0]["file"], "capture/tier0/1.stdin");
    assert_eq!(run.report["payloads"][0]["ok"], false);
}

/// Per-payload contract classification (reported, never a failure): a payload missing a required
/// field under its registered `--event` is a `violation` naming the event and field, a truncated
/// one is `malformed`, a good one is `ok`, and the report carries the harness's `contract_id`.
#[test]
fn planted_capture_reports_contract_classification() {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/harness/testdata/canary");
    let tmp = std::env::temp_dir().join(format!("canary-contract-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(tmp.join("capture/tier0")).unwrap();
    std::fs::create_dir_all(tmp.join("bin")).unwrap();
    std::fs::copy(src.join("canary-probe.json"), tmp.join("canary-probe.json")).unwrap();
    std::fs::copy(src.join("bin/claude"), tmp.join("bin/claude")).unwrap();
    let plant = |n: u32, stdin: &str| {
        std::fs::write(tmp.join(format!("capture/tier0/{n}.stdin")), stdin).unwrap();
        std::fs::write(
            tmp.join(format!("capture/tier0/{n}.argv")),
            "--state-dir\n/s\nhook\nclaude\n--event\nSessionStart\n",
        )
        .unwrap();
    };
    plant(
        1,
        r#"{"hook_event_name":"SessionStart","source":"startup"}"#,
    );
    plant(2, r#"{"hook_event_name":"SessionStart","source":"sta"#);
    plant(
        3,
        r#"{"hook_event_name":"SessionStart","source":"startup","session_id":"s"}"#,
    );
    let run = run_capture(&tmp);
    let _ = std::fs::remove_dir_all(&tmp);
    let payloads = run.report["payloads"].as_array().unwrap();
    assert_eq!(payloads.len(), 3, "{payloads:#?}");
    assert_eq!(
        payloads[0]["contract"],
        serde_json::json!({"kind": "violation", "event": "SessionStart", "field": "session_id"})
    );
    assert_eq!(
        payloads[1]["contract"],
        serde_json::json!({"kind": "malformed", "event": null, "field": null})
    );
    assert_eq!(
        payloads[2]["contract"],
        serde_json::json!({"kind": "ok", "event": "SessionStart", "field": null})
    );
    assert_eq!(
        run.report["contract_id"],
        contract_id(contract_for("claude").unwrap())
    );
}
