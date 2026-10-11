//! Pure canonical invitation rendering and final-frame sizing. Full context
//! uses the page allowance, independently of the ordinary-message8KiB bound.
use crate::{
    delivery::{FrameItem, HEADER, frame},
    protocol::{
        commands::InvitationConfirmation,
        ids::{InvitationId, ThreadId},
        output::format_command_argv,
        service::RequiredMembership,
        time::UtcMillis,
        watch::WatchInvitation,
    },
};
use serde::{Deserialize, Serialize};

pub const MAX_BLOCKS: usize = 32;
pub const MAX_TEXT_BYTES: usize = 65536;

#[derive(Debug, Clone)]
pub struct InvitationContext {
    pub id: InvitationId,
    pub thread: ThreadId,
    pub topic: String,
    pub goal: String,
    pub episode: u64,
    pub topic_revision: u64,
    pub command_epoch: u64,
    pub confirmation: Option<InvitationConfirmation>,
    pub deadline: Option<UtcMillis>,
    pub required: Option<RequiredMembership>,
}
impl InvitationContext {
    pub fn key(&self) -> String {
        serde_json::json!([
            self.episode.to_string(),
            self.topic_revision.to_string(),
            self.command_epoch.to_string(),
            self.confirmation,
            self.deadline,
            self.required
        ])
        .to_string()
    }
    pub fn render(&self, prefix: &[String]) -> FrameItem {
        let mut argv = if prefix.is_empty() {
            vec!["herdr-threads".into()]
        } else {
            prefix.to_vec()
        };
        if let Some(required) = &self.required {
            argv.extend([
                "accept-required".into(),
                self.thread.as_str().into(),
                "--invitation".into(),
                self.id.as_str().into(),
                "--requirement".into(),
                required.requirement.as_str().into(),
                "--revision".into(),
                required.revision.to_string(),
            ]);
        } else {
            argv.extend(["accept".into(), self.thread.as_str().into()]);
        }
        let mut body = format!("{}\ngoal: {}", self.topic, self.goal);
        if let Some(deadline) = self.deadline {
            body.push_str(&format!("\ndeadline: {}", deadline.0));
        }
        body.push_str(&format!("\nnext: {}", format_command_argv(&argv)));
        FrameItem {
            kind: "invitation".into(),
            id: self.id.as_str().into(),
            thread: Some(self.thread.as_str().into()),
            thread_name: Some(self.topic.chars().take(120).collect()),
            body,
            ..FrameItem::default()
        }
    }
    pub fn partial(&self, prefix: &[String]) -> FrameItem {
        let mut argv = if prefix.is_empty() {
            vec!["herdr-threads".into()]
        } else {
            prefix.to_vec()
        };
        argv.extend(["thread".into(), "show".into(), self.thread.as_str().into()]);
        FrameItem {
            kind: "invitation".into(),
            id: self.id.as_str().into(),
            thread: Some(self.thread.as_str().into()),
            thread_name: Some(self.topic.chars().take(120).collect()),
            body: format!("…truncated; next: {}", format_command_argv(&argv)),
            ..FrameItem::default()
        }
    }
    pub fn watch(&self, prefix: &[String], partial: bool) -> WatchInvitation {
        let rendered = if partial {
            self.partial(prefix)
        } else {
            self.render(prefix)
        };
        WatchInvitation {
            topic_revision: Some(self.topic_revision.to_string()),
            command_context_epoch: Some(self.command_epoch.to_string()),
            thread: self.thread.clone(),
            thread_name: rendered.thread_name,
            body: rendered.body,
            truncated: partial,
            wakes: self.wakes(),
        }
    }
    pub fn wakes(&self) -> bool {
        self.required.is_some() || self.confirmation != Some(InvitationConfirmation::Lazy)
    }
}

/// A frozen descriptor addresses its block in one immutable framed outbox.
/// Context bodies are stored once in `text`, not duplicated inside descriptors.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FrozenInvitation {
    pub id: InvitationId,
    pub key: String,
    pub thread: ThreadId,
    pub topic_revision: String,
    pub command_epoch: String,
    pub thread_name: Option<String>,
    pub truncated: bool,
    pub wakes: bool,
    pub start: usize,
    pub end: usize,
}
impl FrozenInvitation {
    pub fn block<'a>(&self, text: &'a str) -> Option<&'a str> {
        text.get(self.start..self.end)
    }
    pub fn watch(&self, text: &str) -> Option<WatchInvitation> {
        let (_, indented) = self.block(text)?.split_once('\n')?;
        let body = indented
            .split('\n')
            .map(|line| line.strip_prefix("  ").unwrap_or(line))
            .collect::<Vec<_>>()
            .join("\n");
        Some(WatchInvitation {
            topic_revision: Some(self.topic_revision.clone()),
            command_context_epoch: Some(self.command_epoch.clone()),
            thread: self.thread.clone(),
            thread_name: self.thread_name.clone(),
            body,
            truncated: self.truncated,
            wakes: self.wakes,
        })
    }
}

pub fn block_text(item: &FrameItem) -> String {
    frame(std::slice::from_ref(item))[HEADER.len() + 2..].to_owned()
}
pub fn join_blocks(blocks: &[String]) -> String {
    if blocks.is_empty() {
        String::new()
    } else {
        format!("{HEADER}\n\n{}", blocks.join("\n\n"))
    }
}
pub fn render_key(prefix: &[String]) -> String {
    use sha2::{Digest, Sha256};
    format!(
        "{:x}",
        Sha256::digest(serde_json::to_vec(prefix).expect("string prefix serializes"))
    )
}
