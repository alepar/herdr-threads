//! The goal of ht-ma6i end to end (spec §4.8): a real daemon started by a private Herdr's plugin
//! startup entry, stand-in Codex agents running the hooks `setup codex` installed, and a real
//! `service_session_v3` connection. A keyed agent is started from the pane's own shell, as a
//! launcher outside threads does: threads' `launch` scrubs the variable from what it starts.
//!
//! Each test owns its Herdr root, so nothing here reaches the shared server. A `sweep` is `None`
//! when `herdr` is absent, as the neighbouring Herdr suites skip.
//!
//! The `qual_` tests are ignored: they are the opt-in real-Herdr qualification of the source
//! design's §6 (herdr-graph `docs/reviews/2026-10-29-seat-review-H2-identity-design.md`) that
//! `scripts/clone-identity-qual` runs per listed Herdr release with `--run-ignored all`: a pane
//! closed during downtime (G5), a cold restart with a client attached (Herdr resumes the agent,
//! C1 reattaches), and resurrection across a cold restart. A `qual_` test that cannot run its
//! case prints `skipped: <reason>` and, when `HT_CLONE_QUAL_SKIPS` names a directory, writes the
//! reason to `<dir>/<test>.skip` for the script's report.

#[path = "integration/herdr_release_smoke.rs"]
mod herdr_release_smoke;

use herdr_release_smoke::{Matrix, wait_for};
use herdr_threads::{
    app::SystemClock,
    client::service::{PersistentServiceClient, ServiceIntentJournal},
    protocol::{
        ids::{ContinuityKey, OperationId, SeatId},
        pagination::PageRequest,
        results::ContinuityConflictKind,
        service::{
            ContinuityAlert, FinalizeRetirement, ReserveContinuityKey,
            ServiceContinuityAlertsQuery, ServiceOperation, ServiceResult, UnretireSeat,
        },
        time::{CallBudget, Cancellation, Clock, MonoInstant},
    },
    test_support::spawn::{OwnedChild, SpawnOwned},
};
use rusqlite::{Connection, OpenFlags, OptionalExtension};
use serde_json::Value;
use std::{
    fs,
    path::PathBuf,
    process::{Output, Stdio},
    sync::Arc,
    thread,
    time::{Duration, Instant},
};
use uuid::Uuid;

const WAIT: Duration = Duration::from_secs(60);

/// The one registered service connection, over the daemon's published endpoint.
struct Service {
    runtime: tokio::runtime::Runtime,
    client: PersistentServiceClient,
    clock: Arc<dyn Clock>,
}

impl Service {
    fn connect(m: &Matrix) -> Self {
        let descriptor: Value = serde_json::from_slice(
            &fs::read(
                m.host
                    .root()
                    .join("st/herdr/plugins/herdr-threads/node/endpoint.json"),
            )
            .unwrap(),
        )
        .unwrap();
        let clock: Arc<dyn Clock> = Arc::new(SystemClock::new());
        let client = PersistentServiceClient::new(
            descriptor["endpoint"].as_str().unwrap().into(),
            Arc::clone(&clock),
            descriptor["node_uuid"].as_str().unwrap().parse().unwrap(),
            None,
            ServiceIntentJournal::open(m.host.root().join("service-intents")).unwrap(),
        );
        let service = Self {
            runtime: tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap(),
            client,
            clock,
        };
        service
            .runtime
            .block_on(service.client.register(&service.budget()))
            .unwrap();
        service
    }

    fn budget(&self) -> CallBudget {
        CallBudget {
            deadline: MonoInstant(self.clock.monotonic_now().0 + 10_000),
            cancellation: Cancellation::default(),
        }
    }

    fn submit(&self, operation: ServiceOperation) -> ServiceResult {
        let budget = self.budget();
        self.runtime
            .block_on(self.client.submit(operation, &budget))
            .unwrap()
            .1
    }

    fn reserve(&self, key: &str, seat: Option<&str>) -> Option<SeatId> {
        let result = self.submit(ServiceOperation::ReserveContinuityKey(
            ReserveContinuityKey {
                key: ContinuityKey::new(key),
                seat: seat.map(SeatId::new),
                operation: operation(),
            },
        ));
        let ServiceResult::ContinuityKeyReserved(reservation) = result else {
            panic!("reserve answered {result:?}");
        };
        reservation.seat
    }

    fn unretire(&self, seat: &str) {
        let result = self.submit(ServiceOperation::UnretireSeat(UnretireSeat {
            seat: SeatId::new(seat),
            operation: operation(),
        }));
        let ServiceResult::SeatUnretired(unretired) = result else {
            panic!("unretire answered {result:?}");
        };
        assert_eq!(unretired.seat, SeatId::new(seat));
    }

    fn finalize(&self, seat: &str) {
        let result = self.submit(ServiceOperation::FinalizeRetirement(FinalizeRetirement {
            seat: SeatId::new(seat),
            operation: operation(),
        }));
        assert_eq!(
            result,
            ServiceResult::RetirementFinalized(SeatId::new(seat))
        );
    }

    fn alerts(&self) -> Vec<ContinuityAlert> {
        let budget = self.budget();
        let result = self
            .runtime
            .block_on(self.client.query(
                ServiceOperation::ContinuityAlerts(ServiceContinuityAlertsQuery {
                    page: PageRequest::default(),
                }),
                &budget,
            ))
            .unwrap();
        let ServiceResult::ContinuityAlerts(page) = result else {
            panic!("alerts answered {result:?}");
        };
        page.items
    }
}

fn operation() -> OperationId {
    OperationId::new(Uuid::new_v4().to_string())
}

/// The service is declared first, so its session ends before the daemon stops.
struct Sweep {
    service: Service,
    m: Matrix,
}

fn sweep(case: &str) -> Option<Sweep> {
    sweep_with_daemon_env(case, &[])
}

/// A sweep whose daemon runs with `env` on top of what Herdr's startup entry gives it: the
/// startup daemon is replaced by one `daemon ensure` starts with `env`, before the service
/// registers. The daemon outlives a Herdr restart (its own session, ht-mp72), so the variables
/// hold across `cold_restart`.
fn sweep_with_daemon_env(case: &str, env: &[(&str, &str)]) -> Option<Sweep> {
    if std::process::Command::new("herdr")
        .arg("--version")
        .output()
        .is_err()
    {
        println!("skipped: no herdr: {case}");
        return None;
    }
    let m = Matrix::new(case);
    m.setup("codex");
    if !env.is_empty() {
        restart_daemon_with(&m, env);
    }
    Some(Sweep {
        service: Service::connect(&m),
        m,
    })
}

fn restart_daemon_with(m: &Matrix, env: &[(&str, &str)]) {
    let stopped = m.ht_out(&["daemon", "stop"]);
    assert!(
        stopped.status.success(),
        "daemon stop: {}",
        String::from_utf8_lossy(&stopped.stderr)
    );
    // As the startup entry's environment has it: the Herdr binary the daemon drives.
    let herdr = m
        .host
        .command("sh")
        .args(["-c", "command -v herdr"])
        .output()
        .unwrap();
    let herdr = String::from_utf8_lossy(&herdr.stdout).trim().to_owned();
    assert!(!herdr.is_empty(), "no herdr on PATH");
    let mut cmd = m.cli();
    m.globals(&mut cmd);
    cmd.env("HERDR_BIN_PATH", &herdr)
        .envs(env.iter().copied())
        .args(["daemon", "ensure"]);
    let ensured = cmd.output().unwrap();
    assert!(
        ensured.status.success(),
        "daemon ensure: {}",
        String::from_utf8_lossy(&ensured.stderr)
    );
    wait_for("daemon host reachability", WAIT, || {
        let health = m.ht(&["daemon", "health"]);
        let data = &health["result"]["data"];
        if data["host"]["reachability"] == "ready" && !data["last_reconciliation_at"].is_null() {
            Ok(())
        } else {
            Err(health.to_string())
        }
    });
}

fn data(value: &Value) -> &Value {
    &value["result"]["data"]
}

/// `inspection` reads retired with `retirement_state` `state`; `None` means the field is absent
/// (finally retired).
fn is_retired(inspection: &Value, state: Option<&str>) -> bool {
    inspection["summary"]["continuity"] == "retired"
        && match state {
            Some(state) => inspection["retirement_state"] == state,
            None => inspection.get("retirement_state").is_none(),
        }
}

fn text(value: &Value) -> String {
    value
        .as_str()
        .unwrap_or_else(|| panic!("no string in {value}"))
        .to_owned()
}

impl Sweep {
    fn db(&self) -> Connection {
        Connection::open_with_flags(self.db_path(), OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap()
    }

    fn db_path(&self) -> std::path::PathBuf {
        self.m
            .host
            .root()
            .join("st/herdr/plugins/herdr-threads/node/herdr-threads.db")
    }

    fn count(&self, sql: &str, arg: &str) -> i64 {
        self.db().query_row(sql, [arg], |r| r.get(0)).unwrap()
    }

    fn seats(&self) -> i64 {
        self.db()
            .query_row("SELECT count(*) FROM seats", [], |r| r.get(0))
            .unwrap()
    }

    fn open_bindings(&self, seat: &str) -> i64 {
        self.count(
            "SELECT count(*) FROM occupant_bindings WHERE seat_id=?1 AND ended_at IS NULL",
            seat,
        )
    }

    /// Whether `session` holds an open binding on `seat`.
    fn open_binding_of(&self, seat: &str, session: &str) -> bool {
        self.db()
            .query_row(
                "SELECT count(*) FROM occupant_bindings WHERE seat_id=?1 AND native_session=?2 AND ended_at IS NULL",
                [seat, session],
                |r| r.get::<_, i64>(0),
            )
            .unwrap()
            != 0
    }

    /// The host's restore hold, which a restart raises while unresolved seats remain.
    fn restore_held(&self) -> bool {
        self.db()
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM herdr_hosts WHERE baseline_hold_unclaimed=1)",
                [],
                |r| r.get(0),
            )
            .unwrap()
    }

    fn reversible_rows(&self, seat: &str) -> i64 {
        self.count(
            "SELECT count(*) FROM reversible_retirements WHERE seat_id=?1",
            seat,
        )
    }

    fn decisions(&self, seat: &str) -> Vec<String> {
        self.db()
            .prepare("SELECT kind FROM allocation_decisions WHERE seat_id=?1 ORDER BY ordinal")
            .unwrap()
            .query_map([seat], |r| r.get(0))
            .unwrap()
            .map(Result::unwrap)
            .collect()
    }

    fn key_seat(&self, key: &str) -> Option<String> {
        self.db()
            .query_row(
                "SELECT seat_id FROM continuity_keys WHERE key=?1",
                [key],
                |r| r.get(0),
            )
            .optional()
            .unwrap()
            .flatten()
    }

    fn membership(&self, thread: &str, seat: &str) -> String {
        self.db()
            .query_row(
                "SELECT state FROM memberships WHERE thread_id=?1 AND seat_id=?2",
                [thread, seat],
                |r| r.get(0),
            )
            .unwrap()
    }

    fn inspect(&self, seat: &str) -> Value {
        data(&self.m.ht(&["seat", "inspect", seat])).clone()
    }

    fn wait_continuity(&self, seat: &str, continuity: &str) {
        wait_for(&format!("{seat} reads {continuity}"), WAIT, || {
            let inspection = self.inspect(seat);
            if inspection["summary"]["continuity"] == continuity {
                Ok(())
            } else {
                Err(inspection.to_string())
            }
        });
    }

    /// Retired with `retirement_state` `state`; `None` means finally retired (the field absent).
    fn wait_retirement(&self, seat: &str, state: Option<&str>) {
        self.wait_retirement_for(seat, state, WAIT);
    }

    fn wait_retirement_for(&self, seat: &str, state: Option<&str>, timeout: Duration) {
        wait_for(&format!("{seat} retired {state:?}"), timeout, || {
            let inspection = self.inspect(seat);
            if is_retired(&inspection, state) {
                Ok(())
            } else {
                Err(inspection.to_string())
            }
        });
    }

    /// Reattached by a key claim at `pane`: resolved there, binding open, and the decision recorded.
    fn wait_reattached(&self, seat: &str, pane: &str) {
        wait_for(&format!("{seat} reattached at {pane}"), WAIT, || {
            let inspection = self.inspect(seat);
            let decided = self.decisions(seat);
            if inspection["summary"]["continuity"] == "resolved"
                && inspection["summary"]["target"] == pane
                && decided.last().map(String::as_str) == Some("cooperative_continuity_key")
                && self.open_bindings(seat) >= 1
            {
                Ok(())
            } else {
                Err(format!("{inspection} decisions: {decided:?}"))
            }
        });
    }

    /// A stand-in Codex started from the pane's own shell with the key in its environment; returns
    /// once its SessionStart hook ran and it reads input. Every hook of a start must succeed.
    fn start_agent(&self, pane: &str, command: &str) -> String {
        let previous = self.m.log_of(pane).first().map(|e| e["session"].clone());
        self.m.submit(pane, command);
        let codex = wait_for(&format!("{command} in {pane}"), WAIT, || {
            let log = self.m.log_of(pane);
            let fresh = log.first().map(|e| e["session"].clone()) != previous;
            if fresh && log.iter().any(|e| e["kind"] == "ready") {
                Ok(log)
            } else {
                Err(format!("{log:?}"))
            }
        });
        for event in codex.iter().filter(|e| e["kind"] == "hook") {
            assert_eq!(event["rc"], 0, "a hook failed: {event}");
        }
        text(&codex[0]["session"])
    }

    fn codex(&self) -> String {
        self.m.bin.join("codex").display().to_string()
    }

    fn keyed_agent(&self, pane: &str, key: &str) -> String {
        self.start_agent(
            pane,
            &format!("HERDR_THREADS_CONTINUITY_KEY={key} {}", self.codex()),
        )
    }

    /// A reserved key's agent in a fresh pane: enrolled, and the key bound at its check-in.
    fn keyed_seat(&self, pane: &str, key: &str) -> String {
        assert_eq!(self.service.reserve(key, None), None);
        self.keyed_agent(pane, key);
        let seat = wait_for(&format!("{key} bound"), WAIT, || {
            self.key_seat(key).ok_or_else(|| "unbound".to_owned())
        });
        self.wait_continuity(&seat, "resolved");
        let inspection = self.inspect(&seat);
        assert_eq!(inspection["summary"]["target"], pane, "{inspection}");
        assert_eq!(inspection["continuity_key"]["key"], key, "{inspection}");
        assert_eq!(
            inspection["continuity_key"]["bound_by"], "check_in",
            "{inspection}"
        );
        seat
    }

    fn close_pane(&self, pane: &str) {
        self.m.herdr_input(&["pane", "close", pane]);
    }

    /// A thread the agent in `pane` was invited to and accepted.
    fn joined_thread(&self, sender: &str, pane: &str, seat: &str) -> String {
        let thread = text(data(
            &self
                .m
                .person(sender, &["thread", "create", "--topic", "clone identity"]),
        ));
        self.m.person(sender, &["invite", &thread, "--seat", seat]);
        self.m.agent_ht(pane, &format!("accept {thread}"));
        assert_eq!(self.membership(&thread, seat), "joined");
        thread
    }

    /// An ACK request the agent has not seen: it is inside a tool call, so no hook displays it and
    /// no wake is typed (delivery contract rule 4). The call's process goes with the pane.
    fn pending_ack(&self, sender: &str, thread: &str, pane: &str, seat: &str) -> String {
        let hooks = |log: &[Value]| {
            log.iter()
                .filter(|e| e["kind"] == "hook" && e["event"] == "PreToolUse")
                .count()
        };
        let before = hooks(&self.m.log_of(pane));
        self.m.start_turn(pane, "run: sleep 45");
        wait_for("the agent is inside a tool call", WAIT, || {
            let log = self.m.log_of(pane);
            if hooks(&log) > before {
                Ok(())
            } else {
                Err(format!("{log:?}"))
            }
        });
        let message = text(data(&self.m.person(
            sender,
            &[
                "send",
                thread,
                "--body",
                "clone-identity-ack",
                "--require-ack",
                seat,
                "--deadline",
                "3600",
            ],
        )));
        let recipient = self.recipient(sender, &message);
        assert_eq!(recipient["status"], "pending", "{recipient}");
        message
    }

    fn recipient(&self, sender: &str, message: &str) -> Value {
        let recipients = self.m.person(sender, &["delivery", "recipients", message]);
        let items = data(&recipients)["items"].as_array().unwrap();
        assert_eq!(items.len(), 1, "{recipients}");
        items[0].clone()
    }

    fn person_raw(&self, pane: &str, args: &[&str]) -> Output {
        let mut cmd = self.m.cli();
        cmd.env("HERDR_PANE_ID", pane)
            .env("HERDR_ENV", "1")
            .arg("human");
        self.m.globals(&mut cmd);
        cmd.args(args).output().unwrap()
    }

    /// Every alert of `key`, oldest first.
    fn alerts_of(&self, key: &str) -> Vec<ContinuityAlert> {
        self.service
            .alerts()
            .into_iter()
            .filter(|alert| alert.key.as_str() == key)
            .collect()
    }
}

fn deadline(recipient: &Value) -> i64 {
    recipient["deadline"]
        .as_i64()
        .unwrap_or_else(|| panic!("no deadline in {recipient}"))
}

/// Kills: a close that retires finally, a send that reaches a frozen seat, an un-retire that does
/// not shift the deadline, a relaunch that mints a seat or reopens membership.
#[test]
fn reserve_launch_close_unretire_relaunch() {
    let Some(s) = sweep("sweep-relaunch") else {
        return;
    };
    let (sender, _) = s.m.person_seat("relaunch-sender");
    let pane = s.m.workspace("relaunch-agent");
    let seat = s.keyed_seat(&pane, "clone-1");
    let thread = s.joined_thread(&sender, &pane, &seat);
    let invitations = s.count("SELECT count(*) FROM invitations WHERE seat_id=?1", &seat);
    let message = s.pending_ack(&sender, &thread, &pane, &seat);
    let before = deadline(&s.recipient(&sender, &message));
    let seats = s.seats();

    s.close_pane(&pane);
    s.wait_retirement(&seat, Some("reversible"));
    assert_eq!(
        s.count("SELECT count(*) FROM retirements WHERE seat_id=?1", &seat),
        0
    );

    let refused = s.person_raw(&sender, &["send", "--seat", &seat, "--body", "too late"]);
    assert!(
        !refused.status.success(),
        "a direct send reached a retired seat"
    );
    let refused = s.person_raw(
        &sender,
        &[
            "send",
            &thread,
            "--body",
            "too late",
            "--require-ack",
            &seat,
        ],
    );
    assert!(
        !refused.status.success(),
        "an ACK request reached a retired seat"
    );
    let stderr = String::from_utf8_lossy(&refused.stderr);
    assert!(stderr.contains("retired"), "{stderr}");
    let frozen = s.recipient(&sender, &message);
    assert_eq!(frozen["status"], "retired", "{frozen}");
    assert_eq!(frozen["physical_status"], "pending", "{frozen}");
    assert_eq!(s.membership(&thread, &seat), "joined");

    s.service.unretire(&seat);
    let inspection = s.inspect(&seat);
    assert_eq!(
        inspection["summary"]["continuity"], "unresolved",
        "{inspection}"
    );
    assert!(inspection["summary"]["target"].is_null(), "{inspection}");
    assert!(inspection["retirement_state"].is_null(), "{inspection}");

    // The remaining time is what it was at the close.
    let after = deadline(&s.recipient(&sender, &message));
    let (cutover, unretired): (i64, i64) = s
        .db()
        .query_row(
            "SELECT cutover_at,unretired_at FROM reversible_retirements WHERE seat_id=?1",
            [&seat],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert!(unretired > cutover);
    assert_eq!(after - before, unretired - cutover);

    let relaunched = s.m.tab(&sender, "relaunch-new");
    s.keyed_agent(&relaunched, "clone-1");
    s.wait_reattached(&seat, &relaunched);
    assert_eq!(s.seats(), seats, "a relaunch minted a seat");
    assert_eq!(s.reversible_rows(&seat), 0);
    assert_eq!(s.membership(&thread, &seat), "joined");
    assert_eq!(
        s.count("SELECT count(*) FROM invitations WHERE seat_id=?1", &seat),
        invitations,
        "a re-invitation"
    );
    // No re-acceptance: the seat acts on the thread, and the frozen request reaches it.
    s.m.agent_ht(&relaunched, &format!("send {thread} --body back"));
    // The check-in names the frozen request; displaying it settles it.
    let log = s.m.log_of(&relaunched);
    assert!(
        log.iter()
            .any(|e| e["kind"] == "hook"
                && e["context"].as_str().is_some_and(|c| c.contains(&message))),
        "the check-in does not name the frozen request: {log:?}"
    );
    let inbox = s.m.agent_run(&relaunched, "herdr-threads inbox");
    assert!(
        inbox["stdout"]
            .as_str()
            .is_some_and(|out| out.contains("clone-identity-ack")),
        "{inbox}"
    );
    wait_for("the frozen request is acknowledged", WAIT, || {
        let recipient = s.recipient(&sender, &message);
        if recipient["status"] == "acknowledged" {
            Ok(())
        } else {
            Err(recipient.to_string())
        }
    });
}

/// Kills: a second seat for a live key, a failing hook, a repeated alert, a conflict not surfaced.
#[test]
fn second_agent_with_a_live_key_is_held() {
    let Some(s) = sweep("sweep-live-key") else {
        return;
    };
    let first = s.m.workspace("live-first");
    let seat = s.keyed_seat(&first, "clone-1");
    let seats = s.seats();

    let second = s.m.workspace("live-second");
    s.keyed_agent(&second, "clone-1");
    let conflict = wait_for("the conflict on SeatInspect", WAIT, || {
        let inspection = s.inspect(&seat);
        let conflict = &inspection["continuity_key"]["conflict"];
        if conflict.is_null() {
            Err(inspection.to_string())
        } else {
            Ok(conflict.clone())
        }
    });
    assert_eq!(conflict["pane"], second.as_str(), "{conflict}");
    assert_eq!(conflict["kind"], "key_live_elsewhere", "{conflict}");

    // The next event presents the key again: same refusal, nothing new.
    let events = s.m.turn(&second, "next");
    for event in events.iter().filter(|e| e["kind"] == "hook") {
        assert_eq!(event["rc"], 0, "a hook failed: {event}");
    }
    assert_eq!(s.seats(), seats, "the second agent got a seat");
    let alerts = s.alerts_of("clone-1");
    assert_eq!(alerts.len(), 1, "{alerts:?}");
    assert_eq!(alerts[0].kind, ContinuityConflictKind::KeyLiveElsewhere);
    assert_eq!(alerts[0].seat, Some(SeatId::new(&seat)));
    assert_eq!(alerts[0].pane.as_str(), second);
    assert_eq!(s.inspect(&seat)["summary"]["target"], first.as_str());
    // The hold stands against keyless resolution of that pane.
    let held = s.m.ht_out(&["seat", "resolve", "--pane", &second]);
    assert!(!held.status.success(), "the held pane got a seat");
    assert_eq!(s.seats(), seats, "resolving the held pane minted a seat");
}

/// Kills: shell continuity missing for a bare or exited-agent pane, and a live agent's seat decided
/// by the seat-level path, which opens no binding.
#[test]
fn live_handoff_with_mixed_panes() {
    let Some(s) = sweep("sweep-handoff") else {
        return;
    };
    let bare_pane = s.m.workspace("mixed-bare");
    let bare = text(data(&s.m.ht(&["seat", "resolve", "--pane", &bare_pane])));
    let exited_pane = s.m.workspace("mixed-exited");
    let exited = s.keyed_seat(&exited_pane, "clone-exited");
    s.m.submit(&exited_pane, "exit");
    wait_for("the stand-in exited", WAIT, || {
        let log = s.m.log_of(&exited_pane);
        if log.iter().any(|e| e["kind"] == "exit") {
            Ok(())
        } else {
            Err(format!("{log:?}"))
        }
    });
    let live_pane = s.m.workspace("mixed-live");
    let live = s.keyed_seat(&live_pane, "clone-live");
    let seats = s.seats();
    for seat in [&bare, &exited, &live] {
        wait_for("the seat's shell fingerprint", WAIT, || {
            match s.count(
                "SELECT count(*) FROM seat_shell_fingerprints WHERE seat_id=?1",
                seat,
            ) {
                1 => Ok(()),
                rows => Err(format!("{seat}: {rows} rows")),
            }
        });
    }
    let hooks =
        s.m.log_of(&live_pane)
            .iter()
            .filter(|e| e["kind"] == "hook")
            .count();

    let handoff =
        s.m.host
            .command("herdr")
            .args(["server", "live-handoff"])
            .output()
            .unwrap();
    assert!(
        handoff.status.success(),
        "live-handoff: {}",
        String::from_utf8_lossy(&handoff.stderr)
    );

    for (seat, pane, kind) in [
        (&bare, &bare_pane, "structural_shell_continuity"),
        (&exited, &exited_pane, "structural_shell_continuity"),
        (&live, &live_pane, "structural_process_continuity"),
    ] {
        wait_for(&format!("{seat} decided {kind}"), WAIT, || {
            let inspection = s.inspect(seat);
            let decided = s.decisions(seat);
            if inspection["summary"]["continuity"] == "resolved"
                && inspection["summary"]["target"] == pane.as_str()
                && decided.last().map(String::as_str) == Some(kind)
            {
                Ok(())
            } else {
                Err(format!("{inspection} decisions: {decided:?}"))
            }
        });
    }
    assert_eq!(
        s.open_bindings(&bare),
        0,
        "shell continuity opened a binding"
    );
    assert_eq!(
        s.open_bindings(&exited),
        0,
        "shell continuity opened a binding"
    );
    assert_eq!(s.open_bindings(&live), 1, "the live agent has no binding");
    assert_eq!(
        s.m.log_of(&live_pane)
            .iter()
            .filter(|e| e["kind"] == "hook")
            .count(),
        hooks,
        "the reattachment needed a hook of the agent's own"
    );
    assert_eq!(s.seats(), seats);
}

/// Kills: a finalize that leaves obligations frozen, or one that keeps the key.
#[test]
fn finalize_settles_and_frees_the_key() {
    let Some(s) = sweep("sweep-finalize") else {
        return;
    };
    let (sender, _) = s.m.person_seat("finalize-sender");
    let pane = s.m.workspace("finalize-agent");
    let seat = s.keyed_seat(&pane, "clone-1");
    let thread = s.joined_thread(&sender, &pane, &seat);
    let message = s.pending_ack(&sender, &thread, &pane, &seat);
    s.close_pane(&pane);
    s.wait_retirement(&seat, Some("reversible"));
    assert_eq!(s.recipient(&sender, &message)["physical_status"], "pending");

    s.service.finalize(&seat);
    s.wait_retirement(&seat, None);
    wait_for("the job settled the obligations", WAIT, || {
        let recipient = s.recipient(&sender, &message);
        let membership = s.membership(&thread, &seat);
        if recipient["physical_status"] != "pending" && membership != "joined" {
            Ok(())
        } else {
            Err(format!("{recipient} membership {membership}"))
        }
    });
    assert_eq!(s.reversible_rows(&seat), 0);
    assert_eq!(s.key_seat("clone-1"), None);
    // The row is gone and any service can take the key anew, unbound.
    assert_eq!(
        s.count(
            "SELECT count(*) FROM continuity_keys WHERE key=?1",
            "clone-1"
        ),
        0
    );
    assert_eq!(s.service.reserve("clone-1", None), None);
    assert_eq!(s.key_seat("clone-1"), None);
}

/// A cold restart: the old processes are gone, the pane comes back with a new shell.
fn cold_restart(s: &Sweep, seat: &str) {
    s.m.host.stop();
    s.m.host.start_with_path_prefix(&s.m.bin);
    s.wait_continuity(seat, "unresolved");
}

/// Kills: a fresh agent in a restored pane that gets a new seat, or a hold that stays.
#[test]
fn cold_restart_reattaches_keyed_fresh_agent() {
    let Some(s) = sweep("sweep-cold") else {
        return;
    };
    let pane = s.m.workspace("cold-agent");
    let seat = s.keyed_seat(&pane, "clone-1");
    let seats = s.seats();
    cold_restart(&s, &seat);
    assert!(s.restore_held(), "the restored host is not held");

    s.keyed_agent(&pane, "clone-1");
    s.wait_reattached(&seat, &pane);
    assert_eq!(s.seats(), seats, "the fresh agent got a seat");
    assert!(!s.restore_held(), "the hold stands");
}

/// Holds a cold restart's first reconciliation pass open: the pass fails on its first unresolve and
/// neither moves its marker nor changes the seat, so the lag a restart opens stays open for the test
/// with the key's seat still reading resolved at the restored pane.
const HOLD_PASS: &str = "CREATE TRIGGER sweep_hold_pass BEFORE UPDATE OF state ON seats \
    WHEN NEW.state='unresolved' \
    BEGIN SELECT RAISE(ABORT,'sweep holds the pass'); END";

/// Kills: a claim during lag recording a conflict, minting a seat, or waiting for a later event.
#[test]
fn cold_restart_keyed_claim_during_lag_needs_no_later_event() {
    let Some(s) = sweep("sweep-cold-lag") else {
        return;
    };
    let pane = s.m.workspace("lag-agent");
    let seat = s.keyed_seat(&pane, "clone-1");
    let seats = s.seats();
    let writer = Connection::open(s.db_path()).unwrap();
    writer.busy_timeout(Duration::from_secs(10)).unwrap();
    writer.execute_batch(HOLD_PASS).unwrap();
    s.m.host.stop();
    s.m.host.start_with_path_prefix(&s.m.bin);
    wait_for("the new boot's lag", WAIT, || {
        let lag: bool = s
            .db()
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM herdr_hosts WHERE recovery_boot IS NOT reconciled_boot OR recovery_epoch IS NOT reconciled_epoch)",
                [],
                |r| r.get(0),
            )
            .unwrap();
        if lag {
            Ok(())
        } else {
            Err("no lag".to_owned())
        }
    });

    s.keyed_agent(&pane, "clone-1");
    wait_for("the claim is kept with its key", WAIT, || {
        match s.count(
            "SELECT count(*) FROM kept_continuity_claims WHERE continuity_key=?1",
            "clone-1",
        ) {
            0 => Err(format!("{:?}", s.m.log_of(&pane))),
            _ => Ok(()),
        }
    });
    assert!(s.alerts_of("clone-1").is_empty(), "lag recorded a conflict");
    assert_eq!(s.inspect(&seat)["summary"]["continuity"], "resolved");
    assert_eq!(s.seats(), seats);
    let hooks =
        s.m.log_of(&pane)
            .iter()
            .filter(|e| e["kind"] == "hook")
            .count();

    // The lag ends; the agent stays idle and the observation worker decides what it kept.
    writer
        .execute_batch("DROP TRIGGER sweep_hold_pass")
        .unwrap();
    s.wait_reattached(&seat, &pane);
    assert_eq!(s.seats(), seats, "the fresh agent got a seat");
    assert!(!s.restore_held(), "the hold stands");
    assert!(s.alerts_of("clone-1").is_empty(), "lag recorded a conflict");
    assert_eq!(
        s.m.log_of(&pane)
            .iter()
            .filter(|e| e["kind"] == "hook")
            .count(),
        hooks,
        "the decision needed a later event"
    );
}

/// Kills: reserve-with-seat that does not make the seat's close reversible, or that moves it.
#[test]
fn reserve_with_seat_then_close_unretire_reattach() {
    let Some(s) = sweep("sweep-reserve-seat") else {
        return;
    };
    let (sender, _) = s.m.person_seat("reserve-sender");
    let pane = s.m.workspace("reserve-agent");
    // A clone that exists from before: it started with no key.
    s.start_agent(&pane, &s.codex());
    let seat = wait_for("the keyless seat", WAIT, || {
        let resolved = s.m.ht(&["seat", "resolve", "--pane", &pane]);
        Ok::<_, String>(text(data(&resolved)))
    });
    assert_eq!(s.open_bindings(&seat), 1);
    assert_eq!(
        s.service.reserve("clone-1", Some(&seat)),
        Some(SeatId::new(&seat))
    );
    let inspection = s.inspect(&seat);
    assert_eq!(
        inspection["summary"]["target"],
        pane.as_str(),
        "{inspection}"
    );
    assert_eq!(
        inspection["continuity_key"]["bound_by"], "service_reservation",
        "{inspection}"
    );
    let seats = s.seats();

    s.close_pane(&pane);
    s.wait_retirement(&seat, Some("reversible"));
    s.service.unretire(&seat);
    let relaunched = s.m.tab(&sender, "reserve-new");
    s.keyed_agent(&relaunched, "clone-1");
    s.wait_reattached(&seat, &relaunched);
    assert_eq!(s.seats(), seats, "a relaunch minted a seat");
}

/// Kills: a keyed relaunch before un-retire that gets a seat, repeats the alert, or stays held
/// after the un-retire.
#[test]
fn keyed_relaunch_before_unretire_is_held_then_reattaches() {
    let Some(s) = sweep("sweep-early-relaunch") else {
        return;
    };
    let (sender, _) = s.m.person_seat("early-sender");
    let pane = s.m.workspace("early-agent");
    let seat = s.keyed_seat(&pane, "clone-1");
    let seats = s.seats();
    s.close_pane(&pane);
    s.wait_retirement(&seat, Some("reversible"));

    let relaunched = s.m.tab(&sender, "early-new");
    s.keyed_agent(&relaunched, "clone-1");
    wait_for("the alert", WAIT, || {
        let alerts = s.alerts_of("clone-1");
        if alerts.is_empty() {
            Err(format!("{:?}", s.m.log_of(&relaunched)))
        } else {
            Ok(())
        }
    });
    s.m.turn(&relaunched, "still held");
    let alerts = s.alerts_of("clone-1");
    assert_eq!(alerts.len(), 1, "{alerts:?}");
    assert_eq!(alerts[0].kind, ContinuityConflictKind::KeyRetiredReversibly);
    assert_eq!(alerts[0].pane.as_str(), relaunched);
    assert_eq!(s.seats(), seats, "the early relaunch got a seat");
    assert_eq!(s.inspect(&seat)["retirement_state"], "reversible");

    s.service.unretire(&seat);
    s.m.turn(&relaunched, "next event");
    s.wait_reattached(&seat, &relaunched);
    assert_eq!(s.seats(), seats, "the relaunch minted a seat");
    assert_eq!(s.alerts_of("clone-1").len(), 1, "a second alert");
    assert!(
        s.inspect(&seat)["continuity_key"]["conflict"].is_null(),
        "the conflict stands after the claim"
    );
}

/// Kills: an agent exit treated as a pane close (B3), for the seat or its memberships; a relaunch
/// in that pane, keyed or not, that gets another seat.
#[test]
fn agent_exit_with_pane_open_keeps_the_seat() {
    let Some(s) = sweep("sweep-exit") else {
        return;
    };
    let (sender, _) = s.m.person_seat("exit-sender");
    let pane = s.m.workspace("exit-agent");
    let seat = s.keyed_seat(&pane, "clone-1");
    let thread = s.joined_thread(&sender, &pane, &seat);
    let seats = s.seats();

    let exit = |s: &Sweep| {
        s.m.submit(&pane, "exit");
        wait_for("the stand-in exited", WAIT, || {
            let log = s.m.log_of(&pane);
            if log.iter().any(|e| e["kind"] == "exit") {
                Ok(())
            } else {
                Err(format!("{log:?}"))
            }
        });
        // More than one observation pass (OBSERVATION_CADENCE) with the pane at its shell.
        thread::sleep(Duration::from_secs(11));
        let inspection = s.inspect(&seat);
        assert_eq!(
            inspection["summary"]["continuity"], "resolved",
            "{inspection}"
        );
        assert_eq!(
            inspection["summary"]["target"],
            pane.as_str(),
            "{inspection}"
        );
        assert!(inspection["retirement_state"].is_null(), "{inspection}");
        assert_eq!(s.reversible_rows(&seat), 0);
        assert_eq!(s.membership(&thread, &seat), "joined");
    };
    exit(&s);
    let keyed = s.keyed_agent(&pane, "clone-1");
    wait_for("the keyed relaunch's binding", WAIT, || {
        if s.open_binding_of(&seat, &keyed) {
            Ok(())
        } else {
            Err(format!("{:?}", s.m.log_of(&pane)))
        }
    });
    assert_eq!(s.seats(), seats, "the keyed relaunch got another seat");

    exit(&s);
    let keyless = s.start_agent(&pane, &s.codex());
    wait_for("the keyless relaunch's binding", WAIT, || {
        if s.open_binding_of(&seat, &keyless) {
            Ok(())
        } else {
            Err(format!("{:?}", s.m.log_of(&pane)))
        }
    });
    assert_eq!(s.seats(), seats, "the keyless relaunch got another seat");
    assert_eq!(s.key_seat("clone-1").as_deref(), Some(seat.as_str()));
}

/// Kills: the old session's claim reviving the finally retired seat or minting a third one, and a
/// key that cannot pass to a successor.
#[test]
fn seat_succession() {
    let Some(s) = sweep("sweep-succession") else {
        return;
    };
    let (sender, _) = s.m.person_seat("succession-sender");
    let pane = s.m.workspace("succession-old");
    let old = s.keyed_seat(&pane, "clone-1");
    let session = text(&s.m.log_of(&pane)[0]["session"]);
    s.close_pane(&pane);
    s.wait_retirement(&old, Some("reversible"));
    s.service.finalize(&old);
    s.wait_retirement(&old, None);

    let successor_pane = s.m.tab(&sender, "succession-new");
    let successor = text(data(&s.m.ht(&[
        "seat",
        "resolve",
        "--pane",
        &successor_pane,
    ])));
    assert_ne!(successor, old);
    assert_eq!(
        s.service.reserve("clone-1", Some(&successor)),
        Some(SeatId::new(&successor))
    );
    let seats = s.seats();

    s.start_agent(
        &successor_pane,
        &format!(
            "HERDR_THREADS_CONTINUITY_KEY=clone-1 {} resume {session}",
            s.codex()
        ),
    );
    wait_for(
        "the resumed session checked in on the successor",
        WAIT,
        || match s.open_bindings(&successor) {
            0 => Err(format!(
                "{:?} {:?}",
                s.inspect(&successor),
                s.m.log_of(&successor_pane)
            )),
            _ => Ok(()),
        },
    );
    let inspection = s.inspect(&successor);
    assert_eq!(
        inspection["summary"]["continuity"], "resolved",
        "{inspection}"
    );
    assert_eq!(inspection["summary"]["target"], successor_pane.as_str());
    assert!(
        inspection["history"]["items"]
            .as_array()
            .unwrap()
            .iter()
            .any(|item| item["data"]["native_session"] == session.as_str()
                && item["data"]["ended_at"].is_null()),
        "the resumed session has no open binding here: {inspection}"
    );
    assert!(is_retired(&s.inspect(&old), None), "the old seat revived");
    assert_eq!(s.open_bindings(&old), 0, "the old seat revived");
    assert_eq!(s.seats(), seats, "the resumed session got a seat");
    assert!(
        s.alerts_of("clone-1").is_empty(),
        "{:?}",
        s.alerts_of("clone-1")
    );
}

// ---- Opt-in real-Herdr qualification (scripts/clone-identity-qual) ----

/// `RESTORED_PANE_GRACE_ENV` in `src/store/seats/observation.rs` (test-support builds only; the
/// module is private, so the name is spelled out as `tests/integration/trust_policy.rs` does).
const RESTORED_PANE_GRACE_ENV: &str = "HT_TEST_RESTORED_PANE_GRACE_MS";
/// A short G5 window: one or two 5 s observation passes after the new boot is reconciled.
const QUAL_GRACE_MS: &str = "3000";
/// Restart, reconciliation and the absence window together.
const QUAL_WAIT: Duration = Duration::from_secs(180);

/// A qualification case that cannot run here: printed, and recorded for the script's report.
fn skip_qual(test: &str, reason: &str) {
    println!("skipped: {test}: {reason}");
    if let Some(dir) = std::env::var_os("HT_CLONE_QUAL_SKIPS") {
        let dir = PathBuf::from(dir);
        let _ = fs::create_dir_all(&dir);
        let _ = fs::write(dir.join(format!("{test}.skip")), reason);
    }
}

/// Herdr's saved session (`session.json` in its data dir, `$XDG_CONFIG_HOME/herdr` for the
/// default session), written by the graceful `server stop` that `IsolatedHerdr::stop` performs.
fn saved_session_path(m: &Matrix) -> PathBuf {
    m.host.root().join("cfg/herdr/session.json")
}

fn saved_session(m: &Matrix) -> Value {
    let path = saved_session_path(m);
    let bytes = fs::read(&path)
        .unwrap_or_else(|e| panic!("Herdr saved no session at {}: {e}", path.display()));
    serde_json::from_slice(&bytes).unwrap()
}

/// The index of the saved workspace created with `--label label` (`custom_name`).
fn saved_workspace(snapshot: &Value, label: &str) -> usize {
    let workspaces = snapshot["workspaces"]
        .as_array()
        .unwrap_or_else(|| panic!("no workspaces in the saved session: {snapshot}"));
    workspaces
        .iter()
        .position(|w| w["custom_name"] == label)
        .unwrap_or_else(|| {
            let names: Vec<&Value> = workspaces.iter().map(|w| &w["custom_name"]).collect();
            panic!("no saved workspace {label:?}; saved: {names:?}")
        })
}

/// Every saved pane of the workspace labelled `label`, over all its tabs.
fn saved_panes(m: &Matrix, label: &str) -> Vec<Value> {
    let snapshot = saved_session(m);
    let workspace = &snapshot["workspaces"][saved_workspace(&snapshot, label)];
    workspace["tabs"]
        .as_array()
        .into_iter()
        .flatten()
        .flat_map(|tab| tab["panes"].as_object().into_iter().flatten())
        .map(|(_, pane)| pane.clone())
        .collect()
}

/// Removes the workspace labelled `label` (every tab and pane of it) from what the next start
/// restores: the panes were closed while Herdr was down. The active and selected indices are kept
/// pointing at a surviving workspace.
fn drop_saved_workspace(m: &Matrix, label: &str) {
    let mut snapshot = saved_session(m);
    let index = saved_workspace(&snapshot, label);
    let workspaces = snapshot["workspaces"].as_array_mut().unwrap();
    workspaces.remove(index);
    let last = workspaces.len().saturating_sub(1);
    for field in ["active", "selected"] {
        if let Some(i) = snapshot[field].as_u64() {
            let i = i as usize;
            let kept = if i > index { i - 1 } else { i };
            snapshot[field] = Value::from(kept.min(last));
        }
    }
    fs::write(
        saved_session_path(m),
        serde_json::to_vec_pretty(&snapshot).unwrap(),
    )
    .unwrap();
}

impl Sweep {
    /// Every Herdr host row's recovery boot, quoted, with whether its marker is reconciled.
    fn host_boots(&self) -> Vec<(String, bool)> {
        self.db()
            .prepare(
                "SELECT quote(recovery_boot), recovery_boot IS reconciled_boot AND recovery_epoch IS reconciled_epoch FROM herdr_hosts",
            )
            .unwrap()
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
            .unwrap()
            .map(Result::unwrap)
            .collect()
    }

    /// Waits for a recovery boot not in `before` to be reconciled: the restarted Herdr was seen.
    fn wait_new_boot_reconciled(&self, before: &[(String, bool)]) {
        wait_for("the new Herdr boot reconciled", QUAL_WAIT, || {
            let boots = self.host_boots();
            if boots
                .iter()
                .any(|(boot, done)| *done && !before.iter().any(|(old, _)| old == boot))
            {
                Ok(())
            } else {
                Err(format!("{boots:?} before {before:?}"))
            }
        });
    }

    /// `(closure_evidence, absence_evidence)` of `table`'s row for `seat`.
    fn retirement_evidence(&self, table: &str, seat: &str) -> (Option<String>, Option<String>) {
        self.db()
            .query_row(
                &format!("SELECT closure_evidence,absence_evidence FROM {table} WHERE seat_id=?1"),
                [seat],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap_or_else(|e| panic!("no {table} row for {seat}: {e}"))
    }
}

/// Source §6 "pane closed during downtime" (G5, ht-ma6i.4): the seats' panes are gone from what
/// the restarted Herdr restores, so a coherent snapshot of the new incarnation never shows their
/// targets. Kills: an unresolved seat left in limbo; an absence retirement that is not reversible
/// for a key-bearing seat, or reversible for a keyless one; evidence other than
/// `restored_target_absent` (or a closure evidence, which only a watched restored pane has); an
/// un-retire or keyed relaunch after it that mints a seat or loses a membership.
#[test]
#[ignore = "qualification: scripts/clone-identity-qual"]
fn qual_pane_closed_during_downtime_retires_by_absence() {
    const TEST: &str = "qual_pane_closed_during_downtime_retires_by_absence";
    const CLOSED: &str = "downtime-closed";
    let Some(s) =
        sweep_with_daemon_env("qual-downtime", &[(RESTORED_PANE_GRACE_ENV, QUAL_GRACE_MS)])
    else {
        return;
    };
    let (sender, _) = s.m.person_seat("downtime-sender");
    let keyed_pane = s.m.workspace(CLOSED);
    let keyed = s.keyed_seat(&keyed_pane, "clone-1");
    let thread = s.joined_thread(&sender, &keyed_pane, &keyed);
    let keyless_pane = s.m.tab(&keyed_pane, "downtime-keyless");
    s.start_agent(&keyless_pane, &s.codex());
    let keyless = wait_for("the keyless seat", WAIT, || {
        let resolved = s.m.ht(&["seat", "resolve", "--pane", &keyless_pane]);
        Ok::<_, String>(text(data(&resolved)))
    });
    assert_ne!(keyless, keyed);
    let seats = s.seats();
    let boots = s.host_boots();

    s.m.host.stop();
    if saved_panes(&s.m, CLOSED).len() != 2 {
        skip_qual(
            TEST,
            &format!(
                "Herdr's saved session does not hold the two panes of {CLOSED:?}: {:?}",
                saved_panes(&s.m, CLOSED)
            ),
        );
        return;
    }
    drop_saved_workspace(&s.m, CLOSED);
    s.m.host.start_with_path_prefix(&s.m.bin);
    s.wait_new_boot_reconciled(&boots);

    s.wait_retirement_for(&keyless, None, QUAL_WAIT);
    s.wait_retirement_for(&keyed, Some("reversible"), QUAL_WAIT);
    assert_eq!(
        s.retirement_evidence("retirements", &keyless),
        (None, Some("restored_target_absent".to_owned())),
        "the keyless seat's retirement"
    );
    assert_eq!(
        s.count("SELECT count(*) FROM retirements WHERE seat_id=?1", &keyed),
        0,
        "the key-bearing seat retired finally"
    );
    assert_eq!(
        s.retirement_evidence("reversible_retirements", &keyed),
        (None, Some("restored_target_absent".to_owned())),
        "the key-bearing seat's reversible retirement"
    );
    assert_eq!(
        s.count(
            "SELECT count(*) FROM restored_target_absence WHERE seat_id=?1",
            &keyed
        ),
        0,
        "the absence ledger outlived the retirement"
    );
    assert_eq!(s.membership(&thread, &keyed), "joined");
    assert_eq!(s.seats(), seats, "the restart minted a seat");

    s.service.unretire(&keyed);
    let inspection = s.inspect(&keyed);
    assert_eq!(
        inspection["summary"]["continuity"], "unresolved",
        "{inspection}"
    );
    assert!(inspection["retirement_state"].is_null(), "{inspection}");
    let relaunched = s.m.workspace("downtime-new");
    s.keyed_agent(&relaunched, "clone-1");
    s.wait_reattached(&keyed, &relaunched);
    assert_eq!(s.seats(), seats, "the relaunch minted a seat");
    assert_eq!(s.reversible_rows(&keyed), 0);
    assert_eq!(s.membership(&thread, &keyed), "joined");
    assert!(is_retired(&s.inspect(&keyless), None));
}

/// Attaches a Herdr client to the isolated server under a pseudo-terminal (`TERM` set, 120x40),
/// so Herdr has a terminal area and starts its pending agent resumes. The helper answers the
/// cursor-position and device-attribute queries a TUI may block on, keeps the newest 64 KiB of
/// the client's output in `client.out` and its wait status in `client.status`, and on SIGTERM
/// stops the client (which leads its own session on the pty) before exiting.
const PTY_CLIENT: &str = r#"import fcntl, os, pty, signal, struct, sys, termios, time
out_path, status_path, argv = sys.argv[1], sys.argv[2], sys.argv[3:]
pid, fd = pty.fork()
if pid == 0:
    fcntl.ioctl(0, termios.TIOCSWINSZ, struct.pack("HHHH", 40, 120, 0, 0))
    os.execvp(argv[0], argv)
tail = b""
def flush():
    with open(out_path, "wb") as f:
        f.write(tail)
def stop(*_):
    for sig in (signal.SIGTERM, signal.SIGKILL):
        try:
            os.kill(pid, sig)
        except ProcessLookupError:
            break
        for _ in range(40):
            try:
                if os.waitpid(pid, os.WNOHANG)[0]:
                    flush()
                    os._exit(0)
            except ChildProcessError:
                flush()
                os._exit(0)
            time.sleep(0.05)
    flush()
    os._exit(0)
signal.signal(signal.SIGTERM, stop)
replies = {b"\x1b[6n": b"\x1b[1;1R", b"\x1b[c": b"\x1b[?62;22c", b"\x1b[0c": b"\x1b[?62;22c"}
flushed = 0.0
while True:
    try:
        chunk = os.read(fd, 65536)
    except OSError:
        break
    if not chunk:
        break
    for query, answer in replies.items():
        if query in chunk:
            os.write(fd, answer)
    tail = (tail + chunk)[-65536:]
    if time.monotonic() - flushed > 1:
        flush()
        flushed = time.monotonic()
flush()
_, status = os.waitpid(pid, 0)
with open(status_path, "w") as f:
    f.write(str(status))
"#;

/// A Herdr client attached to the isolated server; Drop stops it (the helper's process group,
/// whose SIGTERM handler stops the client first).
struct AttachedClient {
    child: OwnedChild,
}

impl AttachedClient {
    /// `herdr client` (the hidden attach-only mode, which never starts a server), else plain
    /// `herdr` (attaches to the running server). `Err` with the client's output when neither
    /// stays attached for 5 s.
    fn attach(m: &Matrix) -> Result<Self, String> {
        let helper = m.host.root().join("pty-client.py");
        fs::write(&helper, PTY_CLIENT).unwrap();
        let mut failures = vec![];
        for argv in [&["herdr", "client"][..], &["herdr"][..]] {
            let out = m.host.root().join("client.out");
            let status = m.host.root().join("client.status");
            let _ = fs::remove_file(&status);
            let mut cmd = m.host.command("python3");
            cmd.arg("-I")
                .arg(&helper)
                .arg(&out)
                .arg(&status)
                .args(argv)
                .env("TERM", "xterm-256color")
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null());
            let mut child = cmd.spawn_owned().unwrap();
            let deadline = Instant::now() + Duration::from_secs(5);
            while Instant::now() < deadline && !status.exists() {
                thread::sleep(Duration::from_millis(200));
            }
            if !status.exists() {
                return Ok(Self { child });
            }
            child.stop();
            let output = fs::read(&out).unwrap_or_default();
            let output = String::from_utf8_lossy(&output);
            failures.push(format!(
                "{argv:?} exited ({}): {}",
                fs::read_to_string(&status).unwrap_or_default(),
                tail(&output, 400)
            ));
        }
        Err(failures.join("; "))
    }

    /// The client's newest output, for a failure message.
    fn output(m: &Matrix) -> String {
        let output = fs::read(m.host.root().join("client.out")).unwrap_or_default();
        tail(&String::from_utf8_lossy(&output), 2000)
    }
}

/// The last `chars` characters of `text`.
fn tail(text: &str, chars: usize) -> String {
    let skip = text.chars().count().saturating_sub(chars);
    text.chars().skip(skip).collect()
}

impl Drop for AttachedClient {
    fn drop(&mut self) {
        self.child.stop();
    }
}

/// Source §6 "cold restart with a client attached" (G3a): Herdr restores the pane and, once a
/// client gives it a terminal area, resumes the agent's saved session there (`codex resume <id>`,
/// Herdr's own resume plan, no continuity key in its environment). Kills: a resumed session that
/// gets a new seat, a reattachment decided by anything but the session (C1,
/// `cooperative_continuity`), a binding that does not reopen for the session, a hold that stays.
#[test]
#[ignore = "qualification: scripts/clone-identity-qual"]
fn qual_cold_restart_with_client_attached_resumes_the_seat() {
    const TEST: &str = "qual_cold_restart_with_client_attached_resumes_the_seat";
    const ATTACHED: &str = "attached-agent";
    let Some(s) = sweep("qual-attached") else {
        return;
    };
    let pane = s.m.workspace(ATTACHED);
    let seat = s.keyed_seat(&pane, "clone-1");
    let session = text(&s.m.log_of(&pane)[0]["session"]);
    let seats = s.seats();
    let boots = s.host_boots();

    s.m.host.stop();
    let saved = saved_panes(&s.m, ATTACHED);
    if !saved
        .iter()
        .any(|p| !p["agent_session"].is_null() || !p["agent_resume"].is_null())
    {
        skip_qual(
            TEST,
            &format!("Herdr saved no resumable agent session for the pane: {saved:?}"),
        );
        return;
    }
    s.m.host.start_with_path_prefix(&s.m.bin);
    s.wait_new_boot_reconciled(&boots);
    s.wait_continuity(&seat, "unresolved");
    assert!(s.restore_held(), "the restored host is not held");

    let client = match AttachedClient::attach(&s.m) {
        Ok(client) => client,
        Err(reason) => {
            skip_qual(TEST, &format!("no Herdr client could attach: {reason}"));
            return;
        }
    };
    let resumed = wait_for("Herdr resumed the agent", QUAL_WAIT, || {
        let log = s.m.log_of(&pane);
        let started = log
            .first()
            .is_some_and(|e| e["kind"] == "start" && e["argv"][0] == "resume");
        if started
            && log.iter().any(|e| e["kind"] == "ready")
            && log
                .iter()
                .any(|e| e["kind"] == "hook" && e["event"] == "SessionStart")
        {
            Ok(log)
        } else {
            Err(format!("{log:?}; client: {}", AttachedClient::output(&s.m)))
        }
    });
    assert_eq!(text(&resumed[0]["session"]), session, "{resumed:?}");
    for event in resumed.iter().filter(|e| e["kind"] == "hook") {
        assert_eq!(event["rc"], 0, "a hook failed: {event}");
    }
    wait_for(
        &format!("{seat} reattached by its session"),
        QUAL_WAIT,
        || {
            let inspection = s.inspect(&seat);
            let decided = s.decisions(&seat);
            if inspection["summary"]["continuity"] == "resolved"
                && inspection["summary"]["target"] == pane.as_str()
                && decided.last().map(String::as_str) == Some("cooperative_continuity")
                && s.open_binding_of(&seat, &session)
            {
                Ok(())
            } else {
                Err(format!("{inspection} decisions: {decided:?}"))
            }
        },
    );
    assert_eq!(s.seats(), seats, "the resumed session got a seat");
    assert_eq!(s.key_seat("clone-1").as_deref(), Some(seat.as_str()));
    assert!(
        s.alerts_of("clone-1").is_empty(),
        "{:?}",
        s.alerts_of("clone-1")
    );
    assert!(!s.restore_held(), "the hold stands");
    drop(client);
}

/// Source §6 "resurrection, keeping the seat id": the pane closes, nothing finalizes, Herdr
/// cold-restarts, and only then does the service un-retire and relaunch with the key. Kills: a
/// reversible retirement a restart finalizes or forgets, a restart that minted or revived a seat,
/// an un-retire refused after a new incarnation, a keyed relaunch that gets another seat or needs
/// a re-invitation or re-acceptance.
#[test]
#[ignore = "qualification: scripts/clone-identity-qual"]
fn qual_resurrection_after_cold_restart_keeps_the_seat() {
    let Some(s) = sweep("qual-resurrect") else {
        return;
    };
    let (sender, _) = s.m.person_seat("resurrect-sender");
    let pane = s.m.workspace("resurrect-agent");
    let seat = s.keyed_seat(&pane, "clone-1");
    let thread = s.joined_thread(&sender, &pane, &seat);
    let invitations = s.count("SELECT count(*) FROM invitations WHERE seat_id=?1", &seat);
    s.close_pane(&pane);
    s.wait_retirement(&seat, Some("reversible"));
    let seats = s.seats();
    let boots = s.host_boots();

    s.m.host.stop();
    s.m.host.start_with_path_prefix(&s.m.bin);
    s.wait_new_boot_reconciled(&boots);
    // One more observation pass over the reconciled boot.
    thread::sleep(Duration::from_secs(6));
    assert_eq!(s.inspect(&seat)["retirement_state"], "reversible");
    assert_eq!(s.reversible_rows(&seat), 1);
    assert_eq!(
        s.count("SELECT count(*) FROM retirements WHERE seat_id=?1", &seat),
        0,
        "the restart finalized the retirement"
    );
    assert_eq!(s.key_seat("clone-1").as_deref(), Some(seat.as_str()));
    assert_eq!(s.membership(&thread, &seat), "joined");
    assert_eq!(s.seats(), seats, "the restart minted a seat");

    s.service.unretire(&seat);
    let inspection = s.inspect(&seat);
    assert_eq!(
        inspection["summary"]["continuity"], "unresolved",
        "{inspection}"
    );
    assert!(inspection["retirement_state"].is_null(), "{inspection}");
    let relaunched = s.m.workspace("resurrect-new");
    s.keyed_agent(&relaunched, "clone-1");
    s.wait_reattached(&seat, &relaunched);
    assert_eq!(s.seats(), seats, "the relaunch minted a seat");
    assert_eq!(s.reversible_rows(&seat), 0);
    assert_eq!(s.membership(&thread, &seat), "joined");
    assert_eq!(
        s.count("SELECT count(*) FROM invitations WHERE seat_id=?1", &seat),
        invitations,
        "a re-invitation"
    );
    // No re-acceptance: the resurrected seat acts on the thread.
    s.m.agent_ht(&relaunched, &format!("send {thread} --body back"));
}
