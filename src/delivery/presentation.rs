//! One ordered projection and exact bounded output measurement for capable delivery.
use crate::{
    delivery::{FrameItem, frame},
    harness::adapter::PresentationCodec,
    protocol::{
        delivery::{
            PresentationControlStampV2 as Stamp, PresentationItemV2 as Item,
            PresentationRenderSelection as Selection, valid_presentation_epoch,
        },
        watch::{WatchAttention, WatchAttentionCleared, WatchItem, WatchLine, WatchNotice},
    },
};
use std::io::{self, Write};

pub const OUTPUT_LIMIT: usize = 65536;
pub const RPC_RESULT_LIMIT: usize = 1048576 - 512;

/// Service-validated registered descriptors; no caller-provided native envelope.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PresentationRenderProfile {
    Hook {
        confirmable_selector: u8,
        passive_selector: u8,
        confirmable: PresentationCodec,
        passive: PresentationCodec,
    },
    WatchJsonLinesV1,
}
impl PresentationRenderProfile {
    pub fn selection(&self, items: &[Item]) -> Selection {
        match self {
            Self::WatchJsonLinesV1 => Selection::WatchJsonLinesV1,
            Self::Hook {
                confirmable_selector,
                passive_selector,
                ..
            } => Selection::Hook {
                selector: if items
                    .iter()
                    .any(|v| matches!(v,Item::Message{content,..} if !content.truncated))
                {
                    *confirmable_selector
                } else {
                    *passive_selector
                },
            },
        }
    }
    pub fn codec(&self, selection: &Selection) -> Option<PresentationCodec> {
        match (self, selection) {
            (
                Self::Hook {
                    confirmable_selector,
                    confirmable,
                    ..
                },
                Selection::Hook { selector },
            ) if selector == confirmable_selector => Some(*confirmable),
            (
                Self::Hook {
                    passive_selector,
                    passive,
                    ..
                },
                Selection::Hook { selector },
            ) if selector == passive_selector => Some(*passive),
            _ => None,
        }
    }
}

pub fn emitted_controls(items: &[Item]) -> Vec<Stamp> {
    items
        .iter()
        .filter_map(|item| match item {
            Item::Attention {
                epoch,
                attention_version,
                ..
            } => Some(Stamp::Attention {
                epoch: epoch.clone(),
                version: *attention_version,
            }),
            Item::AttentionCleared {
                epoch,
                attention_version,
            } => Some(Stamp::Cleared {
                epoch: epoch.clone(),
                version: *attention_version,
            }),
            _ => None,
        })
        .collect()
}
pub fn presentation_watch_lines(items: &[Item]) -> Vec<WatchLine> {
    items
        .iter()
        .map(|item| match item {
            Item::Message { id, lazy, content } => WatchLine::new(
                id.as_str(),
                if *lazy {
                    WatchItem::Lazy(content.clone())
                } else {
                    WatchItem::Message(content.clone())
                },
            ),
            Item::Invitation { id, content } => WatchLine::new(
                format!("invitation:{}", id.as_str()),
                WatchItem::Invitation(content.clone()),
            ),
            Item::Notice {
                warning,
                thread,
                body,
            } => WatchLine::new(
                format!("notice:{}", warning.as_str()),
                WatchItem::Notice(WatchNotice {
                    warning: warning.clone(),
                    thread: thread.clone(),
                    body: body.clone(),
                }),
            ),
            Item::Attention {
                epoch,
                attention_version,
                text,
            } => WatchLine::new(
                format!("attention:{epoch}"),
                WatchItem::Attention(WatchAttention {
                    presentation_epoch: Some(epoch.clone()),
                    attention_version: *attention_version,
                    text: text.clone(),
                }),
            ),
            Item::AttentionCleared {
                epoch,
                attention_version,
            } => WatchLine::new(
                format!("attention_cleared:{epoch}"),
                WatchItem::AttentionCleared(WatchAttentionCleared {
                    presentation_epoch: Some(epoch.clone()),
                    attention_version: *attention_version,
                }),
            ),
        })
        .collect()
}
pub fn presentation_hook_text(items: &[Item]) -> String {
    if items.is_empty() {
        return String::new();
    }
    let frames = items
        .iter()
        .map(|item| match item {
            Item::Message {
                id,
                lazy,
                content: m,
            } => FrameItem {
                kind: if *lazy { "lazy" } else { "message" }.into(),
                id: id.as_str().into(),
                thread: Some(m.thread.as_str().into()),
                thread_name: m.thread_name.clone(),
                sender: m.sender.as_ref().map(|s| s.as_str().into()),
                sender_name: m.sender_name.clone(),
                author_role: m.author_role.map(|v| v.as_str().into()),
                relays_user: m.relays_user,
                user_intent: m.user_intent.map(|v| v.as_str().into()),
                body: m.body.clone(),
            },
            Item::Invitation { id, content } => FrameItem {
                kind: "invitation".into(),
                id: id.as_str().into(),
                thread: Some(content.thread.as_str().into()),
                thread_name: content.thread_name.clone(),
                body: content.body.clone(),
                ..FrameItem::default()
            },
            Item::Notice {
                warning,
                thread,
                body,
            } => FrameItem {
                kind: "notice".into(),
                id: warning.as_str().into(),
                thread: Some(thread.as_str().into()),
                body: body.clone(),
                ..FrameItem::default()
            },
            Item::Attention { epoch, text, .. } => FrameItem {
                kind: "attention".into(),
                id: format!("attention:{epoch}"),
                body: text.clone(),
                ..FrameItem::default()
            },
            Item::AttentionCleared { epoch, .. } => FrameItem {
                kind: "attention_cleared".into(),
                id: format!("attention_cleared:{epoch}"),
                ..FrameItem::default()
            },
        })
        .collect::<Vec<_>>();
    frame(&frames)
}

pub fn valid_items(items: &[Item]) -> bool {
    items.len() <= 32
        && items.iter().all(|item| match item {
            Item::Attention { epoch, .. } | Item::AttentionCleared { epoch, .. } => {
                valid_presentation_epoch(epoch)
            }
            _ => true,
        })
}
/// Counting writer stops on the first byte beyond the admitted ceiling.
pub struct CappedCount {
    pub bytes: usize,
    limit: usize,
}
impl CappedCount {
    pub fn new(limit: usize) -> Self {
        Self { bytes: 0, limit }
    }
}
impl Write for CappedCount {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let remaining = (self.limit + 1).saturating_sub(self.bytes);
        self.bytes += remaining.min(buf.len());
        if self.bytes > self.limit {
            return Err(io::Error::other("presentation size limit"));
        }
        Ok(buf.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
/// Exact serializer, including native envelope, all Watch typed fields and LF.
pub fn measure(
    profile: &PresentationRenderProfile,
    items: &[Item],
) -> Result<(Selection, usize), usize> {
    if !valid_items(items) {
        return Err(OUTPUT_LIMIT + 1);
    }
    let selection = profile.selection(items);
    if items.is_empty() {
        return Ok((selection, 0));
    }
    let mut count = CappedCount::new(OUTPUT_LIMIT);
    let result: Result<(), io::Error> = (|| {
        match &selection {
            Selection::WatchJsonLinesV1 => {
                for line in presentation_watch_lines(items) {
                    serde_json::to_writer(&mut count, &line)?;
                    count.write_all(b"\n")?;
                }
            }
            Selection::Hook { .. } => {
                let Some(PresentationCodec::JsonStringEnvelope { prefix, suffix }) =
                    profile.codec(&selection)
                else {
                    return Err(io::Error::other("unregistered presentation codec"));
                };
                count.write_all(prefix.as_bytes())?;
                serde_json::to_writer(&mut count, &presentation_hook_text(items))?;
                count.write_all(suffix.as_bytes())?;
            }
        }
        Ok(())
    })();
    result
        .map(|()| (selection, count.bytes))
        .map_err(|_| count.bytes.max(OUTPUT_LIMIT + 1))
}

impl PresentationRenderProfile {
    pub fn validate(&self) -> bool {
        let valid = |codec: PresentationCodec| match codec {
            PresentationCodec::JsonStringEnvelope { prefix, suffix } => prefix
                .len()
                .checked_add(suffix.len())
                .and_then(|n| n.checked_add(2))
                .is_some_and(|n| n <= OUTPUT_LIMIT),
        };
        match self {
            Self::WatchJsonLinesV1 => true,
            Self::Hook {
                confirmable_selector,
                passive_selector,
                confirmable,
                passive,
            } => {
                valid(*confirmable)
                    && valid(*passive)
                    && (confirmable_selector != passive_selector || confirmable == passive)
            }
        }
    }
    pub fn render_key(&self, prefix: &[String]) -> String {
        use sha2::{Digest, Sha256};
        let descriptors = match self {
            Self::WatchJsonLinesV1 => serde_json::json!(["watch_json_lines_v1"]),
            Self::Hook {
                confirmable_selector,
                passive_selector,
                confirmable:
                    PresentationCodec::JsonStringEnvelope {
                        prefix: cp,
                        suffix: cs,
                    },
                passive:
                    PresentationCodec::JsonStringEnvelope {
                        prefix: pp,
                        suffix: ps,
                    },
            } => serde_json::json!([
                "hook",
                confirmable_selector,
                passive_selector,
                cp,
                cs,
                pp,
                ps
            ]),
        };
        let data = serde_json::json!(["presentation-render-v1", descriptors, prefix, OUTPUT_LIMIT]);
        format!("{:x}", Sha256::digest(data.to_string().as_bytes()))
    }
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct SizingWork {
    pub output_passes: usize,
    pub output_bytes: usize,
    pub rpc_passes: usize,
    pub rpc_bytes: usize,
}
pub struct Sizer {
    pub work: SizingWork,
}
impl Sizer {
    pub fn new() -> Self {
        Self {
            work: SizingWork::default(),
        }
    }
    pub fn measure(
        &mut self,
        profile: &PresentationRenderProfile,
        items: &[Item],
    ) -> Result<(Selection, usize), usize> {
        if self.work.output_passes >= 352 {
            return Err(OUTPUT_LIMIT + 1);
        }
        self.work.output_passes += 1;
        let result = measure(profile, items);
        self.work.output_bytes += match result {
            Ok((_, bytes)) => bytes,
            Err(bytes) => bytes,
        };
        result
    }
    pub fn rpc<T: serde::Serialize>(&mut self, value: &T) -> Result<usize, usize> {
        if self.work.rpc_passes != 0 {
            return Err(RPC_RESULT_LIMIT + 1);
        }
        self.work.rpc_passes += 1;
        let mut count = CappedCount::new(RPC_RESULT_LIMIT);
        let result = serde_json::to_writer(&mut count, value);
        self.work.rpc_bytes = count.bytes;
        result
            .map(|()| count.bytes)
            .map_err(|_| count.bytes.max(RPC_RESULT_LIMIT + 1))
    }
}
impl Default for Sizer {
    fn default() -> Self {
        Self::new()
    }
}

/// Exact canonical fixed-row snapshot; it is rechecked in the deciding transaction.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FrozenRenderIdentity {
    pub instance: String,
    pub seat: crate::protocol::ids::SeatId,
    pub generation: u64,
    pub execution: crate::protocol::ids::ExecutionId,
    pub native_session: crate::protocol::ids::NativeSessionId,
    pub consumer: crate::protocol::delivery::PresentationConsumer,
    pub token: String,
    pub render_key: String,
    pub descriptor_key: String,
    pub selection: Selection,
}
#[derive(Debug, Clone)]
pub struct FrozenPresentationRender {
    pub identity: FrozenRenderIdentity,
    pub codec: Option<PresentationCodec>,
}
pub fn descriptor_key(codec: PresentationCodec) -> String {
    use sha2::{Digest, Sha256};
    let PresentationCodec::JsonStringEnvelope { prefix, suffix } = codec;
    format!(
        "{:x}",
        Sha256::digest(
            serde_json::json!(["presentation-codec-v1", prefix, suffix])
                .to_string()
                .as_bytes()
        )
    )
}
