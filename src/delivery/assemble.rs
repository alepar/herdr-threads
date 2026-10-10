//! Chunk reassembly, body truncation and name labels, moved verbatim from the
//! `watch` CLI (delivery-policy spec DP2). An [`Assembler`] turns the items of
//! `InboxBatchV2` pages into [`DeliveryItem`]s: one per message, once per
//! assembler, with a body cut at [`BODY_LIMIT_BYTES`].

use super::page::BODY_LIMIT_BYTES;
use crate::protocol::{
    ids::{MessageId, SeatId},
    results::InboxBatchV2Item,
    watch::{WatchMessage, truncation_marker_for},
};
use std::{
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
};

/// Newest ids kept in the truncation hint file.
const HINT_MAX_LINES: usize = 1024;
/// Most chars of a name on a header line.
const LABEL_MAX_CHARS: usize = 120;

/// Whether an item arrived as an ordinary or a lazy message.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeliveryKind {
    Message,
    Lazy,
}

/// One fully assembled message. It reuses the frozen `watch` wire shape, so
/// `watch` lines are unchanged.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeliveryItem {
    pub kind: DeliveryKind,
    pub id: MessageId,
    pub message: WatchMessage,
}

/// What [`Assembler::push`] made of one page item.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Assembled {
    /// A message completed (whole, or its first `BODY_LIMIT_BYTES`).
    Item(DeliveryItem),
    /// A chunk was buffered; more of the body is needed.
    Pending,
    /// An invitation or a waking warning: the seat needs attention.
    Attention,
    /// A warning the daemon judged does not wake this seat.
    InformationalNotice,
    /// The message was already produced by this assembler.
    Duplicate,
}

/// The daemon's chunks broke the paging contract.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AssembleError;

/// Reassembly state. `emitted` and `attention_shown` outlive one drain; `partial` is
/// per drain (see [`super::page::drain_pages`]).
#[derive(Debug, Default)]
pub struct Assembler {
    /// Ids already produced (and `attention:<version>` ids already shown).
    pub emitted: HashSet<String>,
    /// Text assembled so far for each multi-chunk body.
    pub partial: HashMap<String, String>,
    /// Invocation the truncation marker's commands start with; empty means
    /// bare program name.
    pub command_prefix: Vec<String>,
    /// File remembering the ids shown cut (a hint only).
    pub truncation_hint: Option<PathBuf>,
    /// An attention was reported and not yet retracted.
    pub attention_shown: bool,
}

impl Assembler {
    pub fn new(truncation_hint: Option<PathBuf>) -> Self {
        Self {
            truncation_hint,
            ..Self::default()
        }
    }

    pub fn with_command_prefix(mut self, prefix: Vec<String>) -> Self {
        self.command_prefix = prefix;
        self
    }

    pub fn push(&mut self, item: InboxBatchV2Item) -> Result<Assembled, AssembleError> {
        match item {
            InboxBatchV2Item::Invitation { .. }
            | InboxBatchV2Item::Warning {
                informational: false,
                ..
            } => Ok(Assembled::Attention),
            // An informational notice (the daemon judged it does not wake this
            // seat) reaches the seat at its next check-in; it raises no attention.
            InboxBatchV2Item::Warning {
                informational: true,
                ..
            } => Ok(Assembled::InformationalNotice),
            InboxBatchV2Item::Message {
                thread,
                topic_data,
                message,
                sender,
                author_role,
                relays_user,
                user_intent,
                body,
                body_start,
                body_end,
                body_len,
                ack_candidate,
                ..
            } => self.emit_chunk(Chunk {
                lazy: false,
                ack_required: ack_candidate.is_some(),
                thread,
                topic: topic_data,
                message,
                sender,
                author_role,
                relays_user,
                user_intent,
                body,
                body_start,
                body_end,
                body_len,
            }),
            InboxBatchV2Item::LazyMessage {
                thread,
                topic_data,
                message,
                sender,
                author_role,
                relays_user,
                user_intent,
                body,
                body_start,
                body_end,
                body_len,
                ..
            } => self.emit_chunk(Chunk {
                lazy: true,
                ack_required: false,
                thread,
                topic: topic_data,
                message,
                sender,
                author_role,
                relays_user,
                user_intent,
                body,
                body_start,
                body_end,
                body_len,
            }),
        }
    }

    /// Add one body chunk; the item is produced once the first
    /// `BODY_LIMIT_BYTES` bytes or the whole body are assembled.
    fn emit_chunk(&mut self, chunk: Chunk) -> Result<Assembled, AssembleError> {
        let id = chunk.message.as_str().to_owned();
        if self.emitted.contains(&id) {
            return Ok(Assembled::Duplicate);
        }
        let entry = self.partial.entry(id.clone()).or_default();
        if chunk.body_start == 0 {
            entry.clear();
        }
        if chunk.body_start != entry.len() as u64 {
            self.partial.remove(&id);
            return Err(AssembleError);
        }
        entry.push_str(&chunk.body);
        let whole = chunk.body_end >= chunk.body_len;
        if !whole && entry.len() < BODY_LIMIT_BYTES {
            return Ok(Assembled::Pending);
        }
        let assembled = self.partial.remove(&id).unwrap_or_default();
        let truncated = chunk.body_len > BODY_LIMIT_BYTES as u64;
        let body = if truncated {
            let mut cut = BODY_LIMIT_BYTES.min(assembled.len());
            while !assembled.is_char_boundary(cut) {
                cut -= 1;
            }
            record_truncated(self.truncation_hint.as_deref(), &id);
            format!(
                "{}{}",
                &assembled[..cut],
                truncation_marker_for(&self.command_prefix, &chunk.message, chunk.lazy)
            )
        } else {
            assembled
        };
        let sender_name = Some(
            chunk
                .sender
                .as_ref()
                .map_or_else(|| "service".to_owned(), |seat| seat.as_str().to_owned()),
        );
        let message = WatchMessage {
            thread: chunk.thread,
            thread_name: label(&chunk.topic),
            sender: chunk.sender,
            sender_name,
            author_role: chunk.author_role,
            relays_user: chunk.relays_user,
            user_intent: chunk.user_intent,
            body,
            body_len: chunk.body_len,
            truncated,
            ack_required: chunk.ack_required,
        };
        self.emitted.insert(id);
        Ok(Assembled::Item(DeliveryItem {
            kind: if chunk.lazy {
                DeliveryKind::Lazy
            } else {
                DeliveryKind::Message
            },
            id: chunk.message,
            message,
        }))
    }
}

struct Chunk {
    lazy: bool,
    ack_required: bool,
    thread: crate::protocol::ids::ThreadId,
    topic: String,
    message: MessageId,
    sender: Option<SeatId>,
    author_role: Option<crate::protocol::summary::AuthorRole>,
    relays_user: bool,
    user_intent: Option<crate::protocol::summary::UserIntent>,
    body: String,
    body_start: u64,
    body_end: u64,
    body_len: u64,
}

/// A user-controlled name for one header line (spec D4): every control
/// character and Unicode line or paragraph separator becomes a space, the
/// ends are trimmed, and it is cut to `LABEL_MAX_CHARS` chars plus `…`.
/// `None` when nothing is left.
pub fn label(text: &str) -> Option<String> {
    let flat: String = text
        .chars()
        .map(|c| {
            if c.is_control() || matches!(c, '\u{2028}' | '\u{2029}') {
                ' '
            } else {
                c
            }
        })
        .collect();
    let flat = flat.trim();
    if flat.is_empty() {
        return None;
    }
    let mut out: String = flat.chars().take(LABEL_MAX_CHARS).collect();
    if flat.chars().count() > LABEL_MAX_CHARS {
        out.push('…');
    }
    Some(out)
}

/// Hint only: remember that this seat was shown a cut body for `id`, so
/// `watch ack` can refuse it locally. The daemon decides truncation itself.
pub fn record_truncated(hint_path: Option<&Path>, id: &str) {
    let Some(path) = hint_path else {
        return;
    };
    let mut lines: Vec<String> = std::fs::read_to_string(path)
        .map(|text| text.lines().map(str::to_owned).collect())
        .unwrap_or_default();
    if lines.iter().any(|line| line == id) {
        return;
    }
    lines.push(id.to_owned());
    if lines.len() > HINT_MAX_LINES {
        lines.drain(..lines.len() - HINT_MAX_LINES);
    }
    let mut text = lines.join("\n");
    text.push('\n');
    let temp = path.with_extension("ids.tmp");
    if std::fs::write(&temp, text).is_ok() {
        let _ = std::fs::rename(&temp, path);
    }
}

pub fn read_truncated_hint(path: Option<&Path>) -> HashSet<String> {
    path.and_then(|path| std::fs::read_to_string(path).ok())
        .map(|text| text.lines().map(str::to_owned).collect())
        .unwrap_or_default()
}
