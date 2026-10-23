//! A hook-pull seat's standard hook never reads an inbox page or completes lazy rows
//! (2026-10-10 delivery model): its lazy rows go only through `DeliveryPull`. The seat is the
//! synthetic fourth fixture, which declares hook-pull delivery; the daemon is a scripted
//! in-process service that advertises the inbox page and has one lazy row pending.
use super::*;
use crate::protocol::{
    attention::{AttentionClass, AttentionDigest},
    ids::{MessageId, ThreadId},
    results::{
        CapabilityList, CheckInContextDisposition, CheckInResult, ContinuityStatus, MappingStatus,
        SeatInspection, SeatSummary,
    },
};
use crate::test_support::{isolation::TestIsolation, synthetic_fourth::ADAPTER};
use std::os::unix::fs::DirBuilderExt;
use std::sync::atomic::{AtomicBool, Ordering};

fn herdr() -> HookEnv {
    HookEnv {
        herdr_env: true,
        pane: Some("w9:p1".into()),
    }
}

fn payload(event: &str, event_id: &str) -> Vec<u8> {
    serde_json::to_vec(&serde_json::json!({
        "event": event, "event_id": event_id, "session_id": "s1", "role": "top_level"
    }))
    .unwrap()
}

fn empty<T>() -> crate::protocol::pagination::Page<T> {
    crate::protocol::pagination::Page {
        items: vec![],
        next_cursor: None,
        next_argv: None,
        high_water_ordinal: 0,
        scope_revision: None,
        has_more: false,
        stop_reason: crate::protocol::pagination::StopReason::Complete,
        consistency: crate::protocol::pagination::Consistency::BoundedLive,
    }
}

/// A scripted daemon that records every command; `lazy_pending` says whether the digest
/// reports one pending lazy row.
#[derive(Default)]
struct Service {
    commands: Mutex<Vec<Command>>,
    lazy_pending: AtomicBool,
}
impl Service {
    fn count(&self, matching: impl Fn(&Command) -> bool) -> usize {
        self.commands
            .lock()
            .unwrap()
            .iter()
            .filter(|c| matching(c))
            .count()
    }
}
impl crate::ports::LocalService for Service {
    fn service_control(
        &self,
        _: Command,
        _: crate::protocol::authority::PeerIdentity,
        _: &str,
        _: &str,
        _: &crate::service::live_gate::LiveServiceGate,
        _: &CallBudget,
    ) -> Result<CommandResult, ApiError> {
        Err(crate::test_support::unserved("no service control"))
    }
    fn audit_service_disconnect(
        &self,
        _: &str,
        _: u64,
        _: crate::protocol::authority::PeerIdentity,
        _: &CallBudget,
    ) -> Result<(), ApiError> {
        Err(crate::test_support::unserved("no service disconnect"))
    }
    fn service_operation(
        &self,
        _: crate::protocol::service::ServiceOperation,
        _: &crate::ports::ServiceConnectionAuthority,
        _: &dyn crate::ports::ServiceAuthorityGate,
        _: &CallBudget,
    ) -> Result<crate::protocol::service::ServiceResult, ApiError> {
        Err(crate::test_support::unserved("no service operation"))
    }
    fn handle_with_output(
        &self,
        c: Command,
        p: crate::protocol::authority::PeerIdentity,
        b: &CallBudget,
        _: &OutputSpec,
    ) -> Result<CommandResult, ApiError> {
        self.handle(c, p, b)
    }
    fn handle(
        &self,
        c: Command,
        _: crate::protocol::authority::PeerIdentity,
        _: &CallBudget,
    ) -> Result<CommandResult, ApiError> {
        self.commands.lock().unwrap().push(c.clone());
        let summary = || SeatSummary {
            seat: SeatId::new("callback-seat"),
            continuity: ContinuityStatus::Resolved,
            target: Some(HostTargetId::new("w9:p1")),
            generation: 1,
            created_at: crate::protocol::time::UtcMillis(0),
            retired_at: None,
            active: true,
            availability: None,
            bindings: Vec::new(),
        };
        match c {
            Command::Capabilities => Ok(CommandResult::Capabilities(CapabilityList {
                capabilities: vec![
                    crate::protocol::capabilities::INBOX_BATCH_V2.into(),
                    crate::protocol::capabilities::LAZY_HOOK_DELIVERY.into(),
                ],
            })),
            Command::Seats(_) => {
                let mut page = empty();
                page.items.push(summary());
                Ok(CommandResult::Seats(page))
            }
            Command::ResolveSeat(_) => {
                Ok(CommandResult::SeatResolved(SeatId::new("callback-seat")))
            }
            Command::SeatInspect(_) => Ok(CommandResult::SeatInspect(SeatInspection {
                summary: summary(),
                mapping: MappingStatus {
                    state: ContinuityStatus::Resolved,
                    target: Some(HostTargetId::new("w9:p1")),
                    detail_argv: None,
                },
                hold: None,
                retirement: None,
                open_binding: None,
                history: empty(),
            })),
            Command::Directory(_) => Ok(CommandResult::Directory(empty())),
            Command::AttentionDigest(_) => {
                let none = || AttentionClass {
                    count: 0,
                    items: vec![],
                    has_more: false,
                    count_has_more: false,
                };
                let pending = self.lazy_pending.load(Ordering::SeqCst);
                let mut token = crate::protocol::attention::AttentionToken::default();
                if pending {
                    token.lazy = Some((5, 1));
                }
                Ok(CommandResult::AttentionDigest(AttentionDigest {
                    version: 1,
                    seat: SeatId::new("callback-seat"),
                    token,
                    invitations: none(),
                    receipts: none(),
                    warnings: none(),
                    unavailability_open: false,
                    mod_channel_live: false,
                    lazy: pending.then(|| AttentionClass {
                        count: 1,
                        items: vec![crate::protocol::attention::AttentionRef {
                            id: "m-lazy".into(),
                            thread: ThreadId::new("t-lazy"),
                            requirement: None,
                        }],
                        ..none()
                    }),
                    mail: None,
                    no_turn_delivery: false,
                }))
            }
            Command::CheckIn(check_in) => {
                let mut claim = check_in.claim;
                claim.binding_generation = 2;
                Ok(CommandResult::CheckedIn(CheckInResult {
                    seat: claim.seat.clone(),
                    context: claim,
                    context_disposition: CheckInContextDisposition::Current,
                    offered_through: None,
                    warning_count: 0,
                    warning_count_has_more: false,
                    warnings: empty(),
                    notices: Default::default(),
                    inbox: empty(),
                    fast_forward: None,
                    first_contact: None,
                }))
            }
            Command::InboxBatchV2(_) => {
                let mut page = empty();
                page.items
                    .push(crate::protocol::results::InboxBatchV2Item::LazyMessage {
                        priority: None,
                        thread: ThreadId::new("t-lazy"),
                        topic_data: "lazy topic".into(),
                        message: MessageId::new("m-lazy"),
                        sequence: 1,
                        sender: None,
                        author_role: None,
                        relays_user: false,
                        user_intent: None,
                        author_role_backfilled: false,
                        body: "lazy body".into(),
                        body_start: 0,
                        body_end: 9,
                        body_len: 9,
                        wakes: false,
                    });
                Ok(CommandResult::InboxBatchV2(page))
            }
            _ => Err(crate::test_support::unserved("unused fixture operation")),
        }
    }
}

struct Server {
    shutdown: Cancellation,
    worker: Option<std::thread::JoinHandle<()>>,
}
impl Drop for Server {
    fn drop(&mut self) {
        self.shutdown.cancel();
        self.worker.take().unwrap().join().unwrap();
    }
}

struct Fixture {
    _iso: TestIsolation,
    args: HookArgs,
    service: Arc<Service>,
    _server: Server,
}
fn fixture(label: &str) -> Fixture {
    let iso = TestIsolation::new(label);
    let root = iso.state_root();
    let mut args = HookArgs {
        state_dir: Some(root.join("state")),
        host_endpoint: Some(root.join("host.sock")),
        harness: Harness::Claude,
        event: None,
    };
    args.harness = crate::harness::registry::OccupantHarness::Agent(
        crate::harness::registry::builtins()
            .agent(ADAPTER.metadata().id)
            .unwrap(),
    )
    .into();
    let service = Arc::new(Service::default());
    let context = RuntimeContext::explicit(
        args.state_dir.clone().unwrap(),
        args.host_endpoint.clone().unwrap(),
        None,
    )
    .unwrap();
    let paths = NodePaths::resolve(&context.state_dir).unwrap();
    let owner = crate::daemon::ownership::OwnerLock::acquire_with_test_hint(&paths).unwrap();
    let instance = owner.node_uuid().unwrap();
    let listener = owner.bind_socket().unwrap();
    owner
        .publish_endpoint(
            &listener,
            env!("CARGO_PKG_VERSION"),
            crate::protocol::wire::PROTOCOL_VERSION,
        )
        .unwrap();
    let handler = Arc::clone(&service);
    let shutdown = Cancellation::default();
    let stopped = shutdown.clone();
    let worker = std::thread::spawn(move || {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        runtime.block_on(async {
            let _ = crate::daemon::transport::serve(
                listener.into_async().unwrap(),
                instance,
                handler,
                Arc::new(SystemClock::new()),
                crate::daemon::paths::effective_uid(),
                stopped,
            )
            .await
            .unwrap();
        });
        drop(owner);
    });
    // The state dir exists already (the owner lock made it); keep the mode strict.
    let _ = std::fs::DirBuilder::new()
        .mode(0o700)
        .create(args.state_dir.as_ref().unwrap());
    Fixture {
        _iso: iso,
        args,
        service,
        _server: Server {
            shutdown,
            worker: Some(worker),
        },
    }
}

fn run(fixture: &Fixture, event: &str, event_id: &str) -> HookOutcome {
    let registration = crate::harness::registry::builtins()
        .by_id(
            crate::harness::registry::builtins()
                .agent(ADAPTER.metadata().id)
                .unwrap(),
        )
        .unwrap();
    let mut args = fixture.args.clone();
    args.event = Some(event.into());
    run_hook_registered(
        registration,
        &args,
        &InstalledHarness::Claude("unused".into()),
        &payload(event, event_id),
        &herdr(),
        Instant::now() + LIFECYCLE_BUDGET,
        Arc::new(SystemClock::new()),
        None,
    )
}

fn context_text(outcome: &HookOutcome) -> String {
    let value: serde_json::Value = serde_json::from_slice(&outcome.stdout).unwrap();
    value["synthetic_context"].as_str().unwrap_or("").to_owned()
}

/// Asserts the seat read no inbox page, completed nothing and showed no inbox block.
fn assert_no_page(fixture: &Fixture, outcome: &HookOutcome) {
    assert_eq!(
        fixture
            .service
            .count(|c| matches!(c, Command::InboxBatchV2(_) | Command::Inbox(_))),
        0,
        "a hook-pull seat's hook reads no inbox page: {:?}",
        fixture.service.commands.lock().unwrap()
    );
    assert_eq!(
        fixture
            .service
            .count(|c| matches!(c, Command::CompleteInboxDelivery(_))),
        0,
        "a hook-pull seat's hook completes no lazy row"
    );
    assert!(
        !context_text(outcome).contains(crate::cli::hook_inbox::INBOX_HEADER),
        "no inbox block in the context: {:?}",
        String::from_utf8_lossy(&outcome.stdout)
    );
}

/// Kills: a lifecycle check-in that appends an inbox page for a hook-pull seat (the
/// double-delivery path: the page completes the row while `DeliveryPull` would offer it too).
#[test]
fn hook_pull_lifecycle_check_in_reads_no_inbox_page() {
    let fixture = fixture("hook-pull-lifecycle");
    fixture.service.lazy_pending.store(true, Ordering::SeqCst);
    let outcome = run(&fixture, "SyntheticStart", "e1");
    assert_eq!(outcome.diagnostic, None);
    // The lifecycle context is still presented, with the digest that counts the lazy row.
    assert!(
        fixture
            .service
            .count(|c| matches!(c, Command::AttentionDigest(_)))
            >= 1
    );
    assert!(context_text(&outcome).contains("Before using threads"));
    assert_no_page(&fixture, &outcome);
}

/// Kills: a tool-boundary check-in that appends an inbox page when the digest token advanced
/// for a hook-pull seat.
#[test]
fn hook_pull_tool_boundary_reads_no_inbox_page() {
    let fixture = fixture("hook-pull-tool");
    let started = run(&fixture, "SyntheticStart", "e1");
    assert_eq!(started.diagnostic, None);
    // A lazy row arrives after the lifecycle check-in seeded the mark: the token advances.
    fixture.service.lazy_pending.store(true, Ordering::SeqCst);
    // Only the tool boundary's own commands count below.
    fixture.service.commands.lock().unwrap().clear();
    let outcome = run(&fixture, "SyntheticTool", "e2");
    assert_eq!(outcome.diagnostic, None);
    assert_eq!(
        fixture.service.count(|c| matches!(c, Command::CheckIn(_))),
        1,
        "the tool boundary presented its check-in"
    );
    assert_no_page(&fixture, &outcome);
}
