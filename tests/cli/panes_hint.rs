//! `pane_not_found` names the precedence, and the autodetect probe is bounded
//! (root §B10 Decision 3). Mounted from `src/cli/panes.rs`.

use super::*;
use crate::cli::setup::{
    AUTODETECT_PROBE_TIMEOUT, DetectInputs, PROBE_TIMEOUT, detect_host_endpoint, detect_state_dir,
};
use crate::protocol::results::ErrorCode;
use std::{
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

fn pane(id: &str, label: Option<&str>) -> PaneName {
    PaneName {
        target: HostTargetId::new(id),
        label: label.map(str::to_owned),
        tab_label: None,
        tab_pane_count: 2,
    }
}

/// Kills: a `not_found` that only says the name is unknown, leaving a person
/// who typed a tab label for a multi-pane tab (or an ID for the wrong
/// workspace) to guess which of the three lookups they missed.
#[test]
fn pane_not_found_hint_names_the_precedence() {
    let error = resolve_pane_name("nope", &[pane("w1:p1", Some("lead"))]).unwrap_err();
    assert_eq!(error.code, ErrorCode::NotFound);
    assert!(
        error.detail.contains(NAME_PRECEDENCE),
        "the precedence must be named: {}",
        error.detail
    );
    let id = error.detail.find("pane ID").expect("names the ID rule");
    let label = error
        .detail
        .find("pane label")
        .expect("names the label rule");
    let tab = error
        .detail
        .find("exactly one pane")
        .expect("names the tab rule");
    assert!(
        id < label && label < tab,
        "in precedence order: {}",
        error.detail
    );
    assert!(
        error.detail.contains("herdr pane current"),
        "{}",
        error.detail
    );
}

/// Kills: a label that equals another pane's ID shadowing that ID.
#[test]
fn an_exact_pane_id_beats_a_label_that_looks_like_it() {
    let panes = [pane("w1:p1", Some("w1:p2")), pane("w1:p2", Some("other"))];
    assert_eq!(
        resolve_pane_name("w1:p2", &panes).unwrap().as_str(),
        "w1:p2"
    );
}

fn hung_herdr(dir: &Path) -> PathBuf {
    let bin = dir.join("herdr");
    std::fs::write(&bin, "#!/bin/sh\nexec sleep 30\n").unwrap();
    std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).unwrap();
    bin
}

/// Kills: detection waiting the old 5 s query timeout on a Herdr that never
/// answers. A hung `herdr` must fail both probes within the short bound (plus
/// scheduling slack), with the pass-it-explicitly remedy in the message.
#[test]
fn autodetect_probe_is_bounded() {
    assert!(
        AUTODETECT_PROBE_TIMEOUT <= Duration::from_millis(2_000),
        "the probe bound is short: {AUTODETECT_PROBE_TIMEOUT:?}"
    );
    let dir = std::env::temp_dir().join(format!("ht-hung-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir(&dir).unwrap();
    let inputs = DetectInputs {
        herdr: Some(hung_herdr(&dir)),
        home: Some(dir.join("home")),
        xdg_state_home: None,
    };
    // Other unit tests run with a generous probe; this one runs with the
    // production bound.
    PROBE_TIMEOUT.with(|probe| probe.set(AUTODETECT_PROBE_TIMEOUT));
    let started = Instant::now();
    let state = detect_state_dir(&inputs).unwrap_err();
    let first = started.elapsed();
    let started = Instant::now();
    let host = detect_host_endpoint(&inputs).unwrap_err();
    let second = started.elapsed();
    std::fs::remove_dir_all(&dir).unwrap();
    assert!(
        first < Duration::from_millis(4_000) && second < Duration::from_millis(4_000),
        "hung probes took {first:?} and {second:?}, not the 5 s query timeout"
    );
    assert!(
        first >= AUTODETECT_PROBE_TIMEOUT - Duration::from_millis(100),
        "the probe gave up after {first:?}, before its {AUTODETECT_PROBE_TIMEOUT:?} bound"
    );
    assert!(state.contains("did not answer"), "{state}");
    assert!(host.contains("did not answer"), "{host}");
}

/// Kills: docs/agent-usage.md drifting from the probe bound (ht-p03.108).
#[test]
fn agent_usage_states_the_probe_bound() {
    let doc =
        std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("docs/agent-usage.md"))
            .unwrap();
    let secs = AUTODETECT_PROBE_TIMEOUT.as_secs();
    assert_eq!(
        AUTODETECT_PROBE_TIMEOUT,
        Duration::from_secs(secs),
        "whole seconds"
    );
    assert!(
        doc.contains(&format!("short probe timeout ({secs} s)")),
        "agent-usage.md does not say {secs} s"
    );
}
