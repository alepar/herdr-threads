//! Public syntax, validation, and typed dispatch preparation.

use crate::harness::cache::{CacheCursorV1, CachePageRequest, CacheRefV1};
use crate::protocol::{
    authority::CallerClaim,
    commands::{Command as WireCommand, *},
    ids::*,
    output::{ContinuationContext, OutputFormat, OutputSpec},
    pagination::{DEFAULT_PAGE_BYTES, DEFAULT_PAGE_LIMIT, PageRequest},
    results::{ApiError, CommandResult, ErrorCode},
    summary::UserIntent,
};
use clap::{ArgAction, Args, Parser, Subcommand};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedCli {
    pub actor: super::actor_route::InvocationActor,
    pub permissions: PermissionCliInputs,
    pub output: OutputSpec,
    /// Terminal presentation of text output; never sent to the daemon.
    pub presentation: crate::cli::output::Presentation,
    pub action: CliAction,
    pub cooperative: Option<CooperativeSelection>,
    /// Target/recipient locators; execution freezes IDs before journal submission.
    pub pane_selector: Option<super::panes::PaneSelector>,
    /// An omitted read selector denotes the caller, preserving own-inbox ACK eligibility.
    pub caller_read_default: bool,
    pub cooperative_selector: Option<super::panes::PaneSelector>,
    /// Raw thread selector, consumed once before any daemon dispatch or journal.
    pub thread_selector: Option<String>,
    pub require_ack_panes: Vec<super::panes::PaneSelector>,
}

/// Literal positive command prefixes. Trailing arguments remain subject to semantic parsing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OrdinaryFamily {
    pub prefix: &'static [&'static str],
    /// These flags change the action to Human and are refused on the root route.
    pub human_options: &'static [&'static str],
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RoutingToken {
    Literal(&'static str),
    StateDirectory,
    HostEndpoint,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutputPosition {
    BeforeFamily,
    AfterArguments,
}
#[derive(Debug, Clone, Copy)]
pub struct OrdinaryCatalog {
    pub families: &'static [OrdinaryFamily],
    /// Leading pinned routing forms; consumers bind slots to validated exact values.
    pub routing_forms: &'static [&'static [RoutingToken]],
    pub output_flags: &'static [&'static str],
    pub output_positions: &'static [OutputPosition],
    pub omissions: &'static [(&'static [&'static str], &'static str)],
}
pub fn ordinary_catalog() -> OrdinaryCatalog {
    OrdinaryCatalog {
        families: &[
            OrdinaryFamily {
                prefix: &["--version"],
                human_options: &[],
            },
            OrdinaryFamily {
                prefix: &["--help"],
                human_options: &[],
            },
            OrdinaryFamily {
                prefix: &["doctor", "fix"],
                human_options: &[],
            },
            OrdinaryFamily {
                prefix: &["summary", "job"],
                human_options: &[],
            },
            OrdinaryFamily {
                prefix: &["summary", "submit"],
                human_options: &[],
            },
            OrdinaryFamily {
                prefix: &["thread", "create"],
                human_options: &[],
            },
            OrdinaryFamily {
                prefix: &["thread", "topic"],
                human_options: &[],
            },
            OrdinaryFamily {
                prefix: &["thread", "name"],
                human_options: &[],
            },
            OrdinaryFamily {
                prefix: &["thread", "rename"],
                human_options: &[],
            },
            OrdinaryFamily {
                prefix: &["thread", "list"],
                human_options: &[],
            },
            OrdinaryFamily {
                prefix: &["thread", "show"],
                human_options: &[],
            },
            OrdinaryFamily {
                prefix: &["thread", "participants"],
                human_options: &[],
            },
            OrdinaryFamily {
                prefix: &["participants"],
                human_options: &[],
            },
            OrdinaryFamily {
                prefix: &["invite"],
                human_options: &["--operator"],
            },
            OrdinaryFamily {
                prefix: &["join"],
                human_options: &[],
            },
            OrdinaryFamily {
                prefix: &["accept"],
                human_options: &[],
            },
            OrdinaryFamily {
                prefix: &["reject"],
                human_options: &[],
            },
            OrdinaryFamily {
                prefix: &["accept-required"],
                human_options: &[],
            },
            OrdinaryFamily {
                prefix: &["leave"],
                human_options: &[],
            },
            OrdinaryFamily {
                prefix: &["send"],
                human_options: &[],
            },
            OrdinaryFamily {
                prefix: &["ack"],
                human_options: &[],
            },
            OrdinaryFamily {
                prefix: &["archive"],
                human_options: &[],
            },
            OrdinaryFamily {
                prefix: &["reopen"],
                human_options: &[],
            },
            OrdinaryFamily {
                prefix: &["inbox"],
                human_options: &[],
            },
            OrdinaryFamily {
                prefix: &["warnings"],
                human_options: &[],
            },
            OrdinaryFamily {
                prefix: &["check-in"],
                human_options: &[],
            },
            OrdinaryFamily {
                prefix: &["cached-check-in"],
                human_options: &[],
            },
            OrdinaryFamily {
                prefix: &["pending-receipts"],
                human_options: &[],
            },
            OrdinaryFamily {
                prefix: &["read"],
                human_options: &[],
            },
            OrdinaryFamily {
                prefix: &["follow"],
                human_options: &[],
            },
            OrdinaryFamily {
                prefix: &["body"],
                human_options: &[],
            },
            OrdinaryFamily {
                prefix: &["search"],
                human_options: &[],
            },
            OrdinaryFamily {
                prefix: &["seat", "list"],
                human_options: &[],
            },
            OrdinaryFamily {
                prefix: &["seat", "inspect"],
                human_options: &[],
            },
            OrdinaryFamily {
                prefix: &["seat", "resolve"],
                human_options: &["--operator", "--new-seat"],
            },
            OrdinaryFamily {
                prefix: &["delivery", "recipients"],
                human_options: &[],
            },
            OrdinaryFamily {
                prefix: &["delivery", "inspect"],
                human_options: &[],
            },
            OrdinaryFamily {
                prefix: &["overdue"],
                human_options: &[],
            },
            OrdinaryFamily {
                prefix: &["diagnostics"],
                human_options: &[],
            },
            OrdinaryFamily {
                prefix: &["daemon", "health"],
                human_options: &[],
            },
            OrdinaryFamily {
                prefix: &["daemon", "ensure"],
                human_options: &[],
            },
            OrdinaryFamily {
                prefix: &["daemon", "stop"],
                human_options: &[],
            },
            OrdinaryFamily {
                prefix: &["service", "inspect"],
                human_options: &[],
            },
            OrdinaryFamily {
                prefix: &["doctor"],
                human_options: &[],
            },
            OrdinaryFamily {
                prefix: &["setup"],
                human_options: &[],
            },
            OrdinaryFamily {
                prefix: &["unsetup"],
                human_options: &[],
            },
            OrdinaryFamily {
                prefix: &["setup-status"],
                human_options: &[],
            },
            OrdinaryFamily {
                prefix: &["pending-ops"],
                human_options: &[],
            },
            OrdinaryFamily {
                prefix: &["retry"],
                human_options: &[],
            },
            OrdinaryFamily {
                prefix: &["view"],
                human_options: &[],
            },
            OrdinaryFamily {
                prefix: &["launch"],
                human_options: &[],
            },
            OrdinaryFamily {
                prefix: &["handoff"],
                human_options: &[],
            },
            OrdinaryFamily {
                prefix: &["skill"],
                human_options: &[],
            },
            OrdinaryFamily {
                prefix: &["--skill"],
                human_options: &[],
            },
            OrdinaryFamily {
                prefix: &["summary"],
                human_options: &[],
            },
            OrdinaryFamily {
                prefix: &["contract-id"],
                human_options: &[],
            },
            OrdinaryFamily {
                prefix: &["harness-version", "normalize"],
                human_options: &[],
            },
            OrdinaryFamily {
                prefix: &["internal", "json-field"],
                human_options: &[],
            },
            OrdinaryFamily {
                prefix: &["internal", "installer-integrations"],
                human_options: &[],
            },
        ],
        routing_forms: &[
            &[],
            &[
                RoutingToken::Literal("--state-dir"),
                RoutingToken::StateDirectory,
            ],
            &[
                RoutingToken::Literal("--host-endpoint"),
                RoutingToken::HostEndpoint,
            ],
            &[
                RoutingToken::Literal("--state-dir"),
                RoutingToken::StateDirectory,
                RoutingToken::Literal("--host-endpoint"),
                RoutingToken::HostEndpoint,
            ],
            &[
                RoutingToken::Literal("--host-endpoint"),
                RoutingToken::HostEndpoint,
                RoutingToken::Literal("--state-dir"),
                RoutingToken::StateDirectory,
            ],
        ],
        output_flags: &["--human", "--machine", "--json"],
        output_positions: &[OutputPosition::BeforeFamily, OutputPosition::AfterArguments],
        omissions: &[(
            &["seat", "retirements"],
            "operator cleanup inventory omitted from native ordinary coverage",
        )],
    }
}

/// Syntactic inputs only; neither executable ownership nor consent attestation.
#[derive(Debug, Clone, PartialEq, Eq, Default, Args)]
pub struct PermissionCliInputs {
    #[arg(long)]
    pub permissions: bool,
    #[arg(long, conflicts_with = "without_permissions")]
    pub with_permissions: bool,
    #[arg(long)]
    pub without_permissions: bool,
    #[arg(long, value_parser = permission_path)]
    pub permission_installed_binary: Option<String>,
    #[arg(long, requires = "permission_installed_binary", value_parser = permission_path)]
    pub permission_link_path: Option<String>,
    #[arg(long, requires = "permission_installed_binary", value_parser = permission_path)]
    pub permission_alias_path: Option<String>,
}
fn permission_path(value: &str) -> Result<String, String> {
    if value.is_empty()
        || value.len() > 4096
        || value.chars().any(char::is_control)
        || !std::path::Path::new(value).is_absolute()
    {
        return Err("permission inventory must be a nonempty absolute path without controls, at most 4096 UTF-8 bytes".into());
    }
    Ok(value.to_owned())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CooperativeSelection {
    pub seat: SeatId,
    pub target: HostTargetId,
    pub harness: crate::harness::context::Harness,
    pub role: crate::harness::context::Role,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CliAction {
    Wire(WireCommand),
    Mutation(MutationSpec),
    PendingOps(PageRequest),
    Retry(LocalRecoveryRef),
    View {
        once: bool,
        page: PageRequest,
    },
    Daemon(DaemonAction),
    /// Diagnose local state; `fix` attempts bounded owned repairs.
    Doctor {
        debug: bool,
        fix: bool,
    },
    CachedCheckIn(CachePageRequest),
    /// Local harness hook setup, removal or inspection; never contacts the daemon.
    Setup(super::setup::SetupRequest),
    /// `setup`, `unsetup` or `setup-status` with no harness named: every
    /// harness (setup: every detected one); local, never contacts the daemon.
    SetupAll(
        super::setup::SetupVerb,
        super::setup::PromptSuggestionPolicy,
    ),
    /// Managed native launch into one explicit existing empty shell pane.
    Launch(super::launch::LaunchRequest),
    Handoff(super::handoff::HandoffRequest),
    /// `me init`: record the invoking pane as the person's own seat identity.
    /// `operator` overrides the agent-to-human guard (TRUST-POLICY A4).
    MeInit {
        operator: bool,
    },
    /// Print the embedded agent skill (`skill` or `--skill`); local only.
    Skill,
    /// `contract-id [--harness H]`: the native hook payload contract ids;
    /// local only.
    ContractId {
        harness: Option<crate::harness::context::Harness>,
    },
    /// Hidden `watch --harness claude --session ID`: the mod delivery child.
    Watch(super::watch::WatchRequest),
    /// Hidden `watch ack --session ID --via VIA IDS...`.
    WatchAck(super::watch::WatchAckRequest),
    /// `harness-version normalize HARNESS RAW`: the canonical bare semver of a
    /// raw `--version` string; local only.
    HarnessVersionNormalize {
        harness: crate::harness::context::Harness,
        raw: String,
    },
    /// Hidden `internal json-field PATH`: print a field of the JSON on stdin.
    InstallerIntegrations {
        confirm_missing: bool,
    },
    InternalJsonField {
        path: String,
    },
    /// `read THREAD --follow`: recent messages, then each new one as it is
    /// committed. Read-only: never ACKs or accepts.
    Follow(FollowRequest),
    /// Human terminal discovery; never sent to the daemon.
    Picker(PickerRequest),
    /// `summary THREAD`, `summary job`, `summary submit`: act as the invoking
    /// seat (cooperative claim from its saved context), never journaled.
    Summary(super::summary::SummaryCli),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PickerRequest {
    pub page: PageRequest,
    pub initial: Option<HistoryRange>,
    pub follow: bool,
    pub no_system: bool,
}
impl PickerRequest {
    pub fn selected(&self, thread: ThreadId) -> CliAction {
        if self.follow {
            CliAction::Follow(FollowRequest {
                thread,
                recent: match self.initial {
                    Some(HistoryRange::Recent { count }) => count,
                    _ => 0,
                },
                after: match self.initial {
                    Some(HistoryRange::After { sequence }) => Some(sequence),
                    _ => None,
                },
                no_system: self.no_system,
            })
        } else {
            CliAction::Wire(WireCommand::History(HistoryQuery {
                thread,
                page: self.page.clone(),
                initial: self.initial.clone(),
                full_bodies: false,
            }))
        }
    }
}

/// `read THREAD --follow` options.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FollowRequest {
    pub thread: ThreadId,
    /// Recent messages printed before following (0: only new ones).
    pub recent: u16,
    /// Start after this timeline sequence instead of the recent tail.
    pub after: Option<u64>,
    /// Hide system notices (joins, ACKs, warnings).
    pub no_system: bool,
}

/// Recent messages `read --follow` prints before following.
pub const FOLLOW_DEFAULT_RECENT: u16 = 20;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DaemonAction {
    Health,
    Ensure,
    Stop,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LocalAction {
    PendingOps(PageRequest),
    Retry(LocalRecoveryRef),
    View { once: bool, page: PageRequest },
    Daemon(DaemonAction),
    Doctor,
    CachedCheckIn(CachePageRequest),
}

/// Application composition supplies transport, journal, daemon lifecycle and
/// view services. A stub can assert the exact typed request without a daemon.
pub trait CliBackend {
    fn call(
        &mut self,
        command: WireCommand,
        output: &OutputSpec,
    ) -> Result<CommandResult, ApiError>;
    fn local(
        &mut self,
        _action: LocalAction,
        _output: &OutputSpec,
    ) -> Result<CommandResult, ApiError> {
        Err(ApiError::unsupported("local command backend unavailable"))
    }
}

pub fn dispatch<B: CliBackend>(
    parsed: ParsedCli,
    backend: &mut B,
    claim: Option<CallerClaim>,
    operation: Option<OperationId>,
) -> Result<CommandResult, ApiError> {
    if parsed.pane_selector.as_ref().is_some_and(|selector| {
        selector.direct_id().is_none()
            || matches!(
                parsed.action,
                CliAction::Wire(_) | CliAction::Mutation(MutationSpec::Invite { .. })
            )
    }) || !parsed.require_ack_panes.is_empty()
        || parsed
            .cooperative_selector
            .as_ref()
            .is_some_and(|selector| selector.direct_id().is_none())
    {
        return Err(invalid(
            "pane selectors require runtime resolution before dispatch",
        ));
    }
    if parsed.thread_selector.is_some() {
        return Err(invalid(
            "thread selectors require runtime resolution before dispatch",
        ));
    }
    let command = match parsed.action {
        CliAction::Wire(command) => command,
        CliAction::Mutation(mutation) => mutation.into_command(
            claim,
            operation.ok_or_else(|| invalid("durable operation key required"))?,
        )?,
        CliAction::PendingOps(page) => {
            return backend.local(LocalAction::PendingOps(page), &parsed.output);
        }
        CliAction::Retry(reference) => {
            return backend.local(LocalAction::Retry(reference), &parsed.output);
        }
        CliAction::View { once, page } => {
            return backend.local(LocalAction::View { once, page }, &parsed.output);
        }
        CliAction::Daemon(action) => {
            return backend.local(LocalAction::Daemon(action), &parsed.output);
        }
        CliAction::Doctor { .. } => return backend.local(LocalAction::Doctor, &parsed.output),
        CliAction::CachedCheckIn(request) => {
            return backend.local(LocalAction::CachedCheckIn(request), &parsed.output);
        }
        CliAction::Setup(_)
        | CliAction::SetupAll(..)
        | CliAction::Launch(_)
        | CliAction::Handoff(_)
        | CliAction::MeInit { .. }
        | CliAction::Skill
        | CliAction::ContractId { .. }
        | CliAction::Watch(_)
        | CliAction::WatchAck(_)
        | CliAction::HarnessVersionNormalize { .. }
        | CliAction::InstallerIntegrations { .. }
        | CliAction::InternalJsonField { .. }
        | CliAction::Picker(_)
        | CliAction::Follow(_)
        | CliAction::Summary(_) => {
            return Err(ApiError::unsupported(
                "setup, launch and skill are local compositions without a daemon backend",
            ));
        }
    };
    backend.call(command, &parsed.output)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MutationSpec {
    Resolve(HostTargetId),
    Create {
        name: Option<String>,
        topic: String,
        goal: String,
    },
    Name {
        thread: ThreadId,
        name: Option<String>,
    },
    Topic {
        thread: ThreadId,
        topic: String,
    },
    Invite {
        thread: ThreadId,
        seat: SeatId,
        deadline_millis: Option<u64>,
        operator: bool,
    },
    Join(ThreadId),
    Accept(ThreadId),
    Reject {
        thread: ThreadId,
        invitation: InvitationId,
        reason: String,
    },
    AcceptRequired {
        thread: ThreadId,
        invitation: InvitationId,
        requirement: RequirementId,
        expected_revision: u64,
    },
    Leave(ThreadId),
    Send {
        delivery_mode: DeliveryMode,
        thread: ThreadId,
        body: String,
        require_ack: Vec<SeatId>,
        deadline_millis: Option<u64>,
        relays_user: bool,
        user_intent: Option<UserIntent>,
    },
    Ack(Vec<MessageId>),
    Archive(ThreadId),
    Reopen(ThreadId),
    CheckIn,
    CheckInLifecycle {
        event_id: String,
        native_session: Option<String>,
        /// `me init --operator`: submitted as `Command::OperatorCheckIn`.
        operator: bool,
    },
    Rebind {
        seat: SeatId,
        pane: HostTargetId,
    },
    FreshSeat(HostTargetId),
    /// `seat retire SEAT --operator`: abandon a seat.
    Retire(SeatId),
    /// `seat rebind OLD --pane P --replace NEW --operator`.
    Replace {
        seat: SeatId,
        pane: HostTargetId,
        replace: SeatId,
    },
}

impl MutationSpec {
    /// Compose a parsed mutation with the journal's stable key and invocation
    /// claim. The claim is only a hint; the service proves authority afresh.
    pub fn into_command(
        self,
        claim: Option<CallerClaim>,
        operation: OperationId,
    ) -> Result<WireCommand, ApiError> {
        let needs_claim = !matches!(
            self,
            Self::Resolve(_)
                | Self::Rebind { .. }
                | Self::Retire(_)
                | Self::Replace { .. }
                | Self::FreshSeat(_)
                | Self::Invite { operator: true, .. }
        );
        let claim = if needs_claim {
            Some(claim.ok_or_else(|| invalid("native caller context required"))?)
        } else {
            None
        };
        let command = match self {
            Self::Resolve(target) => WireCommand::ResolveSeat(ResolveSeat { target, operation }),
            Self::Create { name, topic, goal } => WireCommand::CreateThread(CreateThread {
                name,
                topic,
                goal,
                operation,
                claim: claim.unwrap(),
            }),
            Self::Name { thread, name } => WireCommand::SetThreadName(SetThreadName {
                thread,
                name,
                operation,
                claim: claim.unwrap(),
            }),
            Self::Topic { thread, topic } => WireCommand::SetTopic(SetTopic {
                thread,
                topic,
                operation,
                claim: claim.unwrap(),
            }),
            Self::Invite {
                thread,
                seat,
                deadline_millis,
                operator: false,
            } => WireCommand::Invite(Invite {
                thread,
                seat,
                deadline_millis,
                operation,
                claim: claim.unwrap(),
            }),
            Self::Invite {
                thread,
                seat,
                deadline_millis,
                operator: true,
            } => WireCommand::OperatorOrphanInvite(OperatorOrphanInvite {
                thread,
                seat,
                deadline_millis,
                operation,
            }),
            Self::Reject {
                thread,
                invitation,
                reason,
            } => WireCommand::Reject(Reject {
                thread,
                invitation,
                reason,
                operation,
                claim: claim.unwrap(),
            }),
            Self::Join(thread) => WireCommand::Join(Join {
                thread,
                operation,
                claim: claim.unwrap(),
            }),
            Self::Accept(thread) => WireCommand::Accept(Accept {
                thread,
                operation,
                claim: claim.unwrap(),
            }),
            Self::AcceptRequired {
                thread,
                invitation,
                requirement,
                expected_revision,
            } => WireCommand::AcceptRequired(AcceptRequired {
                thread,
                invitation,
                requirement,
                expected_revision,
                operation,
                claim: claim.unwrap(),
            }),
            Self::Leave(thread) => WireCommand::Leave(Leave {
                thread,
                operation,
                claim: claim.unwrap(),
            }),
            Self::Send {
                delivery_mode,
                thread,
                body,
                require_ack,
                deadline_millis,
                relays_user,
                user_intent,
            } => WireCommand::SendMessage(SendMessage {
                delivery_mode,
                thread,
                body,
                invited_recipients: require_ack,
                deadline_millis,
                operation,
                claim: claim.unwrap(),
                relays_user,
                user_intent,
            }),
            Self::Ack(messages) => WireCommand::Ack(Ack {
                messages,
                operation,
                claim: claim.unwrap(),
            }),
            Self::Archive(thread) => WireCommand::Archive(ThreadMutation {
                thread,
                operation,
                claim: claim.unwrap(),
            }),
            Self::Reopen(thread) => WireCommand::Reopen(ThreadMutation {
                thread,
                operation,
                claim: claim.unwrap(),
            }),
            Self::CheckInLifecycle { .. } => {
                return Err(invalid(
                    "explicit lifecycle requires durable context bridge",
                ));
            }
            Self::CheckIn => WireCommand::CheckIn(CheckIn {
                mode: crate::protocol::commands::CheckInMode::Current,
                claim: claim.unwrap(),
                operation,
            }),
            Self::Rebind { seat, pane } => WireCommand::OperatorRebind(OperatorRebind {
                seat,
                target: pane,
                operation,
            }),
            Self::Retire(seat) => {
                WireCommand::OperatorRetire(crate::protocol::commands::OperatorRetire {
                    seat,
                    operation,
                })
            }
            Self::Replace {
                seat,
                pane,
                replace,
            } => WireCommand::OperatorReplace(crate::protocol::commands::OperatorReplace {
                seat,
                target: pane,
                replace,
                operation,
            }),
            Self::FreshSeat(pane) => WireCommand::OperatorFreshSeat(OperatorFreshSeat {
                target: pane,
                operation,
            }),
        };
        command.validate().map_err(invalid)?;
        Ok(command)
    }
}

#[derive(Parser)]
#[command(
    name = "herdr-threads",
    version,
    after_help = format!("Examples:\n  herdr-threads inbox\n  herdr-threads read THREAD\n  herdr-threads send THREAD --body 'Hello'\n  herdr-threads ack MESSAGE\n\n{}\n\n{}", exit_status_help(), super::skill::AI_HELP_FOOTER),
    about = "Person/operator commands require immediate `human`: ht human [GLOBALS] COMMAND. --human only selects output. Read threads and explicitly ACK exact message IDs. Accept invitations separately. Optional cheap subagents can summarize recent or full history without ACK authority."
)]
struct Cli {
    /// Select the local herdr-threads state directory. May repeat with an
    /// identical value; conflicting values are refused.
    #[arg(long, global = true, action = ArgAction::Append)]
    state_dir: Vec<String>,
    /// Select the local Herdr host socket path. May repeat with an identical
    /// value; conflicting values are refused.
    #[arg(long, global = true, action = ArgAction::Append)]
    host_endpoint: Vec<String>,
    /// Emit a structured JSON result on stdout.
    #[arg(long, global = true, action = ArgAction::SetTrue)]
    json: bool,
    /// Human-readable tables and transcripts (the default on a terminal).
    #[arg(long, global = true, action = ArgAction::SetTrue, conflicts_with_all = ["json", "machine"])]
    human: bool,
    /// The stable `key: value` text form for scripts and models (the default
    /// when stdout is not a terminal).
    #[arg(long, global = true, action = ArgAction::SetTrue, conflicts_with = "json")]
    machine: bool,
    /// Explicit seat ID for a cooperative agent call; requires all four
    /// --cooperative-* options.
    #[arg(long, global = true)]
    cooperative_seat: Option<String>,
    /// Exact live pane ID or scoped name bound to the cooperative seat.
    #[arg(long, global = true)]
    cooperative_target: Option<String>,
    /// Agent harness making the cooperative call.
    #[arg(long, global = true, value_parser = ["codex", "claude"])]
    cooperative_harness: Option<String>,
    /// Agent role for the cooperative call.
    #[arg(long, global = true, value_parser = ["top-level", "subagent"])]
    cooperative_role: Option<String>,
    #[command(subcommand)]
    command: Top,
}

#[derive(Subcommand)]
enum Top {
    /// Create, list and inspect threads and their participants.
    Thread {
        #[command(subcommand)]
        command: ThreadSub,
    },
    /// Invite a seat to join a thread.
    Invite(InviteArgs),
    /// Same as `thread participants THREAD` (the caller's own seat is marked
    /// `"self": true`).
    Participants {
        thread: String,
        #[command(flatten)]
        page: PageArgs,
    },
    /// Join an active thread without an invitation (THREAD is a name or ID).
    /// Pending invitations require accept or accept-required instead.
    Join { thread: String },
    /// Accept an invitation to a thread.
    Accept { thread: String },
    /// Reject an exact ordinary invitation with an explicit reason.
    Reject {
        thread: String,
        #[arg(long)]
        invitation: String,
        #[arg(long)]
        reason: String,
    },
    /// Accept a service-required invitation with its exact revision.
    AcceptRequired {
        thread: String,
        #[arg(long)]
        invitation: String,
        #[arg(long)]
        requirement: String,
        #[arg(long)]
        revision: u64,
    },
    /// Leave a thread without changing other participants.
    Leave { thread: String },
    /// Send a message to a thread, with optional explicit ACK recipients.
    Send(SendArgs),
    /// ACK exact message IDs after reading them.
    Ack {
        #[arg(required = true, num_args = 1..)]
        messages: Vec<String>,
    },
    /// Archive a thread.
    Archive { thread: String },
    /// Reopen an archived thread.
    Reopen { thread: String },
    /// Display compact inbox bodies; default text ACKs fully displayed pending agent receipts after flush. --machine, --json and --seat remain read-only.
    Inbox(InboxArgs),
    /// List warning history for a seat, or active warning conditions for one thread.
    Warnings(WarningsArgs),
    /// Check in so pending work can be delivered to this seat.
    CheckIn(CheckInArgs),
    /// Read a cached check-in page by its reference.
    CachedCheckIn(CachedCheckInArgs),
    /// List messages still waiting for ACKs.
    PendingReceipts(PendingReceiptsArgs),
    /// Read thread history or follow new messages; reading never ACKs.
    #[command(
        after_help = "Without THREAD, browse channels across this instance, including archives and nonmembers. Rows show active/archived status, joined participants, sampled messages/minute and the last message; active channels rank first by participants weighted with activity. Type a fuzzy name/topic filter; arrows or Ctrl-N/P move, Enter reads the canonical ID, Esc/Ctrl-C cancel with exit 0. Requires stdin/stdout/stderr TTYs, usable TERM, and no agent/cooperative caller, --machine or --json. Agents and scripts must supply an exact thread ID or unique name.\n\nAgent author/recipient nicknames are SPACE/PANE/SEAT_ID with no harness suffix. Human nicknames are relative to the live caller: alice, tryout/alice, project/tryout/alice. History and follow never ACK or accept."
    )]
    Read(ReadArgs),
    /// Follow new messages, shorthand for `read --follow`; never ACKs or accepts.
    #[command(
        after_help = "Without THREAD, choose a channel in the same human terminal picker as read --follow. Rows show active/archived status, joined participants, sampled messages/minute and the last message. Type a fuzzy name/topic filter; arrows or Ctrl-N/P move, Enter follows the canonical ID, Esc/Ctrl-C cancel with exit 0. Requires stdin/stdout/stderr TTYs, usable TERM, and no agent/cooperative caller, --machine or --json. Agents and scripts must supply an exact thread ID or unique name. History and follow never ACK or accept."
    )]
    Follow(FollowArgs),
    /// Read a message body, including pages beyond its preview.
    Body(BodyArgs),
    /// Search for literal text in thread topics and messages.
    Search(SearchArgs),
    /// Inspect and repair durable seat mappings.
    Seat {
        #[command(subcommand)]
        command: SeatSub,
    },
    /// Inspect who received a message and each delivery state.
    Delivery {
        #[command(subcommand)]
        command: DeliverySub,
    },
    /// List overdue obligations across seats and threads.
    Overdue(PageArgs),
    /// Read service diagnostics, optionally filtered by seat or thread.
    Diagnostics(DiagnosticsArgs),
    /// Check, start or stop the local herdr-threads daemon.
    Daemon {
        #[command(subcommand)]
        command: DaemonSub,
    },
    /// Inspect or disconnect the live service connection for recovery.
    #[command(
        after_help = "Recovery workflow:\n  herdr-threads service inspect\n  herdr-threads human service disconnect --expected-boot BOOT --expected-generation GENERATION\n\nUse the boot and generation returned by inspect. Disconnect revokes that exact connection; it does not stop the daemon."
    )]
    Service {
        #[command(subcommand)]
        command: ServiceSub,
    },
    /// Check daemon, harness hooks and local state; lead with judgments and fixes.
    Doctor {
        /// Print the full diagnostic inventory.
        #[arg(long, global = true)]
        debug: bool,
        #[command(subcommand)]
        command: Option<DoctorSub>,
    },
    /// Install the owned herdr-threads hooks for a harness at user level
    /// (with no harness: for every detected harness).
    /// Claude: `$CLAUDE_CONFIG_DIR/settings.json` (hooks and allow rule).
    /// Codex: `$CODEX_HOME/hooks.json`. Ownership manifests stay private in
    /// plugin state. Setup uses the declared hook contract; installed version
    /// metadata is optional and may be unknown. Configured hooks do not prove
    /// observed delivery or native harness support.
    #[command(after_help = super::setup::SETUP_HELP)]
    Setup(SetupArgs),
    /// Remove the owned herdr-threads hooks installed by `setup`, keeping
    /// every other setting; unchanged settings are restored byte for byte.
    /// Removal does not depend on installed harness metadata.
    #[command(after_help = super::setup::SETUP_HELP)]
    Unsetup(SetupArgs),
    /// Report whether the owned hooks are installed (separately from native
    /// observation), with optional installed version metadata. Installation
    /// does not prove native callback delivery or launch support.
    #[command(after_help = super::setup::SETUP_HELP)]
    SetupStatus(SetupArgs),
    /// List local mutations whose outcome is still unknown.
    PendingOps(PageArgs),
    /// Retry a local recovery reference from pending-ops.
    Retry { recovery_ref: String },
    /// Show a local attention view of threads and actions.
    View(ViewArgs),
    /// Start Codex or Claude in one explicit existing empty shell pane with
    /// the owned hooks, after resolving the pane's seat. Launch never
    /// registers, accepts or ACKs: the handoff is read through the hooks.
    #[command(after_help = super::launch::LAUNCH_HELP)]
    Launch(LaunchArgs),
    /// Deliver one durable task, then start an agent in an explicit pane.
    #[command(after_help = super::handoff::HANDOFF_HELP)]
    Handoff(HandoffArgs),
    /// Your own identity as a person in this Herdr pane.
    Me {
        #[command(subcommand)]
        command: MeSub,
    },
    /// Print the agent skill (SKILL.md): how agents should use herdr-threads.
    /// Also available as `--skill`. Local only; never contacts the daemon.
    #[command(long_flag = "skill")]
    Skill,
    /// Thread summary for compaction recovery: returns the summary when
    /// ready, else jobs for summary workers (see the 'Thread summaries'
    /// section of `herdr-threads skill`).
    #[command(args_conflicts_with_subcommands = true)]
    Summary(SummaryArgs),
    /// Mod delivery child: streams herdr-threads deliveries as JSON lines.
    #[command(
        hide = true,
        args_conflicts_with_subcommands = true,
        after_help = "Exit codes: 0 stream ended (restart with backoff), 1 other error \
(restart with backoff), 2 refused (retry after backoff), 3 permanent (stop until \
reload). Registration refusals no_binding, session_mismatch, held, unresolved, \
cooldown, busy and stopping exit 2; not_claude and disabled exit 3. A Close of \
disabled or replaced exits 3, any other Close exits 0. Local reasons no_pane, \
env_disabled and unsupported exit 3; daemon_unavailable and error exit 1; \
stream_ended exits 0. Spawned by the herdr-threads Claude mod."
    )]
    Watch(WatchArgs),
    /// Print the contract id of each harness's native hook payload (the
    /// declared event kinds, required fields and JSON types the hook parsers
    /// consume). With `--json`: `{"claude": ID, "codex": ID, "normalize":
    /// {...}}`. Local only; never contacts the daemon.
    ContractId {
        /// Print only this harness.
        #[arg(long, value_parser = ["claude", "codex"])]
        harness: Option<String>,
    },
    /// Harness version helpers shared with the compatibility canary. Local
    /// only; never contacts the daemon.
    HarnessVersion {
        #[command(subcommand)]
        command: HarnessVersionSub,
    },
    /// Helpers for scripts/install.sh; not part of the public interface.
    #[command(hide = true)]
    Internal {
        #[command(subcommand)]
        command: InternalSub,
    },
}

#[derive(Args)]
struct WatchArgs {
    #[arg(long, value_parser = ["claude"])]
    harness: Option<String>,
    #[arg(long)]
    session: Option<String>,
    #[command(subcommand)]
    command: Option<WatchSub>,
}

#[derive(Subcommand)]
enum WatchSub {
    /// Report items the mod delivered; one JSON line per id.
    Ack {
        #[arg(long)]
        session: String,
        #[arg(long, value_parser = ["context", "submit", "append"])]
        via: String,
        #[arg(required = true)]
        ids: Vec<String>,
    },
}

#[derive(Subcommand)]
enum DoctorSub {
    /// Attempt safe, owned repairs and report remaining manual actions.
    Fix,
}

#[derive(Args)]
struct SummaryArgs {
    thread: Option<String>,
    #[command(subcommand)]
    command: Option<SummarySub>,
}

#[derive(Subcommand)]
enum SummarySub {
    /// Print the bundle for a leased summary job as JSON (or that its
    /// reservation lapsed).
    Job {
        job: String,
        #[arg(long)]
        lease: String,
    },
    /// Submit a summary for a leased job: the submission JSON on stdin (at
    /// most 64 KiB); prints Stored or the Rejected reasons.
    Submit {
        job: String,
        #[arg(long)]
        lease: String,
    },
}

#[derive(Subcommand)]
enum HarnessVersionSub {
    /// Print the canonical bare `MAJOR.MINOR.PATCH` of a raw `--version`
    /// string (for example `codex-cli 0.158.0` or `2.1.286 (Claude Code)`);
    /// exits non-zero for a string it does not recognize.
    Normalize {
        #[arg(value_parser = ["claude", "codex"])]
        harness: String,
        #[arg(allow_hyphen_values = true)]
        raw: String,
    },
}

#[derive(Subcommand)]
enum InternalSub {
    /// Reconcile supported harness hooks and skills, asking separately for missing components.
    InstallerIntegrations {
        /// Explicitly confirm installation of every missing integration (installer --setup).
        #[arg(long)]
        confirm_missing: bool,
        #[command(flatten)]
        permissions: PermissionCliInputs,
    },
    /// Read JSON on stdin and print the value at a dotted path (exit 1 when absent).
    JsonField { path: String },
}

#[derive(Subcommand)]
enum MeSub {
    /// Record this pane (HERDR_PANE_ID) as your own seat, with operator/human
    /// provenance, so `thread create`, `invite`, `send`, `read`, `ack` and
    /// `accept` run here with no caller flags. Re-run after a daemon restart.
    #[command(after_help = super::me::ME_INIT_HELP)]
    Init {
        /// Override the agent guard as the local account: record yourself over
        /// an agent's binding or where agent markers are present.
        #[arg(long)]
        operator: bool,
    },
}

#[derive(Subcommand)]
enum ThreadSub {
    /// Create a thread with a topic, optional goal and exact name.
    Create {
        /// Optional exact name (1..128 UTF-8 bytes, no controls; duplicates allowed).
        #[arg(long)]
        name: Option<String>,
        /// Initial topic shown in the thread directory.
        #[arg(long)]
        topic: String,
        /// Goal for participants (default: the topic).
        #[arg(long)]
        goal: Option<String>,
    },
    /// Show a thread's topic, or change it with --set.
    Topic {
        thread: String,
        #[arg(long = "set")]
        set: Option<String>,
    },
    /// Show a thread name, set it, or clear it (THREAD accepts an exact ID or name).
    Name {
        thread: String,
        #[arg(long, conflicts_with = "clear")]
        set: Option<String>,
        #[arg(long)]
        clear: bool,
    },
    /// Alias for thread name THREAD --set NAME.
    Rename { thread: String, name: String },
    /// List threads, optionally filtered by membership or topic text.
    List(ThreadListArgs),
    /// Show a thread's topic, state and recent details.
    Show {
        thread: String,
        #[command(flatten)]
        page: PageArgs,
    },
    /// List the seats participating in a thread.
    Participants {
        thread: String,
        #[command(flatten)]
        page: PageArgs,
    },
}
#[derive(Subcommand)]
enum SeatSub {
    /// List current seats, newest first. Use --include-retired for history.
    List {
        #[arg(long, help = "Include retired seats in the list")]
        include_retired: bool,
        #[command(flatten)]
        page: PageArgs,
    },
    /// Resolve a pane to its seat; --new-seat creates a fresh operator seat.
    Resolve {
        #[command(flatten)]
        selector: super::panes::PaneSelector,
        #[arg(long, requires = "operator")]
        new_seat: bool,
        #[arg(long, requires = "new_seat")]
        operator: bool,
    },
    /// Inspect one seat's mapping, binding and obligations.
    Inspect {
        seat: String,
        #[command(flatten)]
        page: PageArgs,
    },
    /// Rebind a seat to a pane through explicit operator repair.
    Rebind {
        seat: String,
        #[command(flatten)]
        selector: super::panes::PaneSelector,
        /// Retire the seat that owns the pane (NEW) and rebind this seat onto
        /// the pane in one step; NEW's pending obligations settle as
        /// recipient-retired and nothing moves from NEW to this seat.
        #[arg(long, value_name = "NEW")]
        replace: Option<String>,
        #[arg(long, required = true)]
        operator: bool,
    },
    /// Abandon a seat: it retires and its pending obligations settle as
    /// recipient-retired. The seat is never merged into another.
    Retire {
        seat: String,
        #[arg(long)]
        operator: bool,
    },
    /// List retired seats and their cleanup progress.
    Retirements(PageArgs),
}
#[derive(Subcommand)]
enum DeliverySub {
    /// List the recipients of a message and their receipt states.
    Recipients {
        message: String,
        #[command(flatten)]
        page: PageArgs,
    },
    /// Inspect one message's delivery and ACK details.
    Inspect {
        message: String,
        #[command(flatten)]
        page: PageArgs,
    },
}
#[derive(Subcommand)]
enum DaemonSub {
    /// Show the local daemon's health and limitations.
    Health,
    /// Ensure the local daemon is running.
    Ensure,
    /// Stop the local daemon.
    Stop,
}
#[derive(Subcommand)]
enum ServiceSub {
    /// Observe the live service connection and its generation; does not attest agent liveness.
    Inspect,
    /// Revoke one observed service connection without stopping the daemon.
    #[command(
        after_help = "First run `herdr-threads service inspect`, then pass its daemon_boot and connection_generation values. A stale boot or generation is refused."
    )]
    Disconnect {
        /// Daemon boot ID observed by `service inspect`.
        #[arg(long)]
        expected_boot: String,
        /// Connection generation observed by `service inspect`.
        #[arg(long)]
        expected_generation: u64,
    },
}

#[derive(Args)]
struct CheckInArgs {
    /// Stable external identity persisted by the launch driver before invocation.
    #[arg(long)]
    lifecycle_event: Option<String>,
    /// Actual native reference, if present; absence uses tagged plugin context.
    #[arg(long, requires = "lifecycle_event")]
    native_session: Option<String>,
}
#[derive(Args)]
struct CachedCheckInArgs {
    #[arg(long)]
    reference: String,
    #[arg(long)]
    cursor: Option<String>,
    #[arg(long)]
    max_bytes: Option<u32>,
}
#[derive(Args)]
struct PageArgs {
    /// Continue from a cursor returned by an earlier page.
    #[arg(long)]
    cursor: Option<String>,
    /// Maximum items in this page (default: 20).
    #[arg(long)]
    limit: Option<u16>,
    /// Maximum encoded bytes in this page (default: 16384).
    #[arg(long)]
    max_bytes: Option<u32>,
}
#[derive(Args)]
struct ThreadListArgs {
    /// Order by latest committed timeline activity.
    #[arg(long)]
    recent: bool,
    #[command(flatten)]
    selector: super::panes::PaneSelector,
    #[arg(long)]
    seat: Option<String>,
    #[arg(long, group = "membership")]
    joined: bool,
    #[arg(long, group = "membership")]
    invited: bool,
    #[arg(long, group = "membership")]
    all: bool,
    #[arg(long)]
    search: Option<String>,
    #[command(flatten)]
    page: PageArgs,
}
#[derive(Args)]
struct InboxArgs {
    #[command(flatten)]
    selector: super::panes::PaneSelector,
    #[arg(long)]
    seat: Option<String>,
    #[command(flatten)]
    page: PageArgs,
}
#[derive(Args)]
struct InviteArgs {
    #[command(flatten)]
    selector: super::panes::PaneSelector,
    /// Thread to invite the seat into.
    thread: String,
    /// Seat ID to invite.
    #[arg(long)]
    seat: Option<String>,
    /// Invitation deadline in positive seconds.
    #[arg(long)]
    deadline: Option<u64>,
    /// Mark this as an explicit operator invitation.
    #[arg(long)]
    operator: bool,
}
#[derive(Args)]
struct SendArgs {
    #[command(flatten)]
    selector: super::panes::PaneSelector,
    /// Thread that receives the message.
    thread: String,
    /// Message body given directly on the command line.
    #[arg(long, group = "body_source")]
    body: Option<String>,
    /// Read the message body from this file.
    #[arg(long, group = "body_source")]
    file: Option<String>,
    /// Read the message body from standard input.
    #[arg(long, group = "body_source")]
    stdin: bool,
    /// Require an ACK from this seat; repeat for more seats.
    #[arg(long = "require-ack", num_args = 1.., action = ArgAction::Append)]
    require_ack: Vec<String>,
    /// Require an ACK from one exact pane ID, label or live agent name; repeat
    /// this option for each recipient. Uses --space/--tab scope; qualify conflicts.
    /// Deduplicates with --require-ack seats without changing the sender.
    #[arg(long = "require-ack-pane", action = ArgAction::Append)]
    require_ack_pane: Vec<String>,
    /// Passive delivery at the next explicit inbox check (the default); no wake or ACK obligation.
    #[arg(long, conflicts_with = "nudge")]
    lazy: bool,
    /// Enable ordinary attention and wake behavior; explicit ACK recipients imply this mode.
    #[arg(long)]
    nudge: bool,
    /// ACK deadline in positive seconds; requires --nudge or explicit ACK recipients.
    #[arg(long)]
    deadline: Option<u64>,
    #[arg(
        long = "relays-user",
        help = "This message relays input from your user: a cooperative claim that marks it priority for thread summaries and catch-up"
    )]
    relays_user: bool,
    /// Recorded human input intent; ordinary humans may set it directly, agents require --relays-user.
    /// Missing intent stays unclassified; classification grants no permission.
    #[arg(long, value_parser = parse_user_intent, value_name = "query|request|rule")]
    user_intent: Option<UserIntent>,
}

fn parse_user_intent(value: &str) -> Result<UserIntent, String> {
    UserIntent::from_column(value)
        .ok_or_else(|| "user intent must be query, request or rule".to_owned())
}

#[derive(Args)]
struct PendingReceiptsArgs {
    #[command(flatten)]
    selector: super::panes::PaneSelector,
    #[arg(long)]
    seat: Option<String>,
    #[arg(long)]
    thread: Option<String>,
    #[command(flatten)]
    page: PageArgs,
}
#[derive(Args)]
struct DiagnosticsArgs {
    #[command(flatten)]
    selector: super::panes::PaneSelector,
    #[arg(long)]
    seat: Option<String>,
    #[arg(long)]
    thread: Option<String>,
    #[command(flatten)]
    page: PageArgs,
}
#[derive(Args)]
struct WarningsArgs {
    #[command(flatten)]
    selector: super::panes::PaneSelector,
    #[arg(long)]
    seat: Option<String>,
    /// Show open conditions and actionable legacy warnings in this thread.
    #[arg(long, conflicts_with = "seat")]
    active: Option<String>,
    #[command(flatten)]
    page: PageArgs,
}
#[derive(Args)]
struct ReadArgs {
    /// Thread to read; omitted opens the human terminal picker.
    thread: Option<String>,
    /// Keep running: print the recent messages (default 20, or --recent N)
    /// oldest first, then each new message as it is committed, IRC style for
    /// a person and one record per line otherwise. Never ACKs or accepts;
    /// Ctrl-C stops it.
    #[arg(long, conflicts_with_all = ["before", "cursor", "limit"])]
    follow: bool,
    /// With --follow: hide system notices (joins, ACKs, warnings).
    #[arg(long, requires = "follow")]
    no_system: bool,
    /// Number of latest messages to read; with --follow, defaults to 20.
    #[arg(long, group = "initial")]
    recent: Option<u16>,
    /// Read messages after this sequence number.
    #[arg(long, group = "initial")]
    after: Option<u64>,
    /// Read messages before this sequence number.
    #[arg(long, group = "initial")]
    before: Option<u64>,
    #[command(flatten)]
    page: PageArgs,
}
#[derive(Args)]
struct FollowArgs {
    /// Thread to follow; omitted opens the human terminal picker.
    thread: Option<String>,
    /// Number of latest messages printed first (default: 20; 0 skips history).
    #[arg(long, conflicts_with = "after")]
    recent: Option<u16>,
    /// Start after this sequence number instead of printing the recent tail.
    #[arg(long)]
    after: Option<u64>,
    /// Hide system notices (joins, ACKs, warnings).
    #[arg(long)]
    no_system: bool,
    /// Same byte-budget option as `read --follow`.
    #[arg(long)]
    max_bytes: Option<u32>,
}

#[derive(Args)]
struct BodyArgs {
    message: String,
    #[arg(long, conflicts_with = "cursor")]
    offset: Option<u64>,
    #[arg(long)]
    cursor: Option<String>,
    #[arg(long)]
    max_bytes: Option<u32>,
}
#[derive(Args)]
struct SearchArgs {
    literal: String,
    #[arg(long)]
    thread: Option<String>,
    #[command(flatten)]
    page: PageArgs,
}
#[derive(Args)]
struct SetupArgs {
    #[command(flatten)]
    permissions: PermissionCliInputs,
    /// Harness whose hooks to manage. Omitted: every harness (setup: each
    /// one found on PATH; unsetup and setup-status: both).
    #[arg(value_parser = ["claude", "codex"])]
    harness: Option<String>,
    /// Absolute harness executable (default: the first `claude`/`codex` on
    /// PATH). Version metadata is optional; setup uses the declared contract.
    #[arg(long, value_name = "PATH")]
    harness_binary: Option<String>,
    /// setup (claude): set Claude's `promptSuggestionEnabled` to false without
    /// asking (unsetup reverts it). Default: ask on a terminal, else advise.
    #[arg(long, conflicts_with = "keep_prompt_suggestions")]
    disable_prompt_suggestions: bool,
    /// setup (claude): leave Claude's prompt suggestions as they are, without
    /// asking or advising.
    #[arg(long)]
    keep_prompt_suggestions: bool,
    /// setup (claude): install or keep the hooks but not the delivery mod;
    /// removes the mod's `CLAUDE_CODE_PLUGIN_DIRS` path and files if setup
    /// wrote them (for example under managed policy that forbids the key).
    #[arg(long)]
    hooks_only: bool,
}

#[derive(Args)]
struct LaunchArgs {
    #[command(flatten)]
    selector: super::panes::PaneSelector,
    /// Native agent to start.
    #[arg(long, value_parser = ["claude", "codex"])]
    kind: String,
    /// Absolute harness executable whose `--version` is gated (default: the
    /// first `claude`/`codex` on PATH, which Herdr starts by name).
    #[arg(long, value_name = "PATH")]
    harness_binary: Option<String>,
    /// Herdr agent name (default: the pane's Herdr label, else the compact
    /// seat ID, or `seat-<short seat id>` for a persisted seat). Fitted to
    /// Herdr's `[a-z][a-z0-9_-]{0,31}`.
    #[arg(long, value_name = "NAME")]
    name: Option<String>,
    /// Native agent arguments, kept byte for byte and in order after `--`.
    #[arg(last = true, allow_hyphen_values = true, value_name = "AGENT_ARG")]
    agent_args: Vec<String>,
}

#[derive(Args)]
struct HandoffArgs {
    #[command(flatten)]
    selector: super::panes::PaneSelector,
    #[arg(long, required_unless_present = "thread", conflicts_with = "thread")]
    new_thread: bool,
    #[arg(long)]
    thread: Option<String>,
    #[arg(long, requires = "new_thread")]
    thread_name: Option<String>,
    #[arg(long, requires = "new_thread")]
    topic: Option<String>,
    #[arg(long, requires = "new_thread")]
    goal: Option<String>,
    #[arg(long, value_parser = ["claude", "codex"])]
    kind: String,
    #[arg(long)]
    harness_binary: Option<String>,
    /// Herdr agent name, distinct from --thread-name.
    #[arg(long)]
    name: Option<String>,
    /// Repeat for each native argument (use --agent-arg=-a for flags).
    #[arg(long = "agent-arg", allow_hyphen_values = true)]
    agent_args: Vec<String>,
    /// Exactly one quoted durable message after --; never native argv.
    #[arg(last = true, required = true, num_args = 1, allow_hyphen_values = true)]
    body: String,
}

#[derive(Args)]
struct ViewArgs {
    #[arg(long)]
    once: bool,
    #[command(flatten)]
    page: PageArgs,
}

fn setup_action(verb: super::setup::SetupVerb, args: SetupArgs) -> Result<CliAction, ApiError> {
    use super::setup::PromptSuggestionPolicy;
    use crate::harness::context::Harness;
    let prompt_suggestions = if args.disable_prompt_suggestions {
        PromptSuggestionPolicy::Disable
    } else if args.keep_prompt_suggestions {
        PromptSuggestionPolicy::Keep
    } else {
        PromptSuggestionPolicy::Ask
    };
    if prompt_suggestions != PromptSuggestionPolicy::Ask
        && (verb != super::setup::SetupVerb::Install || args.harness.as_deref() == Some("codex"))
    {
        return Err(invalid(
            "--disable-prompt-suggestions and --keep-prompt-suggestions apply to `setup` and \
             `setup claude` only (unsetup reverts what setup set)",
        ));
    }
    if args.hooks_only
        && (verb != super::setup::SetupVerb::Install || args.harness.as_deref() != Some("claude"))
    {
        return Err(invalid(
            "--hooks-only applies to `setup claude` only (unsetup removes the mod with the hooks)",
        ));
    }
    let Some(harness) = args.harness else {
        if args.harness_binary.is_some() {
            return Err(invalid(
                "--harness-binary needs a harness: `setup claude|codex --harness-binary PATH`",
            ));
        }
        return Ok(CliAction::SetupAll(verb, prompt_suggestions));
    };
    let harness = if harness == "codex" {
        Harness::Codex
    } else {
        Harness::Claude
    };
    if verb == super::setup::SetupVerb::Remove && args.harness_binary.is_some() {
        return Err(invalid(
            "--harness-binary is not used by unsetup: removal never depends on the harness version",
        ));
    }
    Ok(CliAction::Setup(super::setup::SetupRequest {
        verb,
        harness,
        harness_binary: args.harness_binary,
        prompt_suggestions,
        hooks_only: args.hooks_only,
    }))
}

/// The harness named by a `claude|codex` value-parsed argument.
fn harness_arg(name: &str) -> crate::harness::context::Harness {
    if name == "codex" {
        crate::harness::context::Harness::Codex
    } else {
        crate::harness::context::Harness::Claude
    }
}

fn invalid(detail: impl Into<String>) -> ApiError {
    ApiError::invalid_request(detail)
}
fn validation_error(detail: &'static str) -> ApiError {
    let code = if detail.contains("cursor") {
        ErrorCode::InvalidCursor
    } else if detail.contains("bound") || detail.contains("limit") || detail.contains("budget") {
        ErrorCode::InvalidBudget
    } else {
        ErrorCode::InvalidRequest
    };
    ApiError::new(code, detail)
}
fn id<T>(
    value: String,
    parse: impl FnOnce(String) -> Result<T, &'static str>,
) -> Result<T, ApiError> {
    parse(value).map_err(invalid)
}
/// The hook's attention digest lists items as `ITEM@THREAD`. That form is a
/// display reference, not a CLI argument: `(item, thread)` when `value` has it.
fn digest_reference(value: &str) -> Option<(&str, &str)> {
    let (item, thread) = value.split_once('@')?;
    (!item.is_empty() && !thread.is_empty() && !thread.contains('@')).then_some((item, thread))
}
/// Validate a public thread selector while keeping UTF-8 names outside the
/// opaque ID type. Runtime lookup precedes any digest-reference misuse hint.
fn thread_id(value: String) -> Result<ThreadId, ApiError> {
    validate_thread_name(&value).map_err(validation_error)?;
    // UTF-8/space selectors stay in ParsedCli, never in the opaque-ID type.
    Ok(ThreadId::parse(value).unwrap_or_else(|_| ThreadId::new("unresolved-thread-selector")))
}
/// A message argument; the digest form is refused naming the bare message ID.
fn message_id(value: String) -> Result<MessageId, ApiError> {
    if let Some((item, _)) = digest_reference(&value) {
        return Err(invalid(format!(
            "`{value}` is an attention digest item (ITEM@THREAD), not a message ID; pass the bare message ID `{item}`, for example `herdr-threads ack {item}`"
        )));
    }
    id(value, MessageId::parse)
}
fn bounded(value: String, name: &str) -> Result<String, ApiError> {
    if value.is_empty() || value.len() > 1024 {
        Err(invalid(format!("{name} must contain 1..1024 UTF-8 bytes")))
    } else {
        Ok(value)
    }
}
fn page(args: PageArgs) -> Result<PageRequest, ApiError> {
    let page = PageRequest {
        cursor: args.cursor,
        limit: args.limit.unwrap_or(DEFAULT_PAGE_LIMIT),
        max_bytes: args.max_bytes.unwrap_or(DEFAULT_PAGE_BYTES),
    };
    page.validate().map_err(validation_error)?;
    Ok(page)
}
fn deadline(seconds: Option<u64>) -> Result<Option<u64>, ApiError> {
    seconds
        .map(|seconds| {
            seconds
                .checked_mul(1000)
                .filter(|value| *value > 0)
                .ok_or_else(|| invalid("deadline must be positive and fit milliseconds"))
        })
        .transpose()
}

/// Exit statuses are part of the CLI contract; `cli::RunError::exit_code`
/// implements this table.
pub fn exit_status_help() -> String {
    format!(
        "Exit status:
  0  success (a reachable but degraded daemon still counts as success for `daemon ensure`)
  1  the request failed (not found, conflict, unauthorized, stale state, ...)
  2  invalid arguments or invalid local context
  3  daemon or host unavailable; {}
     (version mismatch: {})
  4  unsupported capability in this build or environment (including transport_denied:
     a sandbox refused the daemon socket; see `herdr-threads setup codex`)
  5  outcome unknown; inspect `herdr-threads pending-ops` and `retry` the local reference",
        crate::daemon::remedy::remedy(
            Some(crate::protocol::results::ErrorClass::Unavailable),
            &crate::daemon::remedy::RemedyContext::Exit3
        ),
        crate::daemon::remedy::remedy(
            Some(crate::protocol::results::ErrorClass::VersionSkew),
            &crate::daemon::remedy::RemedyContext::Exit3
        )
    )
}

/// Parser outcome that is not a command: help or version text requested by
/// the caller. It is printed to stdout and exits successfully.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ParseFailure {
    Informational(String),
    /// No subcommand was given: print usage on stderr and exit 2.
    Usage(String),
    Invalid(ApiError),
}

impl From<ApiError> for ParseFailure {
    fn from(value: ApiError) -> Self {
        Self::Invalid(value)
    }
}

/// Parse a real argv array; no shell evaluation or JSON argument is involved.
pub fn parse_argv<I, T>(argv: I) -> Result<ParsedCli, ApiError>
where
    I: IntoIterator<Item = T>,
    T: Into<std::ffi::OsString> + Clone,
{
    parse_argv_or_informational(argv).map_err(|failure| match failure {
        ParseFailure::Informational(text) | ParseFailure::Usage(text) => invalid(text),
        ParseFailure::Invalid(error) => error,
    })
}

/// Like [`parse_argv`], but distinguishes requested help/version output
/// from invalid arguments.
pub fn parse_argv_or_informational<I, T>(argv: I) -> Result<ParsedCli, ParseFailure>
where
    I: IntoIterator<Item = T>,
    T: Into<std::ffi::OsString> + Clone,
{
    use super::actor_route::{InvocationActor, command_index, guidance, split_actor_os_argv};
    let original: Vec<std::ffi::OsString> = argv.into_iter().map(Into::into).collect();
    let (actor, retained) = split_actor_os_argv(original.clone());
    let command = command_index(&retained);
    if actor == InvocationActor::Agent && command.is_some_and(|i| retained[i] == "human") {
        return Err(ParseFailure::Invalid(invalid(guidance(&original, command))));
    }
    let mut cli = Cli::try_parse_from(retained.clone()).map_err(|error| {
        use clap::error::ErrorKind;
        match error.kind() {
            ErrorKind::DisplayHelp | ErrorKind::DisplayVersion => {
                ParseFailure::Informational(error.render().to_string())
            }
            ErrorKind::DisplayHelpOnMissingArgumentOrSubcommand => {
                ParseFailure::Usage(error.render().to_string())
            }
            _ => {
                let legacy = command.is_some_and(|i| {
                    let first = retained[i].to_str();
                    let second = retained.get(i + 1).and_then(|s| s.to_str());
                    matches!((first, second), (Some("me"), Some("init")) | (Some("seat"), Some("rebind" | "retire")) | (Some("service"), Some("disconnect")))
                });
                let text = error.render().to_string();
                let text = if actor == InvocationActor::Agent && legacy {
                    format!("{text}\nPerson/operator actions require immediate human namespace; supplied arguments remain invalid. Supply the required flags listed above, including --operator for operator repair")
                } else if text.contains("--cooperative-harness") {
                    format!("{text}\nHuman is an invocation namespace, not an agent harness; use immediate human without cooperative agent selectors")
                } else { text };
                ParseFailure::Invalid(invalid(text))
            },
        }
    })?;
    // Clap propagates globals from the deepest subcommand and can drop earlier
    // Append values. Preserve recognized leading routing values for conflict checks.
    let mut index = 1;
    while index < command.unwrap_or(retained.len()) {
        let Some(token) = retained[index].to_str() else {
            break;
        };
        let (flag, inline) = token
            .split_once('=')
            .map_or((token, None), |(f, v)| (f, Some(v)));
        if matches!(flag, "--state-dir" | "--host-endpoint") {
            let value = inline.or_else(|| retained.get(index + 1).and_then(|v| v.to_str()));
            if let Some(value) = value {
                if flag == "--state-dir" {
                    cli.state_dir.push(value.to_owned());
                } else {
                    cli.host_endpoint.push(value.to_owned());
                }
            }
        }
        index += if inline.is_none()
            && matches!(
                flag,
                "--state-dir"
                    | "--host-endpoint"
                    | "--cooperative-seat"
                    | "--cooperative-target"
                    | "--cooperative-harness"
                    | "--cooperative-role"
            ) {
            2
        } else {
            1
        };
    }
    let mut parsed = parse_cli(cli).map_err(|error| {
        if actor == InvocationActor::Agent && error.detail == "operator required" {
            invalid(format!(
                "{}; use the human namespace and supply --operator",
                error.detail
            ))
        } else {
            error
        }
    })?;
    if actor == InvocationActor::Human && parsed.cooperative.is_some() {
        return Err(ParseFailure::Invalid(invalid(
            "human namespace cannot be mixed with cooperative agent selectors",
        )));
    }
    let requires_human = matches!(
        &parsed.action,
        CliAction::MeInit { .. }
            | CliAction::Mutation(
                MutationSpec::FreshSeat(_)
                    | MutationSpec::Rebind { .. }
                    | MutationSpec::Replace { .. }
                    | MutationSpec::Retire(_)
                    | MutationSpec::Invite { operator: true, .. }
                    | MutationSpec::CheckInLifecycle { operator: true, .. }
            )
            | CliAction::Wire(WireCommand::ServiceDisconnect(_))
    );
    if actor == InvocationActor::Agent && requires_human {
        return Err(ParseFailure::Invalid(invalid(guidance(&original, None))));
    }
    parsed.actor = actor;
    Ok(parsed)
}

/// P7 (native Claude demo 3): a global path flag may repeat only with one
/// identical value; conflicting values name both and are refused.
fn single_global(flag: &str, values: Vec<String>) -> Result<Option<String>, ApiError> {
    let mut values = values.into_iter();
    let Some(first) = values.next() else {
        return Ok(None);
    };
    match values.find(|value| *value != first) {
        None => Ok(Some(first)),
        Some(other) => Err(invalid(format!(
            "{flag} was given conflicting values `{first}` and `{other}`; repeat it only with the same value"
        ))),
    }
}

fn locator_hint(value: String) -> HostTargetId {
    HostTargetId::parse(value).unwrap_or_else(|_| HostTargetId::new("pending-pane-selector"))
}

fn parse_cli(mut cli: Cli) -> Result<ParsedCli, ApiError> {
    // Normalize before capturing selectors so names, picker eligibility and
    // follow execution share the existing read path.
    cli.command = match cli.command {
        Top::Follow(args) => Top::Read(ReadArgs {
            thread: args.thread,
            follow: true,
            no_system: args.no_system,
            recent: args.recent,
            after: args.after,
            before: None,
            page: PageArgs {
                cursor: None,
                limit: None,
                max_bytes: args.max_bytes,
            },
        }),
        command => command,
    };
    let permissions = match &cli.command {
        Top::Setup(args) => args.permissions.clone(),
        Top::Unsetup(args) | Top::SetupStatus(args) => {
            if args.permissions.with_permissions || args.permissions.without_permissions {
                return Err(invalid(
                    "permission consent flags are only valid for setup or installer-integrations",
                ));
            }
            args.permissions.clone()
        }
        Top::Internal {
            command: InternalSub::InstallerIntegrations { permissions, .. },
        } => permissions.clone(),
        _ => PermissionCliInputs::default(),
    };
    let cooperative_selector =
        cli.cooperative_target
            .as_ref()
            .map(|target| super::panes::PaneSelector {
                pane: Some(target.clone()),
                ..Default::default()
            });
    let cooperative = match (
        cli.cooperative_seat,
        cli.cooperative_target,
        cli.cooperative_harness,
        cli.cooperative_role,
    ) {
        (None, None, None, None) => None,
        (Some(seat), Some(target), Some(harness), Some(role)) => Some(CooperativeSelection {
            seat: id(seat, SeatId::parse)?,
            target: locator_hint(target),
            harness: if harness == "codex" {
                crate::harness::context::Harness::Codex
            } else {
                crate::harness::context::Harness::Claude
            },
            role: if role == "top-level" {
                crate::harness::context::Role::TopLevel
            } else {
                crate::harness::context::Role::Subagent
            },
        }),
        _ => {
            return Err(invalid(
                "cooperative seat, target, harness and role are required together",
            ));
        }
    };
    let output = OutputSpec {
        format: if cli.json {
            OutputFormat::Json
        } else {
            OutputFormat::Text
        },
        context: ContinuationContext {
            state_dir: single_global("--state-dir", cli.state_dir)?,
            host: single_global("--host-endpoint", cli.host_endpoint)?,
        },
    };
    output.validate().map_err(invalid)?;
    if let Top::Thread {
        command: ThreadSub::List(args),
    } = &cli.command
        && args.all
        && args.selector.is_explicit()
    {
        return Err(invalid(
            "--all conflicts with pane selectors; use the unfiltered instance directory or one pane's memberships",
        ));
    }
    let caller_read_default = match &cli.command {
        Top::Thread {
            command: ThreadSub::List(args),
        } => args.seat.is_none() && !args.selector.is_explicit() && !args.all,
        Top::Inbox(args) => args.seat.is_none() && !args.selector.is_explicit(),
        Top::PendingReceipts(args) => {
            args.seat.is_none() && !args.selector.is_explicit() && args.thread.is_none()
        }
        Top::Diagnostics(args) => {
            args.seat.is_none() && !args.selector.is_explicit() && args.thread.is_none()
        }
        Top::Warnings(args) => {
            args.seat.is_none() && !args.selector.is_explicit() && args.active.is_none()
        }
        _ => false,
    };
    let selected = match &cli.command {
        Top::Seat {
            command: SeatSub::Resolve {
                selector, new_seat, ..
            },
        } => {
            if *new_seat && selector.pane.is_none() {
                return Err(invalid("fresh-seat repair requires --pane PANE"));
            }
            Some(selector.clone())
        }
        Top::Seat {
            command: SeatSub::Rebind { selector, .. },
        }
        | Top::Launch(LaunchArgs { selector, .. })
        | Top::Handoff(HandoffArgs { selector, .. }) => {
            if selector.pane.is_none() {
                return Err(invalid("launch, handoff and rebind require --pane PANE"));
            }
            Some(selector.clone())
        }
        Top::Invite(args) => {
            if args.seat.is_some() && args.selector.is_explicit() {
                return Err(invalid("--seat conflicts with pane selectors"));
            }
            args.seat.is_none().then(|| args.selector.clone())
        }
        Top::Inbox(InboxArgs { selector, seat, .. })
        | Top::PendingReceipts(PendingReceiptsArgs { selector, seat, .. })
        | Top::Diagnostics(DiagnosticsArgs { selector, seat, .. })
        | Top::Warnings(WarningsArgs { selector, seat, .. })
        | Top::Thread {
            command: ThreadSub::List(ThreadListArgs { selector, seat, .. }),
        } => {
            if seat.is_some() && selector.is_explicit() {
                return Err(invalid("--seat conflicts with pane selectors"));
            }
            // An omitted selector reads as the caller's seat through the daemon's
            // canonical mapping, even when Herdr is unavailable (C4). Only an
            // explicit human locator needs a live topology read.
            selector.is_explicit().then(|| selector.clone())
        }
        Top::Send(args) => {
            let explicit_ack = !args.require_ack.is_empty() || !args.require_ack_pane.is_empty();
            if args.lazy && (explicit_ack || args.deadline.is_some()) {
                return Err(invalid(
                    "--lazy cannot be combined with explicit ACK recipients or a deadline",
                ));
            }
            if args.deadline.is_some() && !args.nudge && !explicit_ack {
                return Err(invalid(
                    "lazy sends cannot have a deadline; use --nudge or explicit ACK recipients",
                ));
            }
            if args.selector.pane.is_some() {
                return Err(invalid("send uses --require-ack-pane PANE"));
            }
            if args.selector.is_explicit() && args.require_ack_pane.is_empty() {
                return Err(invalid("send parents require --require-ack-pane"));
            }
            None
        }
        _ => None,
    };
    if let Top::Warnings(args) = &cli.command
        && args.active.is_some()
        && args.selector.is_explicit()
    {
        return Err(invalid("--active conflicts with pane selectors"));
    }
    let require_ack_panes = match &cli.command {
        Top::Send(args) => {
            if args.require_ack.len() + args.require_ack_pane.len() > MAX_BATCH_ITEMS {
                return Err(invalid("too many explicit recipients"));
            }
            args.require_ack_pane
                .iter()
                .map(|pane| super::panes::PaneSelector {
                    pane: Some(pane.clone()),
                    ..args.selector.clone()
                })
                .collect()
        }
        _ => Vec::new(),
    };
    let thread_selector = match &cli.command {
        Top::Thread {
            command:
                ThreadSub::Topic { thread, .. }
                | ThreadSub::Name { thread, .. }
                | ThreadSub::Rename { thread, .. }
                | ThreadSub::Show { thread, .. }
                | ThreadSub::Participants { thread, .. },
        } => Some(thread.clone()),
        Top::Participants { thread, .. }
        | Top::Join { thread }
        | Top::Accept { thread }
        | Top::Reject { thread, .. }
        | Top::AcceptRequired { thread, .. }
        | Top::Leave { thread }
        | Top::Archive { thread }
        | Top::Reopen { thread } => Some(thread.clone()),
        Top::Handoff(args) => args.thread.clone(),
        Top::Invite(args) => Some(args.thread.clone()),
        Top::Send(args) => Some(args.thread.clone()),
        Top::Read(args) => args.thread.clone(),
        Top::PendingReceipts(args) => args.thread.clone(),
        Top::Diagnostics(args) => args.thread.clone(),
        Top::Warnings(args) => args.active.clone(),
        Top::Search(args) => args.thread.clone(),
        Top::Summary(args) => args.thread.clone(),
        _ => None,
    };
    let action = match cli.command {
        Top::Follow(_) => unreachable!("follow was normalized to read"),
        Top::Thread { command } => match command {
            ThreadSub::Create { name, topic, goal } => {
                let goal = goal.unwrap_or_else(|| topic.clone());
                CliAction::Mutation(MutationSpec::Create {
                    name: name
                        .map(|name| {
                            validate_thread_name(&name).map_err(validation_error)?;
                            Ok::<_, ApiError>(name)
                        })
                        .transpose()?,
                    topic: bounded(topic, "topic")?,
                    goal: bounded(goal, "goal")?,
                })
            }
            ThreadSub::Topic {
                thread,
                set: Some(topic),
            } => CliAction::Mutation(MutationSpec::Topic {
                thread: thread_id(thread)?,
                topic: bounded(topic, "topic")?,
            }),
            ThreadSub::Topic { thread, set: None } => {
                CliAction::Wire(WireCommand::Thread(ThreadQuery {
                    thread: thread_id(thread)?,
                    page: PageRequest::default(),
                    caller: None,
                }))
            }
            ThreadSub::Name { thread, set, clear } => {
                let thread = thread_id(thread)?;
                if clear || set.is_some() {
                    if let Some(name) = &set {
                        validate_thread_name(name).map_err(validation_error)?;
                    }
                    CliAction::Mutation(MutationSpec::Name { thread, name: set })
                } else {
                    CliAction::Wire(WireCommand::ThreadName(ThreadNameQuery { thread }))
                }
            }
            ThreadSub::Rename { thread, name } => {
                validate_thread_name(&name).map_err(validation_error)?;
                CliAction::Mutation(MutationSpec::Name {
                    thread: thread_id(thread)?,
                    name: Some(name),
                })
            }
            ThreadSub::List(args) => CliAction::Wire(WireCommand::Directory(DirectoryQuery {
                recent: args.recent,
                membership: args.seat.map(|s| id(s, SeatId::parse)).transpose()?,
                membership_filter: if args.joined {
                    DirectoryMembership::Joined
                } else if args.invited {
                    DirectoryMembership::Invited
                } else if args.all {
                    DirectoryMembership::All
                } else {
                    DirectoryMembership::Default
                },
                topic_contains: args.search.map(|s| bounded(s, "search")).transpose()?,
                page: page(args.page)?,
            })),
            ThreadSub::Show { thread, page: args } => {
                CliAction::Wire(WireCommand::Thread(ThreadQuery {
                    thread: thread_id(thread)?,
                    page: page(args)?,
                    caller: None,
                }))
            }
            ThreadSub::Participants { thread, page: args } => {
                CliAction::Wire(WireCommand::Participants(ParticipantsQuery {
                    thread: thread_id(thread)?,
                    page: page(args)?,
                    caller: None,
                }))
            }
        },
        Top::Invite(args) => CliAction::Mutation(MutationSpec::Invite {
            thread: thread_id(args.thread)?,
            seat: id(
                args.seat.unwrap_or_else(|| "pending-pane-selector".into()),
                SeatId::parse,
            )?,
            deadline_millis: deadline(args.deadline)?,
            operator: args.operator,
        }),
        Top::Participants { thread, page: args } => {
            CliAction::Wire(WireCommand::Participants(ParticipantsQuery {
                thread: thread_id(thread)?,
                page: page(args)?,
                caller: None,
            }))
        }
        Top::Join { thread } => CliAction::Mutation(MutationSpec::Join(thread_id(thread)?)),
        Top::Accept { thread } => CliAction::Mutation(MutationSpec::Accept(thread_id(thread)?)),
        Top::Reject {
            thread,
            invitation,
            reason,
        } => {
            validate_rejection_reason(&reason).map_err(invalid)?;
            CliAction::Mutation(MutationSpec::Reject {
                thread: thread_id(thread)?,
                invitation: id(invitation, InvitationId::parse)?,
                reason,
            })
        }
        Top::AcceptRequired {
            thread,
            invitation,
            requirement,
            revision,
        } => {
            if revision == 0 {
                return Err(invalid("required acceptance revision must be positive"));
            }
            CliAction::Mutation(MutationSpec::AcceptRequired {
                thread: thread_id(thread)?,
                invitation: id(invitation, InvitationId::parse)?,
                requirement: id(requirement, RequirementId::parse)?,
                expected_revision: revision,
            })
        }
        Top::Leave { thread } => CliAction::Mutation(MutationSpec::Leave(thread_id(thread)?)),
        Top::Send(args) => {
            let delivery_mode = if args.nudge
                || !args.require_ack.is_empty()
                || !args.require_ack_pane.is_empty()
            {
                DeliveryMode::Ordinary
            } else {
                DeliveryMode::Lazy
            };
            if args.require_ack.len() > MAX_BATCH_ITEMS {
                return Err(invalid("too many explicit recipients"));
            }
            let recipients = args
                .require_ack
                .into_iter()
                .map(|s| id(s, SeatId::parse))
                .collect::<Result<Vec<_>, _>>()?;
            if recipients
                .iter()
                .collect::<std::collections::HashSet<_>>()
                .len()
                != recipients.len()
            {
                return Err(invalid("duplicate explicit recipient"));
            }
            let thread = thread_id(args.thread)?;
            let deadline_millis = deadline(args.deadline)?;
            let relays_user = args.relays_user;
            let body = crate::cli::input::read_body(args.body, args.file, args.stdin)?;
            CliAction::Mutation(MutationSpec::Send {
                delivery_mode,
                thread,
                body,
                require_ack: recipients,
                deadline_millis,
                relays_user,
                user_intent: args.user_intent,
            })
        }
        Top::Ack { messages } => {
            if messages.len() > MAX_BATCH_ITEMS {
                return Err(invalid("too many ACK IDs"));
            }
            let ids = messages
                .into_iter()
                .map(message_id)
                .collect::<Result<Vec<_>, _>>()?;
            if ids.iter().collect::<std::collections::HashSet<_>>().len() != ids.len() {
                return Err(invalid("duplicate ACK ID"));
            }
            CliAction::Mutation(MutationSpec::Ack(ids))
        }
        Top::Archive { thread } => CliAction::Mutation(MutationSpec::Archive(thread_id(thread)?)),
        Top::Reopen { thread } => CliAction::Mutation(MutationSpec::Reopen(thread_id(thread)?)),
        Top::Inbox(args) => CliAction::Wire(WireCommand::Inbox(InboxQuery {
            seat: args.seat.map(|s| id(s, SeatId::parse)).transpose()?,
            page: page(args.page)?,
        })),
        Top::Warnings(args) => match (
            args.seat
                .or_else(|| selected.as_ref().map(|_| "pending-pane-selector".into())),
            args.active,
        ) {
            (Some(seat), None) => CliAction::Wire(WireCommand::Warnings(WarningsQuery {
                seat: id(seat, SeatId::parse)?,
                page: page(args.page)?,
            })),
            (None, Some(thread)) => CliAction::Wire(WireCommand::ActiveWarnings(
                crate::protocol::commands::ActiveWarningsQuery {
                    thread: thread_id(thread)?,
                    page: page(args.page)?,
                },
            )),
            (None, None) => CliAction::Wire(WireCommand::Warnings(WarningsQuery {
                seat: SeatId::new("pending-pane-selector"),
                page: page(args.page)?,
            })),
            _ => return Err(invalid("warnings cannot combine seat and active thread")),
        },
        Top::CheckIn(args) => CliAction::Mutation(match args.lifecycle_event {
            Some(event_id) => MutationSpec::CheckInLifecycle {
                event_id: bounded(event_id, "lifecycle event identity")?,
                native_session: args
                    .native_session
                    .map(|s| bounded(s, "native session reference"))
                    .transpose()?,
                operator: false,
            },
            None => MutationSpec::CheckIn,
        }),
        Top::PendingReceipts(args) => {
            CliAction::Wire(WireCommand::PendingReceipts(PendingReceiptsQuery {
                seat: args.seat.map(|s| id(s, SeatId::parse)).transpose()?,
                thread: args.thread.map(thread_id).transpose()?,
                page: page(args.page)?,
            }))
        }
        Top::Read(args) if args.thread.is_none() => {
            if args.page.cursor.is_some() {
                return Err(invalid(
                    "bare read cannot use a history cursor; select a thread explicitly",
                ));
            }
            if args.recent.is_some() && args.page.limit.is_some() {
                return Err(invalid("recent conflicts with limit"));
            }
            let initial = args
                .after
                .map(|sequence| HistoryRange::After { sequence })
                .or_else(|| {
                    args.before
                        .map(|sequence| HistoryRange::Before { sequence })
                })
                .or_else(|| {
                    Some(HistoryRange::Recent {
                        count: args
                            .recent
                            .unwrap_or(args.page.limit.unwrap_or(FOLLOW_DEFAULT_RECENT)),
                    })
                });
            let mut page = page(args.page)?;
            if let Some(HistoryRange::Recent { count }) = initial {
                if count > crate::protocol::pagination::MAX_PAGE_LIMIT
                    || (!args.follow && count == 0)
                {
                    return Err(validation_error("invalid recent history count"));
                }
                page.limit = count.max(1);
            }
            CliAction::Picker(PickerRequest {
                page,
                initial,
                follow: args.follow,
                no_system: args.no_system,
            })
        }
        Top::Read(args) if args.follow => {
            let recent = args.recent.unwrap_or(if args.after.is_some() {
                0
            } else {
                FOLLOW_DEFAULT_RECENT
            });
            if recent > crate::protocol::pagination::MAX_PAGE_LIMIT {
                return Err(validation_error("invalid recent history count"));
            }
            CliAction::Follow(FollowRequest {
                thread: thread_id(args.thread.expect("explicit read branch"))?,
                recent,
                after: args.after,
                no_system: args.no_system,
            })
        }
        Top::Read(args) => {
            if args.page.cursor.is_some()
                && (args.recent.is_some() || args.after.is_some() || args.before.is_some())
            {
                return Err(invalid("cursor conflicts with initial history range"));
            }
            if args.recent.is_some() && args.page.limit.is_some() {
                return Err(invalid("recent conflicts with limit"));
            }
            let initial = args
                .recent
                .map(|count| HistoryRange::Recent { count })
                .or_else(|| args.after.map(|sequence| HistoryRange::After { sequence }))
                .or_else(|| {
                    args.before
                        .map(|sequence| HistoryRange::Before { sequence })
                });
            let mut page = page(args.page)?;
            if let Some(HistoryRange::Recent { count }) = initial {
                page.limit = count;
            }
            CliAction::Wire(WireCommand::History(HistoryQuery {
                thread: thread_id(args.thread.expect("explicit read branch"))?,
                page,
                initial,
                full_bodies: false,
            }))
        }
        Top::Body(args) => CliAction::Wire(WireCommand::Message(MessageQuery {
            message: message_id(args.message)?,
            body: BodyReadRequest {
                cursor: args.cursor,
                offset: args.offset,
                max_bytes: args.max_bytes.unwrap_or(DEFAULT_PAGE_BYTES),
            },
        })),
        Top::Search(args) => CliAction::Wire(WireCommand::Search(SearchQuery {
            literal: bounded(args.literal, "search")?,
            thread: args.thread.map(thread_id).transpose()?,
            page: page(args.page)?,
            max_candidates: MAX_SEARCH_CANDIDATES,
        })),
        Top::Seat { command } => match command {
            SeatSub::List {
                include_retired,
                page: args,
            } => CliAction::Wire(WireCommand::Seats(SeatsQuery {
                page: page(args)?,
                target: None,
                include_retired,
            })),
            SeatSub::Resolve {
                selector,
                new_seat: true,
                ..
            } => CliAction::Mutation(MutationSpec::FreshSeat(locator_hint(
                selector.pane.expect("explicit pane validated"),
            ))),
            SeatSub::Resolve { selector, .. } => {
                CliAction::Mutation(MutationSpec::Resolve(locator_hint(
                    selector
                        .pane
                        .unwrap_or_else(|| "pending-caller-selector".into()),
                )))
            }
            SeatSub::Inspect { seat, page: args } => {
                CliAction::Wire(WireCommand::SeatInspect(SeatInspectQuery {
                    seat: id(seat, SeatId::parse)?,
                    page: page(args)?,
                }))
            }
            SeatSub::Retirements(args) => {
                CliAction::Wire(WireCommand::RetirementJobs(RetirementJobsQuery {
                    page: page(args)?,
                }))
            }
            SeatSub::Rebind {
                seat,
                selector,
                replace,
                operator,
            } => {
                if !operator {
                    return Err(invalid("operator required"));
                }
                let seat = id(seat, SeatId::parse)?;
                let pane = locator_hint(selector.pane.expect("explicit pane validated"));
                CliAction::Mutation(match replace {
                    Some(replace) => MutationSpec::Replace {
                        seat,
                        pane,
                        replace: id(replace, SeatId::parse)?,
                    },
                    None => MutationSpec::Rebind { seat, pane },
                })
            }
            SeatSub::Retire { seat, operator } => {
                if !operator {
                    return Err(invalid("operator required"));
                }
                CliAction::Mutation(MutationSpec::Retire(id(seat, SeatId::parse)?))
            }
        },
        Top::Delivery {
            command:
                DeliverySub::Recipients {
                    message,
                    page: args,
                },
        } => CliAction::Wire(WireCommand::Recipients(RecipientsQuery {
            message: message_id(message)?,
            page: page(args)?,
        })),
        Top::Delivery {
            command:
                DeliverySub::Inspect {
                    message,
                    page: args,
                },
        } => CliAction::Wire(WireCommand::DeliveryInspect(DeliveryInspectQuery {
            message: message_id(message)?,
            page: page(args)?,
        })),
        Top::Overdue(args) => CliAction::Wire(WireCommand::Diagnostics(DiagnosticsQuery {
            seat: None,
            thread: None,
            page: page(args)?,
        })),
        Top::Diagnostics(args) => CliAction::Wire(WireCommand::Diagnostics(DiagnosticsQuery {
            seat: args
                .seat
                .map(|value| id(value, SeatId::parse))
                .transpose()?,
            thread: args.thread.map(thread_id).transpose()?,
            page: page(args.page)?,
        })),
        Top::Daemon { command } => CliAction::Daemon(match command {
            DaemonSub::Health => DaemonAction::Health,
            DaemonSub::Ensure => DaemonAction::Ensure,
            DaemonSub::Stop => DaemonAction::Stop,
        }),
        Top::Service { command } => CliAction::Wire(match command {
            ServiceSub::Inspect => WireCommand::ServiceInspect,
            ServiceSub::Disconnect {
                expected_boot,
                expected_generation,
            } => WireCommand::ServiceDisconnect(ServiceDisconnectRequest {
                expected_boot: bounded(expected_boot, "expected boot")?,
                expected_generation,
            }),
        }),
        Top::Doctor { debug, command } => CliAction::Doctor {
            debug,
            fix: matches!(command, Some(DoctorSub::Fix)),
        },
        Top::Setup(args) => setup_action(super::setup::SetupVerb::Install, args)?,
        Top::Unsetup(args) => setup_action(super::setup::SetupVerb::Remove, args)?,
        Top::SetupStatus(args) => setup_action(super::setup::SetupVerb::Status, args)?,
        Top::Me {
            command: MeSub::Init { operator },
        } => CliAction::MeInit { operator },
        Top::Launch(args) => {
            use crate::harness::context::Harness;
            let harness = if args.kind == "codex" {
                Harness::Codex
            } else {
                Harness::Claude
            };
            if let Some(name) = &args.name
                && crate::ports::sanitize_agent_name(name).is_none()
            {
                return Err(invalid(format!(
                    "--name `{name}` has no letter to start a Herdr agent name \
                     ([a-z][a-z0-9_-]{{0,31}})"
                )));
            }
            CliAction::Launch(super::launch::LaunchRequest {
                target: locator_hint(args.selector.pane.expect("explicit pane validated")),
                harness,
                harness_binary: args.harness_binary,
                argv: args.agent_args,
                name: args.name,
                pane_label: None,
            })
        }
        Top::Handoff(args) => {
            if !args.new_thread
                && (args.thread_name.is_some() || args.topic.is_some() || args.goal.is_some())
            {
                return Err(invalid(
                    "--thread-name, --topic and --goal require --new-thread",
                ));
            }
            if args
                .name
                .as_ref()
                .is_some_and(|name| crate::ports::sanitize_agent_name(name).is_none())
            {
                return Err(invalid("invalid agent --name"));
            }
            if let Some(name) = &args.thread_name {
                validate_thread_name(name).map_err(validation_error)?;
            }
            let body = bounded(args.body, "body")?;
            CliAction::Handoff(super::handoff::HandoffRequest {
                thread: args.thread.map(thread_id).transpose()?,
                thread_name: args.thread_name,
                topic: args.topic.map(|s| bounded(s, "topic")).transpose()?,
                goal: args.goal.map(|s| bounded(s, "goal")).transpose()?,
                body,
                launch: super::launch::LaunchRequest {
                    target: locator_hint(args.selector.pane.expect("explicit pane validated")),
                    harness: if args.kind == "codex" {
                        crate::harness::context::Harness::Codex
                    } else {
                        crate::harness::context::Harness::Claude
                    },
                    harness_binary: args.harness_binary,
                    argv: args.agent_args,
                    name: args.name,
                    pane_label: None,
                },
            })
        }
        Top::PendingOps(args) => CliAction::PendingOps(page(args)?),
        Top::Skill => CliAction::Skill,
        Top::Summary(args) => {
            use super::summary::SummaryCli;
            let job = |job: String, lease: String| -> Result<_, ApiError> {
                Ok((id(job, SummaryJobId::parse)?, id(lease, LeaseToken::parse)?))
            };
            CliAction::Summary(match (args.thread, args.command) {
                (Some(thread), None) => SummaryCli::Summary {
                    thread: thread_id(thread)?,
                },
                (None, Some(SummarySub::Job { job: j, lease })) => {
                    let (job, lease) = job(j, lease)?;
                    SummaryCli::Job { job, lease }
                }
                (None, Some(SummarySub::Submit { job: j, lease })) => {
                    let (job, lease) = job(j, lease)?;
                    SummaryCli::Submit { job, lease }
                }
                _ => {
                    return Err(invalid(
                        "summary needs a THREAD, or the `job` or `submit` subcommand \
                         (see `herdr-threads summary --help`)",
                    ));
                }
            })
        }
        Top::Watch(args) => match (args.command, args.harness, args.session) {
            (Some(WatchSub::Ack { session, via, ids }), _, _) => {
                CliAction::WatchAck(super::watch::WatchAckRequest {
                    session,
                    via: match via.as_str() {
                        "context" => crate::protocol::watch::ModDeliveryVia::Context,
                        "submit" => crate::protocol::watch::ModDeliveryVia::Submit,
                        _ => crate::protocol::watch::ModDeliveryVia::Append,
                    },
                    messages: ids
                        .into_iter()
                        .map(|value| id(value, MessageId::parse))
                        .collect::<Result<_, _>>()?,
                })
            }
            (None, Some(_), Some(session)) => CliAction::Watch(super::watch::WatchRequest {
                harness: crate::harness::context::Harness::Claude,
                session,
            }),
            _ => {
                return Err(invalid(
                    "watch needs --harness claude and --session ID, or the `ack` subcommand",
                ));
            }
        },
        Top::ContractId { harness } => CliAction::ContractId {
            harness: harness.as_deref().map(harness_arg),
        },
        Top::HarnessVersion {
            command: HarnessVersionSub::Normalize { harness, raw },
        } => CliAction::HarnessVersionNormalize {
            harness: harness_arg(&harness),
            raw,
        },
        Top::Internal {
            command:
                InternalSub::InstallerIntegrations {
                    confirm_missing, ..
                },
        } => CliAction::InstallerIntegrations { confirm_missing },
        Top::Internal {
            command: InternalSub::JsonField { path },
        } => CliAction::InternalJsonField { path },
        Top::Retry { recovery_ref } => CliAction::Retry(id(recovery_ref, LocalRecoveryRef::parse)?),
        Top::View(args) => CliAction::View {
            once: args.once,
            page: page(args.page)?,
        },
        Top::CachedCheckIn(args) => {
            let max_bytes = args.max_bytes.unwrap_or(65_536);
            if !(256..=65_536).contains(&max_bytes) {
                return Err(validation_error("cache byte budget out of bounds"));
            }
            CliAction::CachedCheckIn(CachePageRequest {
                reference: CacheRefV1::parse(&args.reference)
                    .map_err(|_| invalid("invalid cache reference"))?,
                cursor: args
                    .cursor
                    .map(|value| {
                        CacheCursorV1::parse(&value)
                            .map_err(|_| validation_error("invalid cache cursor"))
                    })
                    .transpose()?,
                max_bytes,
            })
        }
    };
    if let CliAction::Wire(command) = &action {
        // Inbox is a client-side placeholder until capability discovery. A v2
        // continuation must retain that action for own-text settlement, while
        // legacy wire commands continue to reject the distinct cursor namespace.
        if let WireCommand::Inbox(query) = command
            && query
                .page
                .cursor
                .as_deref()
                .is_some_and(|raw| raw.starts_with(crate::protocol::pagination::INBOX_V2_PREFIX))
        {
            WireCommand::InboxBatchV2(query.clone())
                .validate()
                .map_err(validation_error)?;
        } else {
            command.validate().map_err(validation_error)?;
        }
    }
    Ok(ParsedCli {
        actor: super::actor_route::InvocationActor::Agent,
        permissions,
        output,
        presentation: if cli.human {
            crate::cli::output::Presentation::Human
        } else if cli.machine {
            crate::cli::output::Presentation::Machine
        } else {
            crate::cli::output::Presentation::Auto
        },
        action,
        cooperative,
        pane_selector: selected,
        caller_read_default,
        cooperative_selector,
        require_ack_panes,
        thread_selector,
    })
}

#[cfg(test)]
mod tests {
    include!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/cli/commands.rs"
    ));
}
