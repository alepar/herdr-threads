//! Host-neutrality source scan: routing, delivery, receipts and binding never
//! learn which environment a seat lives in. Under `src/`, outside the paths in
//! `tests/host_neutral_allowlist.txt`, no code names a `HostKind` variant,
//! calls `.kind()` on a host, port or session, or spells a host env name as a
//! string literal. The matcher is a pure function on in-memory source so the
//! negative self-tests run without touching the tree. The full
//! `scripts/check-host-neutral` (G6) stays follow-on.

use std::fs;
use std::path::{Path, PathBuf};

/// Env names that only a host adapter or the caller-location reader may spell.
const ENV_NAMES: [&str; 6] = [
    "HC_TERMINAL_ID",
    "HC_TERMINAL_SOCKET",
    "TMUX",
    "TMUX_PANE",
    "HERDR_THREADS_SEAT_NAME",
    "HERDR_THREADS_SEAT_READONLY",
];

/// Modules that carry routing, delivery, receipt or binding logic: no allowlist
/// entry may cover any of them.
const NEUTRAL_PATHS: [&str; 8] = [
    "src/delivery/",
    "src/store/receipts.rs",
    "src/store/read_cursors.rs",
    "src/store/bindings.rs",
    "src/store/lazy_delivery/",
    "src/scheduler/",
    "src/notification/",
    "src/protocol/",
];

/// Paths no entry may be, or be an ancestor of (a glob-like exemption).
const NO_BROAD_ENTRY: [&str; 5] = [
    "src/store/",
    "src/delivery/",
    "src/service/dispatch.rs",
    "src/scheduler/",
    "src/notification/",
];

/// True when `entry_path` (file, or directory ending in `/`) covers `target`
/// (a file, or a directory ending in `/`).
fn covers(entry_path: &str, target: &str) -> bool {
    entry_path == target || (entry_path.ends_with('/') && target.starts_with(entry_path))
}

fn is_comment(line: &str) -> bool {
    line.trim_start().starts_with("//")
}

/// A `HostKind::` use naming a variant (`Herdr`, `Unknown`, `Named`, `Hc`, ...):
/// CamelCase after the path separator. `HostKind::IMPLEMENTED` and the
/// associated fns (`from_stored`, `is_implemented`) are the permitted surface.
fn names_host_kind_variant(line: &str) -> bool {
    line.match_indices("HostKind::").any(|(at, m)| {
        let mut rest = line[at + m.len()..].chars();
        matches!((rest.next(), rest.next()), (Some(a), Some(b)) if a.is_ascii_uppercase() && b.is_ascii_lowercase())
    })
}

/// A `.kind()` call whose receiver chain (identifiers and dots immediately
/// before it) has a segment that is `host`, `hosts`, `port`, `hostport` or
/// `session`, split on `.` and `_`: `self.port.kind()`, `host.kind()`,
/// `session.kind()`, `host_port.kind()`. `error.kind()` (io errors) does not
/// match.
fn calls_host_kind(line: &str) -> bool {
    line.match_indices(".kind()").any(|(at, _)| {
        let head = &line[..at];
        let start = head
            .rfind(|c: char| !(c.is_ascii_alphanumeric() || c == '_' || c == '.'))
            .map_or(0, |i| i + 1);
        head[start..]
            .split(['.', '_'])
            .any(|s| matches!(s, "host" | "hosts" | "port" | "hostport" | "session"))
    })
}

/// A string literal that is a host env name (exactly, or as a `NAME=` word)
/// or begins the `HC_` prefix family.
fn spells_host_env(line: &str) -> bool {
    line.contains("\"HC_")
        || ENV_NAMES
            .iter()
            .any(|n| line.contains(&format!("\"{n}\"")) || line.contains(&format!("\"{n}=")))
}

/// Violations in `src` as `path:line: text`. Comment lines are skipped.
fn violations(path: &str, src: &str) -> Vec<String> {
    src.lines()
        .enumerate()
        .filter(|(_, l)| !is_comment(l))
        .filter(|(_, l)| names_host_kind_variant(l) || calls_host_kind(l) || spells_host_env(l))
        .map(|(i, l)| format!("{path}:{}: {}", i + 1, l.trim()))
        .collect()
}

/// Line ranges (1-based, inclusive) of the items named `symbol` in `src`.
fn symbol_spans(src: &str, symbol: &str) -> Vec<(usize, usize)> {
    const KEYWORDS: [&str; 8] = [
        "fn", "struct", "enum", "trait", "impl", "const", "type", "mod",
    ];
    let lines: Vec<&str> = src.lines().collect();
    let mut spans = Vec::new();
    let mut i = 0;
    while i < lines.len() {
        let words: Vec<&str> = lines[i]
            .split(|c: char| !(c.is_alphanumeric() || c == '_'))
            .collect();
        if words.contains(&symbol) && words.iter().any(|w| KEYWORDS.contains(w)) {
            let (mut depth, mut seen, mut end) = (0i32, false, i);
            for (j, l) in lines.iter().enumerate().skip(i) {
                depth += l.matches('{').count() as i32 - l.matches('}').count() as i32;
                seen |= l.contains('{');
                end = j;
                if (seen && depth <= 0) || (!seen && l.trim_end().ends_with(';')) {
                    break;
                }
            }
            spans.push((i + 1, end + 1));
            i = end;
        }
        i += 1;
    }
    spans
}

#[derive(Debug, PartialEq)]
struct Entry {
    path: String,
    symbol: Option<String>,
}

/// Parse the allowlist; every error is a returned string.
fn parse_allowlist(text: &str) -> (Vec<Entry>, Vec<String>) {
    let (mut entries, mut errors) = (Vec::new(), Vec::new());
    for (n, line) in text.lines().enumerate() {
        if line.trim().is_empty() || line.starts_with('#') {
            continue;
        }
        let Some((spec, reason)) = line.split_once(" # ") else {
            errors.push(format!("line {}: no ` # reason`: {line}", n + 1));
            continue;
        };
        if reason.trim().is_empty() || spec.contains('*') || spec.contains(' ') {
            errors.push(format!(
                "line {}: empty reason, glob or space: {line}",
                n + 1
            ));
            continue;
        }
        let (path, symbol) = match spec.split_once("::") {
            Some((p, s)) => (p, Some(s.to_owned())),
            None => (spec, None),
        };
        for target in NEUTRAL_PATHS.iter().chain(&NO_BROAD_ENTRY) {
            // An entry may not be the target or an ancestor of it; a file
            // below a broad directory is a specific exemption, not a glob.
            if covers(path, target) {
                errors.push(format!(
                    "line {}: `{path}` covers neutral `{target}`",
                    n + 1
                ));
            }
        }
        for target in NEUTRAL_PATHS {
            if !target.ends_with('/') || !path.starts_with(target) {
                continue;
            }
            errors.push(format!(
                "line {}: `{path}` is inside neutral `{target}`",
                n + 1
            ));
        }
        if NEUTRAL_PATHS.contains(&path) {
            errors.push(format!("line {}: `{path}` is a neutral module", n + 1));
        }
        entries.push(Entry {
            path: path.to_owned(),
            symbol,
        });
    }
    (entries, errors)
}

/// Violations of `src` at `path` that no entry exempts.
fn unexempted(path: &str, src: &str, entries: &[Entry]) -> Vec<String> {
    let mut found = violations(path, src);
    for e in entries.iter().filter(|e| covers(&e.path, path)) {
        match &e.symbol {
            None => return Vec::new(),
            Some(sym) => {
                let spans = symbol_spans(src, sym);
                found.retain(|v| {
                    let line: usize = v[path.len() + 1..]
                        .split(':')
                        .next()
                        .unwrap()
                        .parse()
                        .unwrap();
                    !spans.iter().any(|(a, b)| (*a..=*b).contains(&line))
                });
            }
        }
    }
    found
}

fn rust_files(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            rust_files(&path, out);
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push(path);
        }
    }
}

fn allowlist_text() -> String {
    fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/host_neutral_allowlist.txt"),
    )
    .unwrap()
}

#[test]
fn matcher_reports_host_kind_in_delivery() {
    let v = violations("src/delivery/x.rs", "match k { HostKind::Hc => 1, _ => 0 }");
    assert_eq!(v.len(), 1, "{v:?}");
    assert!(v[0].starts_with("src/delivery/x.rs:1:"));
    assert_eq!(
        violations("src/service/a.rs", "let k = HostKind::Named;").len(),
        1
    );
}

#[test]
fn matcher_reports_env_literal_in_receipts() {
    let v = violations("src/store/receipts.rs", "let t = std::env::var(\"TMUX\");");
    assert_eq!(v.len(), 1, "{v:?}");
    for lit in [
        "\"TMUX_PANE\"",
        "\"HC_TERMINAL_ID\"",
        "\"HC_ANYTHING\"",
        "\"HERDR_THREADS_SEAT_NAME=x\"",
    ] {
        assert_eq!(
            violations("src/store/receipts.rs", &format!("f({lit});")).len(),
            1,
            "{lit}"
        );
    }
}

#[test]
fn matcher_reports_kind_on_host_receivers() {
    for call in [
        "self.port.kind()",
        "host.kind()",
        "session.kind()",
        "self.host_port.kind()",
        "a.hosts.kind()",
    ] {
        assert_eq!(
            violations("src/service/a.rs", &format!("if {call} == k {{}}")).len(),
            1,
            "{call}"
        );
    }
}

#[test]
fn matcher_permits_the_implemented_set_comments_and_io_errors() {
    for ok in [
        "HostKind::from_stored(&row.kind).is_implemented()",
        "HostKind::IMPLEMENTED.contains(&k)",
        "// HostKind::Hc and \"TMUX\" in a comment",
        "/// `.port.kind()` in a doc comment",
        "Err(error) if error.kind() == io::ErrorKind::NotFound => {}",
        "let s = \"a tmux hint\"; let t = \"TMUX is mentioned in prose\";",
    ] {
        assert!(violations("src/service/a.rs", ok).is_empty(), "{ok}");
    }
}

#[test]
fn allowlist_rejects_entries_over_neutral_modules() {
    for bad in [
        "src/store/ # glob over the store",
        "src/ # everything",
        "src/delivery/ # delivery",
        "src/delivery/assemble.rs # file inside neutral dir",
        "src/store/receipts.rs # receipts",
        "src/service/dispatch.rs # dispatch",
        "src/scheduler/mod.rs::wake # inside scheduler",
        "src/notification/policy.rs # inside notification",
        "src/protocol/locate.rs # inside protocol",
        "src/store/* # glob",
        "src/ports.rs",
    ] {
        assert!(!parse_allowlist(bad).1.is_empty(), "accepted: {bad}");
    }
    let (entries, errors) =
        parse_allowlist("src/ports.rs::HostPort # the trait\nsrc/host/hc/ # adapter");
    assert!(errors.is_empty(), "{errors:?}");
    assert_eq!(
        entries[0],
        Entry {
            path: "src/ports.rs".into(),
            symbol: Some("HostPort".into())
        }
    );
}

#[test]
fn symbol_entry_exempts_only_its_item() {
    let src = "fn allowed() {\n    let k = HostKind::Herdr;\n}\nfn other() {\n    let k = HostKind::Herdr;\n}\n";
    let (entries, errors) = parse_allowlist("src/x.rs::allowed # reader");
    assert!(errors.is_empty(), "{errors:?}");
    let left = unexempted("src/x.rs", src, &entries);
    assert_eq!(left.len(), 1, "{left:?}");
    assert!(left[0].starts_with("src/x.rs:5:"));
    assert!(unexempted("src/y.rs", src, &entries).len() == 2);
}

#[test]
fn mod_entry_exempts_only_its_module() {
    let src = "fn before() {\n    let k = HostKind::Herdr;\n}\n#[cfg(test)]\nmod tests {\n    fn a() {\n        let k = HostKind::Hc;\n    }\n    fn b() {\n        let k = HostKind::Hc;\n    }\n}\nfn after() {\n    let k = HostKind::Herdr;\n}\n";
    let (entries, errors) = parse_allowlist("src/x.rs::tests # fixtures");
    assert!(errors.is_empty(), "{errors:?}");
    let left = unexempted("src/x.rs", src, &entries);
    assert_eq!(left.len(), 2, "{left:?}");
    assert!(left[0].starts_with("src/x.rs:2:"), "{left:?}");
    assert!(left[1].starts_with("src/x.rs:14:"), "{left:?}");
}

#[test]
fn allowlist_is_well_formed_and_every_entry_resolves() {
    let (entries, errors) = parse_allowlist(&allowlist_text());
    assert!(errors.is_empty(), "{}", errors.join("\n"));
    assert!(!entries.is_empty());
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    for e in &entries {
        let p = root.join(&e.path);
        assert!(p.exists(), "allowlist entry {} does not exist", e.path);
        if let Some(sym) = &e.symbol {
            let src = fs::read_to_string(&p).unwrap();
            assert!(
                !symbol_spans(&src, sym).is_empty(),
                "symbol {sym} not in {}",
                e.path
            );
        }
    }
}

#[test]
fn src_names_no_host_kind_or_host_env_outside_the_allowlist() {
    let (entries, errors) = parse_allowlist(&allowlist_text());
    assert!(errors.is_empty(), "{}", errors.join("\n"));
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut files = Vec::new();
    rust_files(&root.join("src"), &mut files);
    assert!(files.len() > 100, "scan found {} sources", files.len());
    let mut offenders = Vec::new();
    for file in files {
        let rel = file
            .strip_prefix(root)
            .unwrap()
            .to_string_lossy()
            .replace('\\', "/");
        offenders.extend(unexempted(
            &rel,
            &fs::read_to_string(&file).unwrap(),
            &entries,
        ));
    }
    assert!(offenders.is_empty(), "{}", offenders.join("\n"));
}
