//! `--help` epilogues assembled from generic core text plus the per-adapter fragments the
//! registered adapters declare (core-sweep D3). Generic text names no harness; adapter text
//! is appended verbatim in registry order, and the kind and `HERDR_THREADS_*_OPTS` lists
//! are generated. Each topic has a `*_in` form over an explicit registry for extension tests.
use crate::harness::registry::{Registry, builtins};

const SETUP_HEAD: &str = r#"No harness named: every registered harness. `setup` sets up every detected harness
and prints one line per harness: installed, already
installed or skipped (not on PATH);
`unsetup` removes recorded installations (on PATH or not); `setup-status` reports
every registered harness. Hook-trust reminders are printed once at the end. The exit status is that
of the first harness that failed; skipped harnesses are not failures.

"#;
const SETUP_SCOPE_HEAD: &str = r#"Scope (user level, like Herdr's own agent hooks):"#;
const SETUP_FOREGROUND_HEAD: &str = r#"Foreground agents (user-managed, read-only inspection): setup and setup-status report user
settings and manual advice for every harness."#;
const SETUP_FOREGROUND_TAIL: &str = r#"Setup never sets these choices;
configured settings/arguments do not prove foreground execution and overrides may change them."#;
const SETUP_TAIL: &str = r#"Herdr instance: --state-dir and --host-endpoint, else HERDR_PLUGIN_STATE_DIR and
HERDR_SOCKET_PATH (set inside Herdr), else Herdr's defaults when they exist
($XDG_STATE_HOME or ~/.local/state, then herdr/plugins/herdr-threads; $XDG_CONFIG_HOME or
~/.config, then herdr/herdr.sock), else detected with the herdr CLI (`herdr plugin list`,
`herdr status server`).
Ambiguity is refused. The installed hook command is
`<this executable> --state-dir <state> --host-endpoint <socket> hook <harness>`, registered
for each event with `--event <EVENT>` appended (hooks installed before that keep working;
`doctor` suggests re-running setup); in any session that is not a pane of that Herdr
instance it prints nothing, runs no version probe and starts no daemon, and only sends a
short best-effort evidence note to a daemon already running (gate files under
<state>/harness/evidence).
Each harness's ownership manifest lives in <state>/setup/.

A hook file that already holds exactly this command's hook groups under another setup's
owner marker (for example a hook file copied with its trust into a second config root) is
adopted: setup records a manifest for it and leaves the file byte-identical (action
`adopted`), setup-status and doctor report it installed (adopted), and unsetup removes only
those groups from that file.

Setup resolves the selected executable without invoking --version, --help or schema probes.
Admission is contract_declared: the registered hook contract is configured, runtime metadata
is unknown. Installation does not prove hook delivery, native support or receipt.

Exit status:
  0  installed / removed / nothing to remove / status reported
  1  refused: the file changed concurrently, an owned entry was edited or removed by hand,
     the recorded allow rule was removed by hand, an unowned identical hook exists (for
     some harnesses also in another config layer), a recorded legacy allowance is partial or
     edited, or a file could not be written
  2  invalid arguments, undetectable Herdr instance, or invalid settings file (not a JSON
     object / not valid TOML, symlink, over 1 MiB)
  4  the named harness is missing from PATH or --harness-binary is not an executable file;
     with no harness named an absent harness is skipped"#;

const LAUNCH_HEAD: &str = r#"Target:
  --pane names one existing Herdr pane at its interactive shell prompt; launch never
  creates, splits or picks a pane and never types into an occupied one. Herdr's
  guarded `agent start` refuses a busy pane.

Preflight (nothing is started when any step refuses):
  - the selected executable must resolve; its registered contract is declared without
    invoking --version, --help or schema probes, and runtime metadata remains unknown;
  - the pane's seat is resolved like `seat resolve --pane`; a recovery-held target
    needs `seat rebind ... --operator` or a fresh seat first;
  - the owned user-level hooks must be set up (`herdr-threads setup HARNESS`) in the
    settings or hook file of the @HINTS@ the agent
    will use (an absolute one the pane's shell exports, else launch's own, else HOME;
    the report's config_dir names it);
  - a last fresh read of the pane just before Herdr starts the agent.

Agent name: --name NAME, else the pane's Herdr label, else s<short seat id>
for a new compact ID (seat-<short seat id> for a persisted ID),
fitted to Herdr's [a-z][a-z0-9_-]{0,31}. If another live agent holds it, launch
retries once with -<short seat id> appended.

Arguments after `--` are passed to the agent unchanged and in order. Optional
@OPTS@ are prepended to those arguments;
unset or empty adds nothing. Use shell-style quotes and escapes; variables and
commands are never expanded.
The combined arguments use the same managed-launch guards. The owned configuration
is on disk, so launch adds no hook or daemon-mode arguments by default.
Native options that would replace the owned configuration are refused. No
auto-approve flag is added.

Launch is not receipt: it never checks in, accepts or ACKs. Invitations and messages
sent before launch stay pending; the agent's SessionStart hook shows them, and
`herdr-threads inbox --seat SEAT` lists them even when the initial prompt is lost.
After an observed start, a seat with no open binding gets a `managed_launch` binding
(report `binding`; `seat inspect` shows it as launched, not checked in): it lets the
daemon wake the idle agent for pending work before its first check-in (some harnesses run no
SessionStart hook until their first turn), and the agent's first check-in replaces it.

A harness that exits right after the start (for example a harness refusing its arguments)
fails launch at once with invalid_request and the pane's last lines.

Harness choices come from the same-binary registry."#;
const LAUNCH_TAIL: &str = r#"Exit status: 0 startup observed; 5 start may have happened but was not confirmed:
inspect the pane (`herdr agent get` / `herdr agent read`) before launching again."#;

const HANDOFF_TEMPLATE: &str = r#"Choose exactly one channel: --new-thread or --thread ID_OR_NAME.
Choose one target mode:
  --pane PANE --kind @KINDS@ launches in an existing explicit pane;
  --new-tab LABEL --kind @KINDS@ creates a tab and launches there;
  --existing with exactly one --seat SEAT or --pane PANE delivers durable work
    to an existing canonical seat without launching or restarting its session.
New modes require an original top-level Agent and the guarded daemon capability.
--new-tab conflicts with --existing, --tab, --pane and --seat. --cwd is new-tab
only: an absolute existing directory, defaulting to the invocation cwd, normalized
before publication. --space selects one live workspace by exact ID or unique label;
omission for creation uses the caller's live workspace, never UI focus. New-tab is
intentional creation even if its label already exists; it does not focus the tab.
For --existing, --space and --tab may qualify --pane, but conflict with --seat.
Delivery forbids --kind, --harness-binary, --name and --agent-arg and ignores the
launch-option environment. An unresolved, held, retired or foreign seat refuses.
An unbound resolved seat may receive staged work; this does not establish a
working or available session. Delivery reports staged work, not a launch result.

New threads join the sender; existing threads require a joined sender. --thread-name,
--topic and --goal apply only to new threads. Topic defaults to Handoff to DISPLAY:
legacy launch uses the pane display, new-tab uses its label, delivery uses the
canonical seat ID. Goal defaults to topic. --name names the native agent, not the channel.
The one quoted body after -- is durable work (1..1024 UTF-8 bytes), never native argv.
Native launch options use repeatable --agent-arg=OPTION; launch -- native arguments
is unchanged. @OPTS@ prepend optional
arguments using shell-style quotes and escapes without variable or command
expansion. Unset or empty adds nothing. Launch handoff freezes the combined options
before preflight; retry uses saved arguments even if the environment changes.
Handoff stages an invitation when needed and a message addressed to the exact
recipient; launch modes then perform guarded launch. Startup gets fixed inbox
instructions, not a second copy of the body. Handoff never accepts or ACKs for
the recipient, nor declares task adoption or task completion.
Native execution still needs the selected harness's approval/configuration.

Committed work survives failure. pending-ops lists the compound reference; retry REF
resumes exact keyed steps without duplicate messages or invitations. A confirmed
pre-start refusal can retry after repair. Unknown creation cannot automatically
create another tab: inspect the reported exact namespace and attempt. It is distinct
from downstream possible start; possible start, unknown launch outcome or a crash
across launch submission never automatically launches again. Inspect the exact
downstream pane/seat before using any reported manual launch argv after confirming no
agent started. Manual launch guidance does not prove tab noncreation.
Completed retry presents the retained historical report and cleans its own local
intent without repeating effects or proving current availability.

Administrative bootstrap recovery uses human immediately after the executable:
  herdr-threads human [GLOBALS] handoff recover REF --attempt N --created-pane EXACT_PANE
  herdr-threads human [GLOBALS] handoff recover REF --attempt N --not-created
  herdr-threads human [GLOBALS] handoff recover REF --attempt N --cancel --reason TEXT
Keep the exact reported reference, positive attempt N, state directory and endpoint;
routing/output globals follow immediate human. Root --human selects output only.
Recovery needs the guarded daemon capability and records a separate local-account
operator assertion, preserving the original agent identity. Created-pane asserts
this exact result belongs to the inspected attempt and needs fresh coherent
structural evidence and ordinary guards; labels are not ownership evidence.
Not-created asserts inspected noncreation and quiescence. Cancellation asserts
quiescence and administrative abandonment, not completion; reason is nonblank and
at most 4096 UTF-8 bytes. A known in-flight invocation refuses conflicting recovery;
snapshots or guessed PIDs do not prove quiescence. Recovery never launches downstream
work. Bootstrap cancellation refuses while an exact legacy child fence or live hint
remains; that child may stay protected indefinitely after pane loss or retirement.
Recovery replay presents its exact recorded decision, never authorizes a newer
attempt. Product cleanup never closes created topology."#;

/// Non-empty `fields` of every registered adapter, in registry order.
fn fragments(
    registry: &Registry,
    field: impl Fn(&crate::harness::adapter::AdapterText) -> &'static str,
) -> Vec<&'static str> {
    registry
        .registrations()
        .iter()
        .map(|registration| field(registration.text()))
        .filter(|text| !text.is_empty())
        .collect()
}

/// The config-root variables adapters name (`A / B`), or a neutral phrase when none does.
pub(crate) fn config_root_hints(registry: &Registry) -> String {
    let hints = fragments(registry, |text| text.config_root_hint);
    if hints.is_empty() {
        "config root".to_owned()
    } else {
        hints.join(" / ")
    }
}

/// `A / B` over the native-options variables of the accepted adapters, or a neutral phrase.
fn options_list(registry: &Registry, include: impl Fn(&str) -> bool) -> String {
    let keys = super::launch::registered_options_keys(registry, include);
    if keys.is_empty() {
        "The native-options variables".to_owned()
    } else {
        keys.join(" / ")
    }
}

/// The `--kind` values a bootstrap plan accepts (the V1 targets and every adapter with a
/// launch policy), in registry order.
fn bootstrap_kinds(registry: &Registry) -> Vec<&'static str> {
    registry
        .registrations()
        .iter()
        .map(|registration| registration.metadata().id)
        .filter(|id| crate::harness::launch::bootstrap_target(registry, id).is_some())
        .collect()
}

pub fn setup_help() -> String {
    setup_help_in(builtins())
}

pub fn setup_help_in(registry: &Registry) -> String {
    let mut help = String::from(SETUP_HEAD);
    help.push_str(SETUP_SCOPE_HEAD);
    for scope in fragments(registry, |text| text.setup_scope) {
        help.push('\n');
        help.push_str(scope);
    }
    help.push_str("\n\n");
    help.push_str(SETUP_FOREGROUND_HEAD);
    for foreground in fragments(registry, |text| text.setup_foreground) {
        help.push(' ');
        help.push_str(foreground);
    }
    help.push(' ');
    help.push_str(SETUP_FOREGROUND_TAIL);
    for notes in fragments(registry, |text| text.setup_notes) {
        help.push_str("\n\n");
        help.push_str(notes);
    }
    help.push_str("\n\n");
    help.push_str(SETUP_TAIL);
    help
}

pub fn launch_help() -> String {
    launch_help_in(builtins())
}

pub fn launch_help_in(registry: &Registry) -> String {
    let mut help = LAUNCH_HEAD
        .replace("@HINTS@", &config_root_hints(registry))
        .replace("@OPTS@", &options_list(registry, |_| true));
    for notes in fragments(registry, |text| text.launch_notes) {
        help.push_str("\n\n");
        help.push_str(notes);
    }
    help.push_str("\n\n");
    help.push_str(LAUNCH_TAIL);
    help
}

pub fn handoff_help() -> String {
    handoff_help_in(builtins())
}

pub fn handoff_help_in(registry: &Registry) -> String {
    let kinds = bootstrap_kinds(registry);
    HANDOFF_TEMPLATE
        .replace("@KINDS@", &kinds.join("|"))
        .replace("@OPTS@", &options_list(registry, |id| kinds.contains(&id)))
}

pub fn me_init_help() -> String {
    me_init_help_in(builtins())
}

pub fn me_init_help_in(registry: &Registry) -> String {
    let markers: Vec<_> = registry.person_guard_markers().map(|m| m.name).collect();
    let refusal = if markers.is_empty() {
        "a registered agent harness reported by Herdr is".to_owned()
    } else {
        format!(
            "agent markers ({}) or a registered agent harness reported by Herdr are",
            markers.join(", ")
        )
    };
    format!(
        "Run it in your own shell pane, then use `ht human` for thread create, invite, send, \
read, ack and accept there with no --cooperative-* flags. Your actions are recorded as \
operator_human, never as an agent. Messages remain readable, but a human seat owes no ACK and \
gets no overdue-ACK warning, even when a sender uses --require-ack. Re-run `human me init` after a daemon restart to \
mark yourself available again. It is refused where {refusal} present, and over \
a seat bound to an agent; `human me init --operator` overrides that as the local account (later \
commands in this pane then run as you)."
    )
}
