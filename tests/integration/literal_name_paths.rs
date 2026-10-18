//! Literal slash labels through the real CLI and a private, test-owned Herdr server.
use super::session_bindings_smoke::Smoke;

#[test]
fn literal_name_paths_preserve_legacy_nickname_precedence() {
    let Some(mut smoke) = Smoke::new("literal-name-paths-legacy") else {
        return;
    };
    let pane = smoke.pane();
    let other = smoke.pane();
    let seat = smoke.seat(&pane);
    for prefix in [r"path:legacy\q", "path:legacy"] {
        smoke.herdr_api(&["pane", "rename", &pane, prefix]);
        let nickname = format!("{prefix}/{seat}");
        smoke.herdr_api(&["pane", "rename", &other, &nickname]);
        let selected = smoke.human(None, &["seat", "inspect", &nickname]);
        assert_eq!(selected.data()["summary"]["seat"], seat);
    }
}

#[test]
fn literal_name_paths_real_labels() {
    let Some(smoke) = Smoke::new("literal-name-paths") else {
        return;
    };
    let created = smoke.herdr_api(&[
        "workspace",
        "create",
        "--label",
        "prj/herdr-threads",
        "--no-focus",
        "--cwd",
        smoke.cwd.to_str().unwrap(),
    ]);
    let pane = created["root_pane"]["pane_id"].as_str().unwrap();
    // Read the actual children; labels assigned by Herdr are never assumed.
    let original = smoke.herdr_api(&["pane", "get", pane]);
    println!("original pane: {original}");
    let tab = original["pane"]["tab_id"].as_str().unwrap();
    let space = original["pane"]["workspace_id"].as_str().unwrap();
    smoke.herdr_api(&["tab", "rename", tab, "tab/main"]);
    smoke.herdr_api(&["pane", "rename", pane, "pane/main"]);
    for (kind, id, label) in [
        ("workspace", space, "prj/herdr-threads"),
        ("tab", tab, "tab/main"),
        ("pane", pane, "pane/main"),
    ] {
        let got = smoke.herdr_api(&[kind, "get", id]);
        println!("verified {kind} {id}: {got}");
        assert_eq!(got[kind]["label"], label, "{got}");
    }
    let seat = smoke.seat(pane);
    let flags = smoke
        .human(
            None,
            &[
                "seat",
                "resolve",
                "--space",
                "prj/herdr-threads",
                "--tab",
                "tab/main",
                "--pane",
                "pane/main",
            ],
        )
        .data();
    assert_eq!(flags, seat);
    for selector in [&seat, pane] {
        assert_eq!(
            smoke.human(None, &["seat", "inspect", selector]).data()["summary"]["seat"],
            seat
        );
    }
    let raw = smoke.human(
        None,
        &["seat", "inspect", "prj/herdr-threads/tab/main/pane/main"],
    );
    println!("raw selector: exit={} {}", raw.code, raw.text());
    assert_ne!(raw.code, 0);
    let selected = smoke.human(
        None,
        &[
            "seat",
            "inspect",
            r"path:prj\/herdr-threads/tab\/main/pane\/main",
        ],
    );
    println!("explicit path: exit={} {}", selected.code, selected.text());
    assert_eq!(selected.data()["summary"]["seat"], seat);
    let nickname = format!(r"path:prj\/herdr-threads/tab\/main/{seat}");
    assert_eq!(
        smoke.human(None, &["seat", "inspect", &nickname]).data()["summary"]["seat"],
        seat
    );
    let full_nickname = format!(r"path:prj\/herdr-threads/tab\/main/pane\/main/{seat}");
    assert_eq!(
        smoke
            .human(None, &["seat", "inspect", &full_nickname])
            .data()["summary"]["seat"],
        seat
    );
}
