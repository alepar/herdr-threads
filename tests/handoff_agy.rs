//! `handoff --kind agy` through the built binary and the owned fake host of `handoff_topology_cli`:
//! the plugin installed by `setup agy` lets the V2 plan compose and start, and without it the
//! handoff is refused with the setup hint before any host effect.
use super::handoff_topology_cli::{BODY, Fixture, successful};

fn agy_args<'a>(_f: &'a Fixture, binary: &'a str) -> Vec<&'a str> {
    vec![
        "handoff",
        "--pane",
        "w4:p2",
        "--new-thread",
        "--topic",
        "Private smoke",
        "--goal",
        "Exact durable work",
        "--kind",
        "agy",
        "--harness-binary",
        binary,
        "--agent-arg=--model",
        "--agent-arg=gemini-3",
        "--",
        BODY,
    ]
}

#[test]
fn handoff_kind_agy_composes_through_bootstrap_v2_and_starts() {
    let f = Fixture::new();
    f.setup("agy");
    assert!(
        f.iso
            .home()
            .join(".gemini/config/plugins/herdr-threads")
            .is_dir(),
        "setup agy installs the owned plugin under the isolated HOME"
    );
    let binary = f.iso.path("agy");
    let binary = binary.to_str().unwrap();
    let out = successful(f.run(&agy_args(&f, binary), ""));
    assert_eq!(out["handoff"]["outcome"], "started");

    let starts = f.host.requests("agent.start");
    assert_eq!(starts.len(), 1);
    assert_eq!(starts[0]["params"]["kind"], "agy");
    assert_eq!(starts[0]["params"]["pane_id"], "w4:p2");
    let argv: Vec<String> = serde_json::from_value(starts[0]["params"]["args"].clone()).unwrap();
    // Caller flags first, then the V2 startup input as the value of `--prompt-interactive`.
    assert_eq!(&argv[..3], ["--model", "gemini-3", "--prompt-interactive"]);
    assert_eq!(argv.len(), 4, "{argv:?}");
    let bootstrap = &argv[3];
    assert!(bootstrap.contains(&f.instance.to_string()));
    assert!(!bootstrap.contains(BODY), "the work body is not in argv");
    // The owned fake host runs the stand-in agy with this exact argv and asserts it echoed back.
    assert_eq!(f.host.state.lock().unwrap().helpers.len(), 1);

    f.assert_work(
        out["handoff"]["seat"].as_str().unwrap(),
        out["handoff"]["thread"].as_str().unwrap(),
    );
    assert_eq!(
        f.effect_counts(),
        (0, 1, 1, 1),
        "one start, one invitation, one message"
    );
}

#[test]
fn handoff_kind_agy_without_the_plugin_is_refused_with_the_setup_hint() {
    let f = Fixture::new();
    let binary = f.iso.path("agy");
    let binary = binary.to_str().unwrap();
    let out = f.run(&agy_args(&f, binary), "");
    assert!(!out.status.success(), "handoff must refuse without setup");
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(text.contains("herdr-threads setup agy"), "{text}");
    assert_eq!(f.effect_counts(), (0, 0, 0, 0), "no host or store effect");
}
