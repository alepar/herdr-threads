//! `internal peer-handoff-launch`: the child a node's peer-launch worker runs for a handoff
//! request instead of plain `launch --pane` (spec section 3.2, Herdr target).
//!
//! Responsibility (handoff launch child)
//! - Owns: the order preflight, then staging, then launch, for one accepted handoff request:
//!   read the stored plan and frozen prompt from the daemon, compose the launch argv from the
//!   startup-input template, the frozen prompt and the plan's profile directory context, resolve
//!   the pane's seat, stage the thread, join and message through `StagingAuthor::PeerRequest`,
//!   and only then start the harness.
//! - Hides: that staging is one daemon command and that the profile directory projection is the
//!   one `handoff_directory` froze at decide.
//! - Must not: stage before everything that can refuse has passed, set `Failed.staged` (the
//!   recorded-answer builder derives it from the row), or fall back to a two-step path on a
//!   daemon without `PEER_HANDOFF_STAGE_V1`.

use std::{io::Write, sync::Arc};

use super::{
    RunError,
    handoff::{self, HandoffLauncher, StagedWork, Staging, StagingAuthor},
    launch::LaunchRequest,
};
use crate::{
    ports::LocalClient,
    protocol::{
        capabilities::PEER_HANDOFF_STAGE_V1,
        commands::Command,
        ids::{HostTargetId, OperationId},
        output::OutputFormat,
        results::{CommandResult, PeerRequestView},
        time::Clock,
    },
};

/// The launch request a handoff plan asks for on `target`: the plan's harness and arguments,
/// the profile directory context `decide` validated (the same `handoff_directory` call).
fn launch_request(
    view: &PeerRequestView,
    target: HostTargetId,
    pane_label: Option<String>,
) -> Result<LaunchRequest, RunError> {
    let plan = &view.plan;
    let id = crate::harness::registry::builtins()
        .agent(&plan.kind)
        .map_err(|_| super::invalid_request("the request's harness is not registered here"))?;
    let directory_context = crate::service::peer_launcher::handoff_directory(plan)
        .map_err(|detail| super::invalid_request(&detail))?;
    Ok(LaunchRequest {
        directory_context,
        target,
        harness: crate::harness::registry::OccupantHarness::Agent(id).into(),
        harness_binary: None,
        argv: plan.argv.clone(),
        name: None,
        pane_label,
    })
}

/// Runs request `request` on `target`: capability, view, preflight, stage, launch, report.
#[allow(clippy::too_many_arguments)]
pub(super) fn run_with<C: LocalClient + ?Sized, W: Write>(
    request: &str,
    target: HostTargetId,
    pane_label: Option<String>,
    client: &C,
    launcher: &mut dyn HandoffLauncher,
    clock: &dyn Clock,
    format: OutputFormat,
    writer: &mut W,
) -> Result<(), RunError> {
    if !client.supports_capability(PEER_HANDOFF_STAGE_V1, &super::cooperative_budget(clock)) {
        return Err(super::invalid_request(&super::older_daemon_refusal(
            client,
            "peer handoff staging",
            &|| super::cooperative_budget(clock),
        )));
    }
    let CommandResult::PeerRequestView(view) = client.call(
        Command::PeerRequestView {
            request: request.to_owned(),
        },
        &super::cooperative_budget(clock),
    )?
    else {
        return Err(super::invalid_request("unexpected peer request view"));
    };
    if view.state != "running" {
        return Err(super::invalid_request("peer request is no longer running"));
    }
    let prompt = view
        .handoff_prompt
        .as_deref()
        .ok_or_else(|| super::invalid_request("peer request froze no handoff prompt"))?;
    let launch = launch_request(&view, target, pane_label)?;
    // Everything that can refuse runs before the first fact is written.
    let (effective, recipient) = handoff::prepare_peer_startup(launcher, &launch, prompt)?;
    let key = OperationId::new(request);
    let mut staged = StagedWork::default();
    let mut phase = "stage";
    handoff::stage_work(
        Staging {
            policy: None,
            channel: &crate::protocol::handoff::HandoffChannel::Automatic,
            body: "",
            recipient: &recipient,
            create_key: &key,
            invite_key: &key,
            send_key: &key,
            skip_joined: false,
        },
        &mut staged,
        &mut phase,
        StagingAuthor::PeerRequest { request },
        &mut |_| Ok(()),
        client,
        clock,
    )?;
    let outcome = launcher.launch(&effective, &recipient, &mut |_| Ok(()))?;
    let bytes = match format {
        OutputFormat::Json => {
            let mut bytes = serde_json::to_vec(&super::launch::json_envelope(outcome.report))
                .map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidData, error))?;
            bytes.push(b'\n');
            bytes
        }
        OutputFormat::Text => super::setup::render_text(&outcome.report).into_bytes(),
    };
    writer.write_all(&bytes)?;
    writer.flush()?;
    if outcome.exit != 0 {
        return Err(RunError::Exit(outcome.exit));
    }
    Ok(())
}

/// Production composition: `run_launch`'s host, seat resolution and daemon connection, with the
/// native launcher.
pub(super) fn run<W: Write>(
    request: &str,
    pane: &str,
    parsed: &super::commands::ParsedCli,
    context: &super::RuntimeContext,
    paths: &super::NodePaths,
    clock: &Arc<dyn Clock>,
    writer: &mut W,
) -> Result<(), RunError> {
    let target = HostTargetId::parse(pane.to_owned()).map_err(super::invalid_request)?;
    let mut env =
        super::setup::SetupEnv::from_process(&parsed.output, parsed.caller_site.as_ref())?;
    env.state_dir = Some(context.state_dir.clone());
    let claim_host = super::topology_runtime::herdr_endpoint_io(
        context,
        &parsed.output,
        parsed.caller_site.as_ref(),
    )
    .map_err(|_| super::no_herdr_host(&format!("pane {pane}")))?;
    let herdr_locator = crate::protocol::ids::HostLocator::parse(claim_host.to_string_lossy())
        .map_err(|detail| super::invalid_request(&detail.detail))?;
    env.host_endpoint = Some(claim_host.clone());
    let (instance, _, client) = super::connect(paths, clock)?;
    let journal = super::journal::Journal::open(paths.node_dir.join("intents"))?;
    let handle = super::host_client::LaunchHandle::herdr(claim_host, Arc::clone(clock));
    let host = handle.host()?;
    // The pane's Herdr label names the agent, as for a plain launch. Best effort.
    let pane_label = host
        .pane_names(&super::cooperative_budget(clock.as_ref()))
        .ok()
        .and_then(|panes| super::launch::pane_label(&target, &panes));
    let seats = super::launch::DaemonSeatResolver::new(
        &client,
        &journal,
        instance,
        herdr_locator,
        clock.as_ref(),
    );
    let shell_probe = super::launch::SystemShellProbe::from_process();
    let mut launcher = handoff::NativeLauncher {
        registry: crate::harness::registry::builtins(),
        parts: super::launch::LaunchParts {
            env: &env,
            host,
            seats: &seats,
            handoff: &client,
            clock: clock.as_ref(),
            record_dir: Some(&paths.node_dir),
            shell_probe: &shell_probe,
        },
    };
    // Test-support builds only: a fake Herdr cannot start a harness, so a multinode test names a
    // file to receive the launch request the child would have submitted.
    #[cfg(feature = "test-support")]
    if let Some(record) = std::env::var_os(RECORD_ENV) {
        let mut recording = RecordingLauncher {
            inner: launcher,
            record: record.into(),
        };
        return run_with(
            request,
            target,
            pane_label,
            &client,
            &mut recording,
            clock.as_ref(),
            parsed.output.format,
            writer,
        );
    }
    run_with(
        request,
        target,
        pane_label,
        &client,
        &mut launcher,
        clock.as_ref(),
        parsed.output.format,
        writer,
    )
}

/// The file `HT_TEST_HANDOFF_LAUNCH_RECORD` names receives the launch request (JSON, one line)
/// the child would have submitted; the harness is not started. Before it, the launch waits while
/// `<file>.hold` exists, and fails (exit 1, as a launch that did not start) while `<file>.fail`
/// exists: the two points a test needs after staging.
#[cfg(feature = "test-support")]
pub const RECORD_ENV: &str = "HT_TEST_HANDOFF_LAUNCH_RECORD";

/// Preflights like `inner`, but records the final launch request instead of submitting it.
#[cfg(feature = "test-support")]
struct RecordingLauncher<L> {
    inner: L,
    record: std::path::PathBuf,
}

#[cfg(feature = "test-support")]
impl<L: HandoffLauncher> HandoffLauncher for RecordingLauncher<L> {
    fn native_options_env(
        &self,
        request: &LaunchRequest,
    ) -> Result<Option<&'static str>, crate::protocol::results::ApiError> {
        self.inner.native_options_env(request)
    }
    fn select_startup_input(
        &mut self,
        request: &LaunchRequest,
    ) -> Result<crate::harness::adapter::StartupInputTemplate, RunError> {
        self.inner.select_startup_input(request)
    }
    fn native_input(&mut self, request: &LaunchRequest) -> Result<Vec<String>, RunError> {
        self.inner.native_input(request)
    }
    fn preflight_startup(
        &mut self,
        original: &LaunchRequest,
        effective: &LaunchRequest,
    ) -> Result<handoff::StartupPreflight, RunError> {
        self.inner.preflight_startup(original, effective)
    }
    fn preflight_saved(
        &mut self,
        original: &LaunchRequest,
        effective: &LaunchRequest,
        recipient: &crate::protocol::ids::SeatId,
    ) -> Result<handoff::StartupPreflight, RunError> {
        self.inner.preflight_saved(original, effective, recipient)
    }
    fn preflight(
        &mut self,
        request: &LaunchRequest,
    ) -> Result<crate::protocol::ids::SeatId, RunError> {
        self.inner.preflight(request)
    }
    fn launch(
        &mut self,
        request: &LaunchRequest,
        recipient: &crate::protocol::ids::SeatId,
        gate: &mut dyn FnMut(bool) -> Result<(), crate::protocol::results::ApiError>,
    ) -> Result<super::launch::LaunchReport, RunError> {
        let beside = |suffix: &str| {
            let mut name = self.record.clone().into_os_string();
            name.push(suffix);
            std::path::PathBuf::from(name)
        };
        while beside(".hold").exists() {
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
        if beside(".fail").exists() {
            return Err(super::invalid_request("launch refused by the test"));
        }
        gate(true)?;
        let line = serde_json::to_string(request).map_err(std::io::Error::other)?;
        std::fs::write(&self.record, format!("{line}\n"))?;
        Ok(super::launch::LaunchReport {
            report: serde_json::json!({
                "outcome": "started",
                "seat": recipient,
                "pane": request.target,
                "harness": request.harness.as_str(),
                "argv": request.argv,
            }),
            exit: 0,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        protocol::{
            ids::{MessageId, SeatId, ThreadId},
            results::ApiError,
            time::{CallBudget, MonoInstant, UtcMillis},
        },
        sync::{
            frame::{LaunchPlan, LaunchTarget},
            handoff_block::SenderBlock,
        },
    };
    use std::sync::Mutex;

    const PROMPT: &str = "fix the parser \u{2014} from: lead (sLEADseat001)";

    struct TestClock;
    impl Clock for TestClock {
        fn utc_now(&self) -> UtcMillis {
            UtcMillis(1_000)
        }
        fn monotonic_now(&self) -> MonoInstant {
            MonoInstant(0)
        }
    }

    /// One log both fakes append to, so the order across the daemon and the launcher is seen.
    type Log = Arc<Mutex<Vec<String>>>;

    struct FakeDaemon {
        log: Log,
        stage_cap: bool,
        plan: LaunchPlan,
    }
    impl LocalClient for FakeDaemon {
        fn supports_capability(&self, name: &str, _: &CallBudget) -> bool {
            self.stage_cap && name == PEER_HANDOFF_STAGE_V1
        }
        fn call_with_output(
            &self,
            command: Command,
            _: &crate::protocol::output::OutputSpec,
            budget: &CallBudget,
        ) -> Result<CommandResult, ApiError> {
            self.call(command, budget)
        }
        fn call(&self, command: Command, _: &CallBudget) -> Result<CommandResult, ApiError> {
            match command {
                Command::PeerRequestView { .. } => {
                    self.log.lock().unwrap().push("view".into());
                    Ok(CommandResult::PeerRequestView(Box::new(PeerRequestView {
                        plan: self.plan.clone(),
                        handoff_prompt: Some(PROMPT.into()),
                        state: "running".into(),
                    })))
                }
                Command::PeerHandoffStage { request, seat } => {
                    self.log
                        .lock()
                        .unwrap()
                        .push(format!("stage {request} {}", seat.as_str()));
                    Ok(CommandResult::PeerHandoffStaged {
                        thread: ThreadId::new("pair-thread"),
                        message: MessageId::new("m-staged"),
                    })
                }
                _ => Err(ApiError::unsupported("not in this fake")),
            }
        }
    }

    struct StubLauncher {
        log: Log,
        launched: Option<LaunchRequest>,
        fail_launch: bool,
    }
    impl HandoffLauncher for StubLauncher {
        fn preflight(&mut self, _: &LaunchRequest) -> Result<SeatId, RunError> {
            self.log.lock().unwrap().push("preflight".into());
            Ok(SeatId::new("sRECIPIENT01"))
        }
        fn launch(
            &mut self,
            request: &LaunchRequest,
            recipient: &SeatId,
            _: &mut dyn FnMut(bool) -> Result<(), ApiError>,
        ) -> Result<crate::cli::launch::LaunchReport, RunError> {
            self.log.lock().unwrap().push("launch".into());
            self.launched = Some(request.clone());
            if self.fail_launch {
                return Err(super::super::invalid_request("host refused"));
            }
            Ok(crate::cli::launch::LaunchReport {
                report: serde_json::json!({"outcome": "started", "seat": recipient}),
                exit: 0,
            })
        }
    }

    fn plan() -> LaunchPlan {
        LaunchPlan {
            target: LaunchTarget::Pane {
                space: None,
                tab: None,
                pane: "w1:p2".into(),
            },
            kind: "claude".into(),
            argv: vec!["--model".into(), "x".into()],
            cwd: None,
            profile_dir: Some("/".into()),
            handoff_prompt: Some("fix the parser".into()),
            message: Some(Box::new(SenderBlock {
                v: 1,
                ..Default::default()
            })),
        }
    }

    struct Run {
        result: Result<(), RunError>,
        log: Vec<String>,
        launched: Option<LaunchRequest>,
        stdout: String,
    }

    fn run_child(stage_cap: bool, fail_launch: bool) -> Run {
        let log = Log::default();
        let daemon = FakeDaemon {
            log: Arc::clone(&log),
            stage_cap,
            plan: plan(),
        };
        let mut launcher = StubLauncher {
            log: Arc::clone(&log),
            launched: None,
            fail_launch,
        };
        let mut out = Vec::new();
        let result = run_with(
            "req-1",
            HostTargetId::new("w1:p2"),
            None,
            &daemon,
            &mut launcher,
            &TestClock,
            OutputFormat::Json,
            &mut out,
        );
        let log = log.lock().unwrap().clone();
        Run {
            result,
            log,
            launched: launcher.launched,
            stdout: String::from_utf8(out).unwrap(),
        }
    }

    // Kills: staging before the pane's seat is resolved (a refused preflight would leave facts),
    // launching before the facts exist (the agent could start with no task), and a different
    // order of the three.
    #[test]
    fn child_stages_before_launch() {
        let run = run_child(true, false);
        run.result.unwrap();
        assert_eq!(
            run.log,
            ["view", "preflight", "stage req-1 sRECIPIENT01", "launch"]
        );
        // The launch envelope the worker reads its seat from.
        assert_eq!(
            crate::cli::launch::json_envelope_seat(&run.stdout).as_deref(),
            Some("sRECIPIENT01")
        );
    }

    // Kills: an argv built from the plan's summary instead of the frozen prompt, the prompt not
    // in the adapter's startup-input slot, the caller's own arguments lost, and a profile
    // directory projected by a second policy rather than the one `decide` validated.
    #[test]
    fn child_argv_carries_frozen_prompt_and_profile_env() {
        let run = run_child(true, false);
        run.result.unwrap();
        let launched = run.launched.unwrap();
        assert_eq!(&launched.argv[..2], ["--model", "x"]);
        assert_eq!(
            launched.argv.iter().filter(|a| a.contains(PROMPT)).count(),
            1,
            "{:?}",
            launched.argv
        );
        let expected = crate::service::peer_launcher::handoff_directory(&plan())
            .unwrap()
            .expect("the plan names a profile directory");
        assert!(!expected.env.is_empty());
        assert_eq!(launched.directory_context, Some(expected));
    }

    // Kills: a daemon without the capability reaching the stage call (an older daemon drops the
    // command with no reply), and a failure that is silent.
    #[test]
    fn missing_stage_cap_fails_without_fact() {
        let run = run_child(false, false);
        let error = run.result.unwrap_err().to_string();
        assert!(error.contains("peer handoff staging"), "{error}");
        assert!(run.log.is_empty(), "{:?}", run.log);
        assert!(run.stdout.is_empty());
    }

    // Kills: a launch failure that reports success, one that undoes the staged facts, and a
    // child that prints a `staged` field itself (the recorded-answer builder derives it).
    #[test]
    fn failure_after_staging_reports_failed() {
        let run = run_child(true, true);
        assert!(run.result.is_err());
        assert_eq!(
            run.log,
            ["view", "preflight", "stage req-1 sRECIPIENT01", "launch"]
        );
        assert!(run.stdout.is_empty(), "{}", run.stdout);
    }
}
