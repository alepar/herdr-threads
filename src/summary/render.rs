//! Rendering of messages into job-bundle lines, `chunking_version`, and the
//! bundle size bound (spec §2, §3). Pure.
use crate::protocol::{
    results::MessageKind,
    summary::{BundleMessage, FoldDisplay, ItemBody, ItemStatus, JobBundle, SummarySettings},
};
use sha2::{Digest, Sha256};

/// Version of the rendering format; part of `chunking_version`, so any change
/// to `render_message` must bump it.
pub const RENDERER_VERSION: u32 = 2;

fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let year = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let month = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (year + i64::from(month <= 2), month, day)
}

/// `YYYY-MM-DDTHH:MM:SSZ`, whole seconds (floor), UTC.
fn rfc3339_seconds(millis: i64) -> String {
    let secs = millis.div_euclid(1000);
    let (year, month, day) = civil_from_days(secs.div_euclid(86_400));
    let rem = secs.rem_euclid(86_400);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}Z",
        rem / 3600,
        rem % 3600 / 60,
        rem % 60
    )
}

fn single_line(text: &str) -> String {
    text.chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect()
}

/// One message as a bundle line: `#<seq> <message id> <author|system>[ [human]][ [relays user]][ [query|request|rule]]
/// <RFC3339 UTC seconds>\n<body>\n`; info/warn render as one compact line
/// `#<seq> info|warn <text>\n`.
pub fn render_message(message: &BundleMessage) -> String {
    match message.kind {
        MessageKind::Ordinary => {
            let author = message
                .author
                .as_ref()
                .map_or("system", |seat| seat.as_str());
            let markers = crate::protocol::results::author_markers(
                message.kind,
                message.author_role,
                message.relays_user,
                message.user_intent,
            );
            format!(
                "#{} {} {author}{markers} {}\n{}\n",
                message.sequence,
                message.message.as_str(),
                rfc3339_seconds(message.created_at.0),
                message.text
            )
        }
        MessageKind::Info => format!(
            "#{} info {}\n",
            message.sequence,
            single_line(&message.text)
        ),
        MessageKind::Warn => format!(
            "#{} warn {}\n",
            message.sequence,
            single_line(&message.text)
        ),
    }
}

pub fn rendered_size(message: &BundleMessage) -> u64 {
    render_message(message).len() as u64
}

/// Identifies one chunking of a thread: derived from the renderer version,
/// `chunk_bytes` and the ordered `tracker_prefixes`. Always 20 bytes
/// (`cv1-` plus 16 hex digits), inside the 64-byte column check.
pub fn chunking_version(settings: &SummarySettings) -> String {
    let input = format!(
        "renderer={RENDERER_VERSION}\nchunk_bytes={}\ntracker_prefixes={}\n",
        settings.chunk_bytes,
        settings.tracker_prefixes.join(",")
    );
    let digest = Sha256::digest(input.as_bytes());
    let hex: String = digest.iter().take(8).map(|b| format!("{b:02x}")).collect();
    format!("cv1-{hex}")
}

fn encoded_len(bundle: &JobBundle) -> usize {
    serde_json::to_vec(bundle).map_or(usize::MAX, |bytes| bytes.len())
}

/// Bound a bundle to the soft target `bundle_bytes` (spec §3). Spill order,
/// oldest first: pinned raw text becomes `text_ref`, then non-open fold
/// entries become `TextRef` (a user instruction's raw text moves to its
/// `text_ref`). Messages, narratives and open entries are never cut; a bundle
/// still over target is emitted with `oversized` and its size.
pub fn bound_bundle(bundle: &mut JobBundle, bundle_bytes: u32) {
    let target = bundle_bytes as usize;
    let mut order: Vec<usize> = (0..bundle.pinned.len())
        .filter(|i| bundle.pinned[*i].text.is_some())
        .collect();
    order.sort_by_key(|i| (bundle.pinned[*i].seq, *i));
    for i in order {
        if encoded_len(bundle) <= target {
            break;
        }
        let pinned = &mut bundle.pinned[i];
        pinned.text = None;
        pinned.text_ref = Some(pinned.seq);
    }
    let mut order: Vec<usize> = (0..bundle.fold.entries.len())
        .filter(|i| {
            let entry = &bundle.fold.entries[*i];
            !matches!(entry.status, ItemStatus::Open | ItemStatus::Active)
                && entry.display != FoldDisplay::TextRef
        })
        .collect();
    order.sort_by_key(|i| (bundle.fold.entries[*i].closed_at_seq.unwrap_or(0), *i));
    for i in order {
        if encoded_len(bundle) <= target {
            break;
        }
        let entry = &mut bundle.fold.entries[i];
        entry.display = FoldDisplay::TextRef;
        let seq = entry.item.seq;
        if let ItemBody::UserInstruction { text, text_ref, .. } = &mut entry.item.body
            && text.is_some()
        {
            *text = None;
            *text_ref = Some(seq);
        }
    }
    // `size_bytes` and `oversized` are inside the encoding: iterate to the
    // fixed point (at most a few rounds; the digit count settles at once).
    for _ in 0..6 {
        let len = encoded_len(bundle).min(u32::MAX as usize) as u32;
        let oversized = len as usize > target;
        if bundle.size_bytes == len && bundle.oversized == oversized {
            break;
        }
        bundle.size_bytes = len;
        bundle.oversized = oversized;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::summary::AuthorRole;
    use crate::protocol::{
        ids::{MessageId, SeatId, SummaryBlockId, SummaryJobId, ThreadId},
        summary::{
            ChildNarrative, Fold, FoldEntry, LedgerItem, PinnedText, SUBMISSION_SCHEMA, SeqRange,
        },
        time::UtcMillis,
    };

    fn message(kind: MessageKind, sequence: u64, text: &str) -> BundleMessage {
        BundleMessage {
            sequence,
            message: MessageId::new("MSG"),
            kind,
            author: None,
            author_role: None,
            relays_user: false,
            user_intent: None,
            created_at: UtcMillis(60_000),
            text: text.to_string(),
        }
    }

    #[test]
    fn ordinary_message_renders_header_and_body() {
        let m = BundleMessage {
            author: Some(SeatId::new("S1")),
            author_role: Some(AuthorRole::Human),
            relays_user: true,
            ..message(MessageKind::Ordinary, 7, "hello\nworld")
        };
        let text = "#7 MSG S1 [human] [relays user] 1970-01-01T00:01:00Z\nhello\nworld\n";
        assert_eq!(render_message(&m), text);
        assert_eq!(rendered_size(&m), text.len() as u64);
        let plain = message(MessageKind::Ordinary, 3, "x");
        assert_eq!(
            render_message(&plain),
            "#3 MSG system 1970-01-01T00:01:00Z\nx\n"
        );
    }

    #[test]
    fn timestamp_handles_leap_days() {
        // 2024-02-29T12:34:56Z = 1_709_210_096 s.
        assert_eq!(rfc3339_seconds(1_709_210_096_999), "2024-02-29T12:34:56Z");
        assert_eq!(rfc3339_seconds(0), "1970-01-01T00:00:00Z");
    }

    #[test]
    fn system_message_renders_one_compact_line() {
        let info = message(MessageKind::Info, 8, "a\nb\tc");
        let warn = message(MessageKind::Warn, 9, "w\r\nz");
        assert_eq!(render_message(&info), "#8 info a b c\n");
        assert_eq!(render_message(&warn), "#9 warn w  z\n");
    }

    #[test]
    fn chunking_version_is_stable_and_sensitive() {
        let base = SummarySettings::default();
        let v = chunking_version(&base);
        assert_eq!(v, chunking_version(&base.clone()));
        assert!(v.starts_with("cv1-"));
        assert_eq!(v.len(), 20);
        let mut other = base.clone();
        other.chunk_bytes += 1;
        assert_ne!(chunking_version(&other), v);
        let mut other = base.clone();
        other.tracker_prefixes = vec!["ab-".into(), "cd-".into()];
        let ab_cd = chunking_version(&other);
        assert_ne!(ab_cd, v);
        other.tracker_prefixes = vec!["cd-".into(), "ab-".into()];
        assert_ne!(chunking_version(&other), ab_cd);
        let mut other = base.clone();
        other.display_bytes += 1;
        assert_eq!(chunking_version(&other), v);
    }

    // ---- bundle bounding ----
    fn instruction(id: &str, status: ItemStatus, closed: Option<u64>, seq: u64) -> FoldEntry {
        FoldEntry {
            item: LedgerItem {
                id: id.to_string(),
                seq,
                body: ItemBody::UserInstruction {
                    author_seat: None,
                    author_role: Some(AuthorRole::Human),
                    relays_user: false,
                    user_intent: None,
                    text: Some("t".repeat(300)),
                    text_ref: None,
                    message_id: None,
                },
            },
            status,
            closed_at_seq: closed,
            display: FoldDisplay::Full,
        }
    }

    fn bundle() -> JobBundle {
        JobBundle {
            job_id: SummaryJobId::new("job"),
            thread: ThreadId::new("t"),
            chunking_version: "cv1-0000000000000000".into(),
            level: 1,
            index: 0,
            range: SeqRange {
                first_seq: 1,
                last_seq: 10,
            },
            submission_schema: SUBMISSION_SCHEMA,
            budget_bytes: 4096,
            narrative_bytes: 3072,
            messages: vec![],
            children: vec![],
            fold: Fold {
                entries: vec![],
                identifiers: vec![],
                rendered_bytes: 0,
            },
            pinned: vec![],
            size_bytes: 0,
            oversized: false,
        }
    }

    fn pinned(seq: u64) -> PinnedText {
        PinnedText {
            item_id: format!("i.{seq}"),
            seq,
            text: Some("p".repeat(400)),
            text_ref: None,
        }
    }

    fn size(b: &JobBundle) -> usize {
        serde_json::to_vec(b).unwrap().len()
    }

    #[test]
    fn small_bundle_is_untouched() {
        let mut b = bundle();
        b.pinned = vec![pinned(5)];
        let before = b.clone();
        bound_bundle(&mut b, 48 * 1024);
        assert!(!b.oversized);
        assert_eq!(b.size_bytes as usize, size(&b));
        assert_eq!(b.pinned, before.pinned);
        assert_eq!(b.fold, before.fold);
    }

    #[test]
    fn pinned_text_spills_oldest_first() {
        let mut b = bundle();
        b.pinned = vec![pinned(12), pinned(5), pinned(9)];
        let full = size(&b);
        // Spilling one text frees about 400 bytes: exactly the oldest (seq 5) spills.
        bound_bundle(&mut b, (full - 300) as u32);
        assert_eq!(b.pinned[1].text, None);
        assert_eq!(b.pinned[1].text_ref, Some(5));
        assert!(b.pinned[0].text.is_some() && b.pinned[2].text.is_some());
        assert!(!b.oversized);
        // A tighter target also spills seq 9, never 12 first.
        let mut b = bundle();
        b.pinned = vec![pinned(12), pinned(5), pinned(9)];
        bound_bundle(&mut b, (full - 700) as u32);
        assert_eq!(b.pinned[1].text_ref, Some(5));
        assert_eq!(b.pinned[2].text_ref, Some(9));
        assert!(b.pinned[0].text.is_some());
        assert!(!b.oversized);
    }

    #[test]
    fn closed_fold_entries_spill_after_pinned_text() {
        let mut b = bundle();
        b.pinned = vec![pinned(5)];
        b.fold.entries = vec![
            instruction("open", ItemStatus::Open, None, 1),
            instruction("late", ItemStatus::Done, Some(90), 2),
            instruction("active", ItemStatus::Active, None, 3),
            instruction("early", ItemStatus::Resolved, Some(40), 4),
        ];
        let full = size(&b);
        // Needs the pinned text (~400) plus one closed entry (~300) to fit.
        bound_bundle(&mut b, (full - 600) as u32);
        assert_eq!(b.pinned[0].text_ref, Some(5), "pinned text spills first");
        let d: Vec<_> = b.fold.entries.iter().map(|e| e.display).collect();
        assert_eq!(d[0], FoldDisplay::Full, "open entries never change");
        assert_eq!(d[2], FoldDisplay::Full, "active entries never change");
        assert_eq!(d[3], FoldDisplay::TextRef, "oldest closed spills first");
        assert_eq!(d[1], FoldDisplay::Full, "stops as soon as it fits");
        assert!(!b.oversized);
        // Pinned text is spilled before any fold entry is touched.
        let mut b2 = bundle();
        b2.pinned = vec![pinned(5)];
        b2.fold.entries = vec![instruction("late", ItemStatus::Done, Some(90), 2)];
        let full2 = size(&b2);
        bound_bundle(&mut b2, (full2 - 100) as u32);
        assert_eq!(b2.pinned[0].text_ref, Some(5));
        assert_eq!(b2.fold.entries[0].display, FoldDisplay::Full);
    }

    #[test]
    fn still_over_is_emitted_oversized() {
        let mut b = bundle();
        b.messages = vec![message(MessageKind::Ordinary, 1, &"m".repeat(2000))];
        b.children = vec![ChildNarrative {
            block_id: SummaryBlockId::new("b"),
            level: 0,
            index: 0,
            range: SeqRange {
                first_seq: 1,
                last_seq: 1,
            },
            narrative: "n".repeat(500),
            fallback: false,
        }];
        let before = (b.messages.clone(), b.children.clone());
        bound_bundle(&mut b, 1000);
        assert!(b.oversized);
        assert_eq!(b.size_bytes as usize, size(&b));
        assert!(b.size_bytes > 1000);
        assert_eq!((b.messages, b.children), before);
    }
}
