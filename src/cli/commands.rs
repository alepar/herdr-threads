//! Public syntax, validation, and typed dispatch preparation.

use crate::harness::cache::{CacheCursorV1, CachePageRequest, CacheRefV1};
use crate::protocol::{
    authority::CallerClaim,
    commands::{Command as WireCommand, *},
    ids::*,
    output::{ContinuationContext, OutputFormat, OutputSpec},
    pagination::{DEFAULT_PAGE_BYTES, DEFAULT_PAGE_LIMIT, PageRequest},
    results::{ApiError, CommandResult, ErrorCode},
};
use clap::{ArgAction, Args, Parser, Subcommand};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedCli {
    pub output: OutputSpec,
    /// Terminal presentation of text output; never sent to the daemon.
    pub presentation: crate::cli::output::Presentation,
    pub action: CliAction,
    pub cooperative: Option<CooperativeSelection>,
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
    /// Read-only diagnostics.
    Doctor,
    CachedCheckIn(CachePageRequest),
    /// Local harness hook setup, removal or inspection; never contacts the daemon.
    Setup(super::setup::SetupRequest),
    /// `setup`, `unsetup` or `setup-status` with no harness named: every
    /// harness (setup: every detected one); local, never contacts the daemon.
    SetupAll(super::setup::SetupVerb),
    /// Managed native launch into one explicit existing empty shell pane.
    Launch(super::launch::LaunchRequest),
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
    /// `harness-version normalize HARNESS RAW`: the canonical bare semver of a
    /// raw `--version` string; local only.
    HarnessVersionNormalize {
        harness: crate::harness::context::Harness,
        raw: String,
    },
    /// Hidden `internal json-field PATH`: print a field of the JSON on stdin.
    InternalJsonField {
        path: String,
    },
    /// `read THREAD --follow`: recent messages, then each new one as it is
    /// committed. Read-only: never ACKs or accepts.
    Follow(FollowRequest),
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
        CliAction::Doctor => return backend.local(LocalAction::Doctor, &parsed.output),
        CliAction::CachedCheckIn(request) => {
            return backend.local(LocalAction::CachedCheckIn(request), &parsed.output);
        }
        CliAction::Setup(_)
        | CliAction::SetupAll(_)
        | CliAction::Launch(_)
        | CliAction::MeInit { .. }
        | CliAction::Skill
        | CliAction::ContractId { .. }
        | CliAction::HarnessVersionNormalize { .. }
        | CliAction::InternalJsonField { .. }
        | CliAction::Follow(_) => {
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
        topic: String,
        goal: String,
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
    Accept(ThreadId),
    AcceptRequired {
        thread: ThreadId,
        invitation: InvitationId,
        requirement: RequirementId,
        expected_revision: u64,
    },
    Leave(ThreadId),
    Send {
        thread: ThreadId,
        body: String,
        require_ack: Vec<SeatId>,
        deadline_millis: Option<u64>,
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
            Self::Create { topic, goal } => WireCommand::CreateThread(CreateThread {
                topic,
                goal,
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
                thread,
                body,
                require_ack,
                deadline_millis,
            } => WireCommand::SendMessage(SendMessage {
                thread,
                body,
                invited_recipients: require_ack,
                deadline_millis,
                operation,
                claim: claim.unwrap(),
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
    after_help = format!("{}\n\n{}", exit_status_help(), super::skill::AI_HELP_FOOTER),
    about = "Read threads and explicitly ACK exact message IDs. Accept invitations separately. Optional cheap subagents can summarize recent or full history without ACK authority."
)]
struct Cli {
    /// May repeat when every value is identical (a wrapper that pins it plus
    /// a ready command that names it); conflicting values are refused.
    #[arg(long, global = true, action = ArgAction::Append)]
    state_dir: Vec<String>,
    /// May repeat when every value is identical; conflicting values are refused.
    #[arg(long, global = true, action = ArgAction::Append)]
    host_endpoint: Vec<String>,
    #[arg(long, global = true, action = ArgAction::SetTrue)]
    json: bool,
    /// Human-readable tables and transcripts (the default on a terminal).
    #[arg(long, global = true, action = ArgAction::SetTrue, conflicts_with_all = ["json", "machine"])]
    human: bool,
    /// The stable `key: value` text form for scripts and models (the default
    /// when stdout is not a terminal).
    #[arg(long, global = true, action = ArgAction::SetTrue, conflicts_with = "json")]
    machine: bool,
    #[arg(long, global = true)]
    cooperative_seat: Option<String>,
    #[arg(long, global = true)]
    cooperative_target: Option<String>,
    #[arg(long, global = true, value_parser = ["codex", "claude"])]
    cooperative_harness: Option<String>,
    #[arg(long, global = true, value_parser = ["top-level", "subagent"])]
    cooperative_role: Option<String>,
    #[command(subcommand)]
    command: Top,
}

#[derive(Subcommand)]
enum Top {
    Thread {
        #[command(subcommand)]
        command: ThreadSub,
    },
    Invite(InviteArgs),
    /// Same as `thread participants THREAD` (the caller's own seat is marked
    /// `"self": true`).
    Participants {
        thread: String,
        #[command(flatten)]
        page: PageArgs,
    },
    Accept {
        thread: String,
    },
    AcceptRequired {
        thread: String,
        #[arg(long)]
        invitation: String,
        #[arg(long)]
        requirement: String,
        #[arg(long)]
        revision: u64,
    },
    Leave {
        thread: String,
    },
    Send(SendArgs),
    Ack {
        #[arg(required = true, num_args = 1..)]
        messages: Vec<String>,
    },
    Archive {
        thread: String,
    },
    Reopen {
        thread: String,
    },
    Inbox(InboxArgs),
    Warnings(WarningsArgs),
    CheckIn(CheckInArgs),
    CachedCheckIn(CachedCheckInArgs),
    PendingReceipts(PendingReceiptsArgs),
    Read(ReadArgs),
    Body(BodyArgs),
    Search(SearchArgs),
    Seat {
        #[command(subcommand)]
        command: SeatSub,
    },
    Delivery {
        #[command(subcommand)]
        command: DeliverySub,
    },
    Overdue(PageArgs),
    Diagnostics(DiagnosticsArgs),
    Daemon {
        #[command(subcommand)]
        command: DaemonSub,
    },
    Service {
        #[command(subcommand)]
        command: ServiceSub,
    },
    /// Read-only local diagnostics: context, daemon health and the owned
    /// user-level Claude and Codex hook installations.
    Doctor,
    /// Install the owned herdr-threads hooks for a harness at user level
    /// (with no harness: for every detected harness).
    /// Claude: `$CLAUDE_CONFIG_DIR/settings.json` (hooks and allow rule).
    /// Codex: `$CODEX_HOME/hooks.json` plus the sandbox socket allowance in
    /// `$CODEX_HOME/config.toml`. Ownership manifests stay private in the
    /// plugin state. Refuses a harness version that is unparsable,
    /// known broken or older than every recipe; a newer unlisted version is
    /// admitted optimistically.
    #[command(after_help = super::setup::SETUP_HELP)]
    Setup(SetupArgs),
    /// Remove the owned herdr-threads hooks installed by `setup`, keeping
    /// every other setting; unchanged settings are restored byte for byte.
    #[command(after_help = super::setup::SETUP_HELP)]
    Unsetup(SetupArgs),
    /// Report whether the owned hooks are installed (separately from native
    /// observation) and whether the installed harness version is supported.
    #[command(after_help = super::setup::SETUP_HELP)]
    SetupStatus(SetupArgs),
    PendingOps(PageArgs),
    Retry {
        recovery_ref: String,
    },
    View(ViewArgs),
    /// Start Codex or Claude in one explicit existing empty shell pane with
    /// the owned hooks, after resolving the pane's seat. Launch never
    /// registers, accepts or ACKs: the handoff is read through the hooks.
    #[command(after_help = super::launch::LAUNCH_HELP)]
    Launch(LaunchArgs),
    /// Your own identity as a person in this Herdr pane.
    Me {
        #[command(subcommand)]
        command: MeSub,
    },
    /// Print the agent skill (SKILL.md): how agents should use herdr-threads.
    /// Also available as `--skill`. Local only; never contacts the daemon.
    #[command(long_flag = "skill")]
    Skill,
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
    Create {
        #[arg(long)]
        topic: String,
        #[arg(long)]
        goal: Option<String>,
    },
    Topic {
        thread: String,
        #[arg(long = "set")]
        set: Option<String>,
    },
    List(ThreadListArgs),
    Show {
        thread: String,
        #[command(flatten)]
        page: PageArgs,
    },
    Participants {
        thread: String,
        #[command(flatten)]
        page: PageArgs,
    },
}
#[derive(Subcommand)]
enum SeatSub {
    List(PageArgs),
    Resolve {
        #[arg(long)]
        pane: String,
        #[arg(long, requires = "operator")]
        new_seat: bool,
        #[arg(long, requires = "new_seat")]
        operator: bool,
    },
    Inspect {
        seat: String,
        #[command(flatten)]
        page: PageArgs,
    },
    Rebind {
        seat: String,
        #[arg(long)]
        pane: String,
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
    Retirements(PageArgs),
}
#[derive(Subcommand)]
enum DeliverySub {
    Recipients {
        message: String,
        #[command(flatten)]
        page: PageArgs,
    },
    Inspect {
        message: String,
        #[command(flatten)]
        page: PageArgs,
    },
}
#[derive(Subcommand)]
enum DaemonSub {
    Health,
    Ensure,
    Stop,
}
#[derive(Subcommand)]
enum ServiceSub {
    Inspect,
    Disconnect {
        #[arg(long)]
        expected_boot: String,
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
    #[arg(long)]
    cursor: Option<String>,
    #[arg(long)]
    limit: Option<u16>,
    #[arg(long)]
    max_bytes: Option<u32>,
}
#[derive(Args)]
struct ThreadListArgs {
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
    #[arg(long)]
    seat: Option<String>,
    #[command(flatten)]
    page: PageArgs,
}
#[derive(Args)]
struct InviteArgs {
    thread: String,
    #[arg(long)]
    seat: String,
    #[arg(long)]
    deadline: Option<u64>,
    #[arg(long)]
    operator: bool,
}
#[derive(Args)]
struct SendArgs {
    thread: String,
    #[arg(long, group = "body_source")]
    body: Option<String>,
    #[arg(long, group = "body_source")]
    file: Option<String>,
    #[arg(long, group = "body_source")]
    stdin: bool,
    #[arg(long = "require-ack", num_args = 1.., action = ArgAction::Append)]
    require_ack: Vec<String>,
    #[arg(long)]
    deadline: Option<u64>,
}
#[derive(Args)]
struct PendingReceiptsArgs {
    #[arg(long)]
    seat: Option<String>,
    #[arg(long)]
    thread: Option<String>,
    #[command(flatten)]
    page: PageArgs,
}
#[derive(Args)]
struct DiagnosticsArgs {
    #[arg(long)]
    seat: Option<String>,
    #[arg(long)]
    thread: Option<String>,
    #[command(flatten)]
    page: PageArgs,
}
#[derive(Args)]
struct WarningsArgs {
    #[arg(long)]
    seat: String,
    #[command(flatten)]
    page: PageArgs,
}
#[derive(Args)]
struct ReadArgs {
    thread: String,
    /// Keep running: print the recent messages (default 20, or --recent N)
    /// oldest first, then each new message as it is committed, IRC style for
    /// a person and one record per line otherwise. Never ACKs or accepts;
    /// Ctrl-C stops it.
    #[arg(long, conflicts_with_all = ["before", "cursor", "limit"])]
    follow: bool,
    /// With --follow: hide system notices (joins, ACKs, warnings).
    #[arg(long, requires = "follow")]
    no_system: bool,
    #[arg(long, group = "initial")]
    recent: Option<u16>,
    #[arg(long, group = "initial")]
    after: Option<u64>,
    #[arg(long, group = "initial")]
    before: Option<u64>,
    #[command(flatten)]
    page: PageArgs,
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
    /// Harness whose hooks to manage. Omitted: every harness (setup: each
    /// one found on PATH; unsetup and setup-status: both).
    #[arg(value_parser = ["claude", "codex"])]
    harness: Option<String>,
    /// Absolute harness executable whose `--version` is observed (default:
    /// the first `claude`/`codex` executable on PATH, as the hook observes).
    #[arg(long, value_name = "PATH")]
    harness_binary: Option<String>,
}

#[derive(Args)]
struct LaunchArgs {
    /// Explicit existing Herdr pane at its interactive shell prompt (never
    /// the focused pane).
    #[arg(long, value_name = "PANE")]
    pane: String,
    /// Native agent to start.
    #[arg(long, value_parser = ["claude", "codex"])]
    kind: String,
    /// Absolute harness executable whose `--version` is gated (default: the
    /// first `claude`/`codex` on PATH, which Herdr starts by name).
    #[arg(long, value_name = "PATH")]
    harness_binary: Option<String>,
    /// Herdr agent name (default: the pane's Herdr label, else `seat-<short
    /// seat id>`). Fitted to Herdr's `[a-z][a-z0-9_-]{0,31}`.
    #[arg(long, value_name = "NAME")]
    name: Option<String>,
    /// Native agent arguments, kept byte for byte and in order after `--`.
    #[arg(last = true, allow_hyphen_values = true, value_name = "AGENT_ARG")]
    agent_args: Vec<String>,
}

#[derive(Args)]
struct ViewArgs {
    #[arg(long)]
    once: bool,
    #[command(flatten)]
    page: PageArgs,
}

fn setup_action(verb: super::setup::SetupVerb, args: SetupArgs) -> Result<CliAction, ApiError> {
    use crate::harness::context::Harness;
    let Some(harness) = args.harness else {
        if args.harness_binary.is_some() {
            return Err(invalid(
                "--harness-binary needs a harness: `setup claude|codex --harness-binary PATH`",
            ));
        }
        return Ok(CliAction::SetupAll(verb));
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
/// A thread argument. The digest form is refused as invalid arguments naming
/// the bare thread ID to pass, never looked up (a lookup of the whole string
/// reported a misleading `current invitation missing (not_found)`).
fn thread_id(value: String) -> Result<ThreadId, ApiError> {
    if let Some((_, thread)) = digest_reference(&value) {
        return Err(invalid(format!(
            "`{value}` is an attention digest item (ITEM@THREAD), not a thread ID; pass the bare thread ID `{thread}`, for example `herdr-threads accept {thread}`"
        )));
    }
    id(value, ThreadId::parse)
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
    let cli = Cli::try_parse_from(argv).map_err(|error| {
        use clap::error::ErrorKind;
        match error.kind() {
            ErrorKind::DisplayHelp | ErrorKind::DisplayVersion => {
                ParseFailure::Informational(error.render().to_string())
            }
            ErrorKind::DisplayHelpOnMissingArgumentOrSubcommand => {
                ParseFailure::Usage(error.render().to_string())
            }
            _ => ParseFailure::Invalid(invalid(error.render().to_string())),
        }
    })?;
    Ok(parse_cli(cli)?)
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

fn parse_cli(cli: Cli) -> Result<ParsedCli, ApiError> {
    let cooperative = match (
        cli.cooperative_seat,
        cli.cooperative_target,
        cli.cooperative_harness,
        cli.cooperative_role,
    ) {
        (None, None, None, None) => None,
        (Some(seat), Some(target), Some(harness), Some(role)) => Some(CooperativeSelection {
            seat: id(seat, SeatId::parse)?,
            target: id(target, HostTargetId::parse)?,
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
    let action = match cli.command {
        Top::Thread { command } => match command {
            ThreadSub::Create { topic, goal } => {
                let goal = goal.unwrap_or_else(|| topic.clone());
                CliAction::Mutation(MutationSpec::Create {
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
            ThreadSub::List(args) => CliAction::Wire(WireCommand::Directory(DirectoryQuery {
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
            seat: id(args.seat, SeatId::parse)?,
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
        Top::Accept { thread } => CliAction::Mutation(MutationSpec::Accept(thread_id(thread)?)),
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
            let body = crate::cli::input::read_body(args.body, args.file, args.stdin)?;
            CliAction::Mutation(MutationSpec::Send {
                thread,
                body,
                require_ack: recipients,
                deadline_millis,
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
        Top::Warnings(args) => CliAction::Wire(WireCommand::Warnings(WarningsQuery {
            seat: id(args.seat, SeatId::parse)?,
            page: page(args.page)?,
        })),
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
                thread: thread_id(args.thread)?,
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
                thread: thread_id(args.thread)?,
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
            SeatSub::List(args) => CliAction::Wire(WireCommand::Seats(SeatsQuery {
                page: page(args)?,
                target: None,
            })),
            SeatSub::Resolve {
                pane,
                new_seat: true,
                ..
            } => CliAction::Mutation(MutationSpec::FreshSeat(id(pane, HostTargetId::parse)?)),
            SeatSub::Resolve { pane, .. } => {
                CliAction::Mutation(MutationSpec::Resolve(id(pane, HostTargetId::parse)?))
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
                pane,
                replace,
                operator,
            } => {
                if !operator {
                    return Err(invalid("operator required"));
                }
                let seat = id(seat, SeatId::parse)?;
                let pane = id(pane, HostTargetId::parse)?;
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
        Top::Doctor => CliAction::Doctor,
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
                target: id(args.pane, HostTargetId::parse)?,
                harness,
                harness_binary: args.harness_binary,
                argv: args.agent_args,
                name: args.name,
                pane_label: None,
            })
        }
        Top::PendingOps(args) => CliAction::PendingOps(page(args)?),
        Top::Skill => CliAction::Skill,
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
        command.validate().map_err(validation_error)?;
    }
    Ok(ParsedCli {
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
    })
}

#[cfg(test)]
mod tests {
    include!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/cli/commands.rs"
    ));
}
