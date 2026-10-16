use super::*;

// Each pair is generated from the JS `frame()` by scripts/gen-frame-goldens.mjs.
macro_rules! golden {
    ($name:literal) => {
        (
            $name,
            include_str!(concat!("../fixtures/delivery/frame-", $name, ".json")),
            include_str!(concat!("../fixtures/delivery/frame-", $name, ".txt")),
        )
    };
}

const GOLDENS: [(&str, &str, &str); 8] = [
    golden!("empty"),
    golden!("multiline"),
    golden!("controls"),
    golden!("name-equals-id"),
    golden!("markers"),
    golden!("attention"),
    golden!("priority"),
    golden!("tagged-ids"),
];

fn items(json: &str) -> Vec<FrameItem> {
    serde_json::from_str(json).unwrap()
}

#[test]
fn frame_matches_the_js_golden_byte_for_byte() {
    for (name, json, txt) in GOLDENS {
        let got = frame(&items(json));
        assert_eq!(got, txt, "case {name}");
        assert!(!got.starts_with('/'), "case {name}");
        assert!(got.starts_with(HEADER), "case {name}");
    }
}

#[test]
fn header_equals_the_js_header_string() {
    assert_eq!(
        HEADER,
        include_str!("../fixtures/delivery/frame-header.txt")
    );
    assert_eq!(frame(&[]), format!("{HEADER}\n\n"));
}

#[test]
fn goldens_exercise_the_cases_they_name() {
    let by_name = |wanted: &str| GOLDENS.iter().find(|(name, ..)| *name == wanted).unwrap().2;
    // CRLF, U+2028 and a lone CR each break a line; the empty line stays empty.
    assert!(
        by_name("multiline")
            .contains("  first\n  second\n  third\n\n  fifth after an empty line\n  sixth\n")
    );
    // A name equal to its id is not repeated; a different name is quoted.
    assert!(by_name("name-equals-id").contains("message m1 in T1 from S1:\n"));
    assert!(by_name("name-equals-id").contains("in T1 \"T1 \" from S1:"));
    // All three markers in the fixed order; an unknown intent adds none.
    assert!(
        by_name("markers")
            .contains("from S1 \"alice\" [human] [relays user] [rule]:\n  all markers")
    );
    assert!(by_name("markers").contains("[query]:") && by_name("markers").contains("[request]:"));
    assert!(by_name("markers").contains("unknown intent adds no marker"));
    // Tagged and direct-thread ids appear unchanged in the header line.
    assert!(by_name("tagged-ids").contains(
        "[herdr-threads] message mQ7aAb12Cd34 in tQ7aZz99Yy88 \"plans\" from sQ7aK3Fq9a2B \"alice\":\n"
    ));
    assert!(by_name("tagged-ids").contains(
        "[herdr-threads] message mQ7aEf56Gh78 in dK3Fq9a2B7x \"plans\" from sQ7aK3Fq9a2B \"bob\" [p0]:\n"
    ));
    // The short attention header has no thread or sender.
    assert!(
        by_name("attention").contains("\n\n[herdr-threads] attention attention:4:\n  You have")
    );
}

#[test]
fn priority_marker_leads_the_markers_and_p1_or_absent_is_silent() {
    let text = GOLDENS
        .iter()
        .find(|(name, ..)| *name == "priority")
        .unwrap()
        .2;
    assert!(text.contains("from S1 \"alice\" [p0]:\n  body of m1"));
    assert!(text.contains("from S1 \"alice\":\n  body of m2"));
    assert!(text.contains("from S1 \"alice\" [p5]:\n  body of m3"));
    assert!(text.contains("[p0] [human] [relays user] [rule]:\n  body of m4"));
    assert!(text.contains("from S1 \"alice\":\n  body of m5"));
    assert!(!text.contains("lazy"));
    // An out-of-range priority from a future daemon adds nothing.
    let mut item = items(include_str!("../fixtures/delivery/frame-priority.json")).remove(0);
    item.priority = Some(10);
    assert!(!frame(&[item]).contains("[p"));
}

#[test]
fn one_line_replaces_every_control_and_separator() {
    assert_eq!(
        one_line("a\u{0}b\u{1f}c\u{7f}d\u{9f}e\u{2028}f\u{2029}g"),
        "a b c d e f g"
    );
    assert_eq!(one_line("é\u{a0}\u{a1}\u{3000}"), "é\u{a0}\u{a1}\u{3000}");
}

#[test]
fn indent_breaks_on_the_js_line_break_set() {
    assert_eq!(indent(""), "");
    assert_eq!(indent("a\r\nb"), "  a\n  b");
    // \r\n is one break; a bare \r then \n is two.
    assert_eq!(indent("a\r\n\r\nb"), "  a\n\n  b");
    assert_eq!(indent("a\r\rb"), "  a\n\n  b");
    assert_eq!(
        indent("a\u{b}b\u{c}c\u{85}d\u{2028}e\u{2029}f\ng"),
        "  a\n  b\n  c\n  d\n  e\n  f\n  g"
    );
    assert_eq!(indent("a\n"), "  a\n");
}

#[test]
fn label_quotes_the_name_after_the_id() {
    assert_eq!(label(Some("T1"), Some("plans")), "T1 \"plans\"");
    assert_eq!(label(Some("T1"), Some("T1")), "T1");
    assert_eq!(label(Some("T1"), None), "T1");
    assert_eq!(label(None, Some("plans")), "\"plans\"");
    assert_eq!(label(Some(""), Some("")), "unknown");
    assert_eq!(
        label(Some("T1"), Some("say \"hi\" \\")),
        "T1 \"say \\\"hi\\\" \\\\\""
    );
}

#[test]
fn the_mod_test_copy_equals_the_fixtures() {
    // The mod's own test imports one module from inside its folder; it must hold the
    // same inputs and outputs as the fixtures asserted above.
    let module = include_str!("../../integrations/claude/mod/tests/fixtures/frame-goldens.ts");
    let json = module.strip_prefix("export default ").unwrap();
    let copy: serde_json::Value = serde_json::from_str(json).unwrap();
    assert_eq!(
        copy["header"].as_str().unwrap(),
        include_str!("../fixtures/delivery/frame-header.txt")
    );
    assert_eq!(copy.as_object().unwrap().len(), GOLDENS.len() + 1);
    for (name, json, txt) in GOLDENS {
        let items: serde_json::Value = serde_json::from_str(json).unwrap();
        assert_eq!(copy[name]["items"], items, "items of {name}");
        assert_eq!(copy[name]["text"].as_str().unwrap(), txt, "text of {name}");
    }
}
