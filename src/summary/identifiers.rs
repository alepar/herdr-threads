//! Daemon identifier extraction (spec §5): precise patterns, per-kind quotas,
//! most-recent-wins, keyed by value. Hand-written token scanner; no regex crate.
use crate::protocol::summary::{BundleMessage, Identifier, IdentifierKind};
use std::borrow::Cow;
use std::collections::{BTreeSet, HashMap};

/// Longest identifier value, in bytes. Spec §5 calls extraction "precise rather
/// than broad"; a value this long is not a precise identifier, and it is stored
/// as `summary_items.item_id` and carried into every later fold and bundle.
pub const MAX_IDENTIFIER_BYTES: usize = 160;

/// `value` unchanged when it fits the cap, else cut at a char boundary and
/// ended with `…` so the result never exceeds `MAX_IDENTIFIER_BYTES`.
fn bounded(value: &str) -> Cow<'_, str> {
    if value.len() <= MAX_IDENTIFIER_BYTES {
        return Cow::Borrowed(value);
    }
    let mut end = MAX_IDENTIFIER_BYTES - '…'.len_utf8();
    while !value.is_char_boundary(end) {
        end -= 1;
    }
    Cow::Owned(format!("{}…", &value[..end]))
}

/// Per-block quotas (sum = `BLOCK_IDENTIFIER_CAP`).
const BLOCK_QUOTAS: [(IdentifierKind, usize); 5] = [
    (IdentifierKind::Path, 24),
    (IdentifierKind::BeadId, 16),
    (IdentifierKind::Sha, 12),
    (IdentifierKind::Url, 8),
    (IdentifierKind::Error, 4),
];
/// Fold-level quotas: double the block quotas (sum = `FOLD_IDENTIFIER_CAP`).
const FOLD_QUOTAS: [(IdentifierKind, usize); 5] = [
    (IdentifierKind::Path, 48),
    (IdentifierKind::BeadId, 32),
    (IdentifierKind::Sha, 24),
    (IdentifierKind::Url, 16),
    (IdentifierKind::Error, 8),
];

const EDGE: &[char] = &[
    '(', ')', '[', ']', '{', '}', '<', '>', ',', ';', ':', '"', '\'', '`',
];

fn trim_token(token: &str) -> &str {
    token.trim_matches(EDGE).trim_end_matches(['.', '!', '?'])
}

fn is_url(token: &str) -> bool {
    let Some((scheme, rest)) = token.split_once("://") else {
        return false;
    };
    !rest.is_empty()
        && scheme.starts_with(|c: char| c.is_ascii_alphabetic())
        && scheme
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'+' | b'-' | b'.'))
}

fn is_bead_id(token: &str, prefixes: &[String]) -> bool {
    prefixes.iter().any(|prefix| {
        let Some(rest) = token.strip_prefix(prefix.as_str()) else {
            return false;
        };
        let mut parts = rest.split('.');
        let first = parts.next().unwrap_or("");
        let ok_first = first.len() >= 2
            && first
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit());
        let ok_rest = parts.all(|p| !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()));
        ok_first && ok_rest && (rest.bytes().any(|b| b.is_ascii_digit()) || rest.len() >= 3)
    })
}

fn is_sha(token: &str) -> bool {
    (7..=40).contains(&token.len())
        && token.bytes().all(|b| b.is_ascii_hexdigit())
        && token.bytes().any(|b| b.is_ascii_digit())
        && token.bytes().any(|b| b.is_ascii_alphabetic())
}

fn is_path(token: &str) -> bool {
    if token.contains("://") {
        return false;
    }
    for lead in ["/", "./", "src/"] {
        if let Some(rest) = token.strip_prefix(lead) {
            return !rest.is_empty() && !rest.starts_with('/');
        }
    }
    if !token.contains('/') {
        return false;
    }
    let last = token.rsplit('/').next().unwrap_or("");
    match last.rsplit_once('.') {
        Some((stem, ext)) => {
            !stem.is_empty()
                && ext.starts_with(|c: char| c.is_ascii_alphabetic())
                && ext.bytes().all(|b| b.is_ascii_alphanumeric())
        }
        None => false,
    }
}

fn classify(token: &str, prefixes: &[String]) -> Option<IdentifierKind> {
    if token.is_empty() || token.len() > MAX_IDENTIFIER_BYTES {
        // An over-long path, URL, SHA or bead id is dropped, not truncated: a
        // truncated URL or path would point somewhere else.
        None
    } else if is_url(token) {
        Some(IdentifierKind::Url)
    } else if is_bead_id(token, prefixes) {
        Some(IdentifierKind::BeadId)
    } else if is_sha(token) {
        Some(IdentifierKind::Sha)
    } else if is_path(token) {
        Some(IdentifierKind::Path)
    } else {
        None
    }
}

/// Backticked spans (closed pairs only) that mention `error` or `failed`.
fn error_spans(text: &str) -> Vec<&str> {
    let parts: Vec<&str> = text.split('`').collect();
    (1..parts.len())
        .step_by(2)
        .filter(|i| i + 1 < parts.len())
        .map(|i| parts[i])
        // A span with a line break (checked before trimming) is a fenced
        // block's body, not an error line.
        .filter(|span| !span.contains(['\n', '\r']))
        .map(str::trim)
        .filter(|span| {
            let lower = span.to_ascii_lowercase();
            lower.contains("error") || lower.contains("failed")
        })
        .collect()
}

fn rank(kind: IdentifierKind) -> usize {
    match kind {
        IdentifierKind::Path => 0,
        IdentifierKind::BeadId => 1,
        IdentifierKind::Sha => 2,
        IdentifierKind::Url => 3,
        IdentifierKind::Error => 4,
    }
}

type Accumulator = HashMap<String, (IdentifierKind, BTreeSet<u64>)>;

/// Keep each kind's quota of most recently mentioned values; output sorted by
/// kind, then most recent first, then value.
fn select(acc: Accumulator, quotas: &[(IdentifierKind, usize); 5]) -> Vec<Identifier> {
    let mut all: Vec<Identifier> = acc
        .into_iter()
        .map(|(value, (kind, seqs))| Identifier {
            value,
            kind,
            seqs: seqs.into_iter().collect(),
        })
        .collect();
    let last = |i: &Identifier| i.seqs.last().copied().unwrap_or(0);
    all.sort_by(|a, b| {
        rank(a.kind)
            .cmp(&rank(b.kind))
            .then(last(b).cmp(&last(a)))
            .then(a.value.cmp(&b.value))
    });
    let mut taken = [0usize; 5];
    all.retain(|i| {
        let quota = quotas
            .iter()
            .find(|(kind, _)| *kind == i.kind)
            .map_or(0, |(_, q)| *q);
        let slot = &mut taken[rank(i.kind)];
        *slot += 1;
        *slot <= quota
    });
    all
}

/// Extract the identifiers mentioned in a chunk's messages (block quotas).
pub fn extract(messages: &[BundleMessage], tracker_prefixes: &[String]) -> Vec<Identifier> {
    let mut acc = Accumulator::new();
    let mut note = |value: &str, kind: IdentifierKind, seq: u64| {
        acc.entry(value.to_string())
            .or_insert_with(|| (kind, BTreeSet::new()))
            .1
            .insert(seq);
    };
    for message in messages {
        for raw in message.text.split_whitespace() {
            let token = trim_token(raw);
            if let Some(kind) = classify(token, tracker_prefixes) {
                note(token, kind, message.sequence);
            }
        }
        for span in error_spans(&message.text) {
            note(&bounded(span), IdentifierKind::Error, message.sequence);
        }
    }
    select(acc, &BLOCK_QUOTAS)
}

/// Merge identifier sets by value (seqs unioned) under the fold quotas.
pub fn merge(sets: &[&[Identifier]]) -> Vec<Identifier> {
    let mut acc = Accumulator::new();
    for set in sets {
        for identifier in *set {
            // Blocks stored before the bound may hold over-long values.
            acc.entry(bounded(&identifier.value).into_owned())
                .or_insert_with(|| (identifier.kind, BTreeSet::new()))
                .1
                .extend(identifier.seqs.iter().copied());
        }
    }
    select(acc, &FOLD_QUOTAS)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::{
        ids::MessageId,
        results::MessageKind,
        summary::{BLOCK_IDENTIFIER_CAP, FOLD_IDENTIFIER_CAP},
        time::UtcMillis,
    };

    fn msg(seq: u64, text: &str) -> BundleMessage {
        BundleMessage {
            sequence: seq,
            message: MessageId::new(format!("m{seq}")),
            kind: MessageKind::Ordinary,
            author: None,
            author_role: None,
            relays_user: false,
            created_at: UtcMillis(0),
            text: text.into(),
        }
    }
    fn prefixes(p: &[&str]) -> Vec<String> {
        p.iter().map(|s| s.to_string()).collect()
    }
    fn values(ids: &[Identifier], kind: IdentifierKind) -> Vec<String> {
        ids.iter()
            .filter(|i| i.kind == kind)
            .map(|i| i.value.clone())
            .collect()
    }
    fn found(text: &str, p: &[&str]) -> Vec<(IdentifierKind, String)> {
        let mut v: Vec<_> = extract(&[msg(1, text)], &prefixes(p))
            .into_iter()
            .map(|i| (i.kind, i.value))
            .collect();
        v.sort_by(|a, b| a.1.cmp(&b.1));
        v
    }

    #[test]
    fn path_patterns() {
        for yes in ["src/x.rs", "./a", "/etc/hosts", "docs/a/b.md"] {
            assert_eq!(
                found(&format!("see {yes}."), &["ht-"]),
                vec![(IdentifierKind::Path, yes.to_string())],
                "{yes}"
            );
        }
        for no in ["a/b", "x.rs", "and/or", "/", "//x"] {
            assert!(found(&format!("see {no}"), &["ht-"]).is_empty(), "{no}");
        }
        // Surrounding punctuation is stripped; a URL is not also a path.
        assert_eq!(
            found("(`src/a.rs`,", &["ht-"]),
            vec![(IdentifierKind::Path, "src/a.rs".into())]
        );
        assert_eq!(
            found("https://x.y/z.html", &["ht-"]),
            vec![(IdentifierKind::Url, "https://x.y/z.html".into())]
        );
    }

    #[test]
    fn bead_ids_use_tracker_prefixes() {
        for id in ["ht-1ip", "ht-1ip.4", "ht-abc"] {
            assert_eq!(
                found(id, &["ht-"]),
                vec![(IdentifierKind::BeadId, id.to_string())],
                "{id}"
            );
        }
        assert!(found("ht-ab", &["ht-"]).is_empty());
        assert!(found("bd-12", &["ht-"]).is_empty());
        assert_eq!(
            found("bd-12", &["ht-", "bd-"]),
            vec![(IdentifierKind::BeadId, "bd-12".into())]
        );
        // `.N` parts must be numeric.
        assert!(found("ht-1ip.x", &["ht-"]).is_empty());
    }

    #[test]
    fn shas() {
        let forty = "3d86dcfc".to_string() + &"0a".repeat(16);
        assert_eq!(forty.len(), 40);
        for sha in ["3d86dcfc", forty.as_str(), "1234567a"] {
            assert_eq!(
                found(&format!("commit {sha}"), &["ht-"]),
                vec![(IdentifierKind::Sha, sha.to_string())],
                "{sha}"
            );
        }
        let forty_one = format!("{forty}0");
        for no in ["deadbeef", "1234567", "3d86dc", "x3d86dcfcx", &forty_one] {
            assert!(found(&format!("commit {no}"), &["ht-"]).is_empty(), "{no}");
        }
    }

    #[test]
    fn urls_and_errors() {
        assert_eq!(
            found("go to https://x.y/z now", &["ht-"]),
            vec![(IdentifierKind::Url, "https://x.y/z".into())]
        );
        assert_eq!(
            found("got `build failed: E0425` here", &["ht-"]),
            vec![(IdentifierKind::Error, "build failed: E0425".into())]
        );
        assert_eq!(
            found("got `error: x` here", &["ht-"]),
            vec![(IdentifierKind::Error, "error: x".into())]
        );
        assert!(found("it is `ok` here", &["ht-"]).is_empty());
        // An unpaired backtick opens no span.
        assert!(found("a `error: never closed", &["ht-"]).is_empty());
    }

    #[test]
    fn quotas_and_recency() {
        let msgs: Vec<_> = (1..=30)
            .map(|n| msg(n, &format!("touch src/f{n}.rs")))
            .collect();
        let ids = extract(&msgs, &prefixes(&["ht-"]));
        let paths = values(&ids, IdentifierKind::Path);
        assert_eq!(paths.len(), 24);
        assert_eq!(paths[0], "src/f30.rs");
        assert_eq!(paths[23], "src/f7.rs");
        assert!(!paths.contains(&"src/f6.rs".to_string()));

        // seqs lists every mention, ascending, once per message.
        let msgs = vec![
            msg(9, "src/a.rs and src/a.rs"),
            msg(3, "src/a.rs"),
            msg(5, "x"),
        ];
        let ids = extract(&msgs, &prefixes(&["ht-"]));
        assert_eq!(ids[0].seqs, vec![3, 9]);

        // Every kind over quota: capped per kind, 64 in total.
        let mut many = Vec::new();
        for n in 1..=40u64 {
            many.push(msg(
                n,
                &format!(
                    "src/p{n}.rs ht-{n}x https://u.example/{n} 3d86dc{n:02}f `error {n}` {n}a{n}b{n}c{n}d{n}"
                ),
            ));
        }
        let ids = extract(&many, &prefixes(&["ht-"]));
        assert_eq!(values(&ids, IdentifierKind::Path).len(), 24);
        assert_eq!(values(&ids, IdentifierKind::BeadId).len(), 16);
        assert_eq!(values(&ids, IdentifierKind::Sha).len(), 12);
        assert_eq!(values(&ids, IdentifierKind::Url).len(), 8);
        assert_eq!(values(&ids, IdentifierKind::Error).len(), 4);
        assert_eq!(ids.len(), BLOCK_IDENTIFIER_CAP);
    }

    #[test]
    fn fenced_code_block_is_not_an_error_identifier() {
        let ids = found("```\nerror: X\n```", &["ht-"]);
        assert!(
            ids.iter().all(|(k, _)| *k != IdentifierKind::Error),
            "{ids:?}"
        );
    }

    #[test]
    fn long_error_span_is_truncated_to_the_cap() {
        let text = format!("`error: {}`", "é".repeat(500));
        let ids = found(&text, &["ht-"]);
        assert_eq!(ids.len(), 1, "{ids:?}");
        let (kind, value) = &ids[0];
        assert_eq!(*kind, IdentifierKind::Error);
        assert!(value.len() <= MAX_IDENTIFIER_BYTES, "{}", value.len());
        assert!(value.ends_with('…'));
        assert!(value.starts_with("error: é"));
    }

    #[test]
    fn over_long_token_is_not_an_identifier() {
        let long = format!("src/{}", "a".repeat(296));
        assert_eq!(long.len(), 300);
        assert!(found(&long, &["ht-"]).is_empty());
        let ok = format!("src/{}", "a".repeat(96));
        assert_eq!(ok.len(), 100);
        assert_eq!(
            found(&ok, &["ht-"]),
            vec![(IdentifierKind::Path, ok.clone())]
        );
    }

    #[test]
    fn merge_bounds_stored_values() {
        let long = Identifier {
            value: format!("error {}", "x".repeat(394)),
            kind: IdentifierKind::Error,
            seqs: vec![1],
        };
        assert_eq!(long.value.len(), 400);
        let merged = merge(&[std::slice::from_ref(&long)]);
        assert_eq!(merged.len(), 1);
        assert!(merged[0].value.len() <= MAX_IDENTIFIER_BYTES);
        assert!(merged[0].value.ends_with('…'));
    }

    #[test]
    fn fold_merge_by_value() {
        let a = vec![
            Identifier {
                value: "src/a.rs".into(),
                kind: IdentifierKind::Path,
                seqs: vec![1, 4],
            },
            Identifier {
                value: "src/b.rs".into(),
                kind: IdentifierKind::Path,
                seqs: vec![2],
            },
        ];
        let b = vec![Identifier {
            value: "src/a.rs".into(),
            kind: IdentifierKind::Path,
            seqs: vec![4, 12],
        }];
        let merged = merge(&[&a, &b]);
        assert_eq!(merged.len(), 2);
        assert_eq!(merged[0].value, "src/a.rs");
        assert_eq!(merged[0].seqs, vec![1, 4, 12]);
        assert_eq!(merged[1].value, "src/b.rs");

        // Fold quotas: 100 paths in, 48 most recent kept.
        let many: Vec<Identifier> = (1..=100u64)
            .map(|n| Identifier {
                value: format!("src/f{n}.rs"),
                kind: IdentifierKind::Path,
                seqs: vec![n],
            })
            .collect();
        let merged = merge(&[&many]);
        assert_eq!(merged.len(), 48);
        assert_eq!(merged[0].value, "src/f100.rs");
        assert_eq!(merged[47].value, "src/f53.rs");

        // Every kind over quota: total cap 128.
        let all_kinds: Vec<Identifier> = [
            (IdentifierKind::Path, "src/p{}.rs", 60u64),
            (IdentifierKind::BeadId, "ht-{}x", 40),
            (IdentifierKind::Sha, "{}abcdef0", 30),
            (IdentifierKind::Url, "https://u/{}", 20),
            (IdentifierKind::Error, "error {}", 10),
        ]
        .into_iter()
        .flat_map(|(kind, pat, count)| {
            (1..=count).map(move |n| Identifier {
                value: pat.replace("{}", &n.to_string()),
                kind,
                seqs: vec![n],
            })
        })
        .collect();
        assert_eq!(merge(&[&all_kinds]).len(), FOLD_IDENTIFIER_CAP);
    }
}
