//! TRUST-POLICY C1 process continuity (ht-hsyg): whether a binding's own process lives in the
//! pane Herdr reports, judged from kernel facts only.
//!
//! A new Herdr incarnation proves nothing about old terminal ids (C2), but it can keep the pane
//! processes alive: a live handoff passes each pane's terminal to a new server process. The
//! evidence is the binding's liveness fingerprint (pid and start stamp, recorded from the
//! process's own claim, under the kernel boot id the daemon read then) together with the pane's
//! shell as Herdr reports it: the process is that shell or belongs to the session the shell leads,
//! on the shell's controlling terminal. After a cold restart the old shells and terminals are
//! gone, so it cannot match.
use crate::{
    ports::{ProcessContinuityCandidate, ProcessInfo},
    protocol::{authority::LivenessFingerprint, ids::SeatId},
};

/// The judgment for one binding at one pane.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    /// The fingerprinted process is the pane's shell or lives in its session on its terminal.
    Continuous,
    /// The facts were read and they rule it out: the process is in another session or on another
    /// terminal, or the shell leads no session on a terminal. None of these turns into a match
    /// for the same process, so the caller may remember it.
    NotContinuous,
    /// The fingerprinted process is not (or no longer) readable as that process: it exited, the
    /// pid was reused, or the read failed. Never remembered; it costs no host read to re-check.
    Gone,
    /// A fact could not be read; nothing is decided.
    Unknown,
}

/// Whether `recorded` names the running kernel boot. An unknown boot on either side is not the
/// same boot.
pub fn same_boot(processes: &dyn ProcessInfo, recorded: Option<&str>) -> bool {
    recorded.is_some_and(|recorded| processes.kernel_boot().as_deref() == Some(recorded))
}

/// Judge `fingerprint`, recorded under the kernel boot `recorded_boot`, against the pane whose
/// process Herdr reports as `shell_pid`. A reboot can repeat a pid, start tick and terminal, so the
/// boot is compared first: a fingerprint of another boot is `Gone` (its process cannot exist in
/// this one), and an unknown boot on either side is `Unknown` unless the process is not running.
pub fn judge(
    processes: &dyn ProcessInfo,
    fingerprint: LivenessFingerprint,
    recorded_boot: Option<&str>,
    shell_pid: u32,
) -> Verdict {
    if !same_boot(processes, recorded_boot) {
        let known = recorded_boot.is_some() && processes.kernel_boot().is_some();
        return if known || !fingerprint.is_alive(processes) {
            Verdict::Gone
        } else {
            Verdict::Unknown
        };
    }
    if !fingerprint.is_alive(processes) {
        return Verdict::Gone;
    }
    let Some(process) = processes.terminal_session(fingerprint.pid) else {
        return Verdict::Unknown;
    };
    // The pid may have exited and been reused while its session was read.
    if !fingerprint.is_alive(processes) {
        return Verdict::Gone;
    }
    let Some(shell) = processes.terminal_session(shell_pid) else {
        return Verdict::Unknown;
    };
    let Some(terminal) = shell.tty else {
        return Verdict::NotContinuous;
    };
    let in_session = fingerprint.pid == shell_pid || process.session == shell_pid;
    if shell.session == shell_pid && in_session && process.tty == Some(terminal) {
        Verdict::Continuous
    } else {
        Verdict::NotContinuous
    }
}

/// What a remembered decline is keyed by: (binding id, seat generation, fingerprint pid, start).
pub type DeclineKey = (String, u64, u32, u64);

pub fn decline_key(candidate: &ProcessContinuityCandidate) -> DeclineKey {
    (
        candidate.binding_id.clone(),
        candidate.seat_generation,
        candidate.liveness.pid,
        candidate.liveness.start,
    )
}

/// One seat's judgment at its saved target.
#[derive(Debug, PartialEq, Eq)]
pub struct SeatJudgment<'a> {
    /// The seat's newest candidate binding whose process the pane hosts.
    pub matched: Option<&'a ProcessContinuityCandidate>,
    /// Candidates the facts ruled out, to remember.
    pub declines: Vec<DeclineKey>,
}

/// Judge one seat's candidates (`group`: one seat, newest binding first) against the pane whose
/// process Herdr reports as `shell_pid` (`None`: unread, nothing decided). Already declined
/// candidates are skipped.
pub fn judge_seat<'a>(
    group: &'a [ProcessContinuityCandidate],
    processes: &dyn ProcessInfo,
    shell_pid: Option<u32>,
    declined: &dyn Fn(&DeclineKey) -> bool,
) -> SeatJudgment<'a> {
    let mut judgment = SeatJudgment {
        matched: None,
        declines: Vec::new(),
    };
    let Some(shell_pid) = shell_pid else {
        return judgment;
    };
    for candidate in group {
        let key = decline_key(candidate);
        if declined(&key) {
            continue;
        }
        match judge(
            processes,
            candidate.liveness,
            candidate.liveness_boot.as_deref(),
            shell_pid,
        ) {
            Verdict::Continuous => {
                judgment.matched = Some(candidate);
                break;
            }
            Verdict::NotContinuous => judgment.declines.push(key),
            Verdict::Gone | Verdict::Unknown => {}
        }
    }
    judgment
}

/// Whether another seat saved at the same target (`rows`: every candidate there) has a process
/// that the pane whose process Herdr reports as `shell_pid` hosts, or one whose facts cannot be
/// read: then the pane's process could be either seat's, so nothing is decided (seats are never
/// chosen between on evidence that fits two). A rival whose process is gone or lives elsewhere
/// does not block.
pub fn has_rival_in_pane(
    rows: &[ProcessContinuityCandidate],
    seat: &SeatId,
    processes: &dyn ProcessInfo,
    shell_pid: u32,
) -> bool {
    rows.iter().any(|row| {
        &row.seat != seat
            && matches!(
                judge(
                    processes,
                    row.liveness,
                    row.liveness_boot.as_deref(),
                    shell_pid
                ),
                Verdict::Continuous | Verdict::Unknown
            )
    })
}

/// Split one listed page (seat order) into per-seat groups. A full page (`full`) may have cut its
/// last seat's rows short, so that seat is left for the next page unless it is the only one.
/// Returns the groups and whether the page ended the listing.
pub fn page_groups(
    rows: Vec<ProcessContinuityCandidate>,
    full: bool,
) -> (Vec<Vec<ProcessContinuityCandidate>>, bool) {
    let mut groups: Vec<Vec<ProcessContinuityCandidate>> = Vec::new();
    for row in rows {
        match groups.last_mut() {
            Some(group) if group[0].seat == row.seat => group.push(row),
            _ => groups.push(vec![row]),
        }
    }
    if full && groups.len() > 1 {
        groups.pop();
    }
    (groups, !full)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ports::{ProcessRecord, TerminalSession};
    use std::{
        collections::HashMap,
        sync::atomic::{AtomicBool, Ordering},
    };

    /// A process table: pid -> (start, session, tty). `exit_after_session_read` makes the named
    /// pid vanish once its session has been read (a pid reused mid-read).
    #[derive(Default)]
    struct Table {
        rows: HashMap<u32, (u64, u32, Option<u64>)>,
        exit_after_session_read: Option<u32>,
        gone: AtomicBool,
        boot: Option<&'static str>,
    }
    impl ProcessInfo for Table {
        fn kernel_boot(&self) -> Option<String> {
            self.boot.map(str::to_owned)
        }
        fn process(&self, pid: u32) -> Option<ProcessRecord> {
            if self.gone.load(Ordering::SeqCst) && self.exit_after_session_read == Some(pid) {
                return None;
            }
            self.rows.get(&pid).map(|(start, _, _)| ProcessRecord {
                pid,
                parent: None,
                start: *start,
                argv: Vec::new(),
            })
        }
        fn terminal_session(&self, pid: u32) -> Option<TerminalSession> {
            if self.exit_after_session_read == Some(pid) {
                self.gone.store(true, Ordering::SeqCst);
            }
            self.rows
                .get(&pid)
                .map(|(_, session, tty)| TerminalSession {
                    session: *session,
                    tty: *tty,
                })
        }
    }

    const SHELL: u32 = 100;
    const AGENT: u32 = 200;
    const TTY: u64 = 34817;

    const BOOT: &str = "boot-a";

    fn booted() -> Table {
        Table {
            boot: Some(BOOT),
            ..Table::default()
        }
    }

    fn table(agent: (u64, u32, Option<u64>)) -> Table {
        let mut t = booted();
        t.rows.insert(SHELL, (1, SHELL, Some(TTY)));
        t.rows.insert(AGENT, agent);
        t
    }

    fn agent(start: u64) -> LivenessFingerprint {
        LivenessFingerprint { pid: AGENT, start }
    }

    #[test]
    fn a_live_process_in_the_shells_session_on_its_terminal_is_continuous() {
        assert_eq!(
            judge(&table((7, SHELL, Some(TTY))), agent(7), Some(BOOT), SHELL),
            Verdict::Continuous
        );
    }

    /// A harness Herdr started as the pane's own process is the shell itself.
    #[test]
    fn the_pane_process_itself_is_continuous() {
        let mut t = booted();
        t.rows.insert(AGENT, (7, AGENT, Some(TTY)));
        assert_eq!(judge(&t, agent(7), Some(BOOT), AGENT), Verdict::Continuous);
    }

    /// Kills: skipping the start-stamp check (pid reuse) or the liveness check, or remembering
    /// an unreadable process as ruled out.
    #[test]
    fn a_gone_or_reused_pid_is_gone() {
        assert_eq!(
            judge(&table((8, SHELL, Some(TTY))), agent(7), Some(BOOT), SHELL),
            Verdict::Gone
        );
        let mut t = table((7, SHELL, Some(TTY)));
        t.rows.remove(&AGENT);
        assert_eq!(judge(&t, agent(7), Some(BOOT), SHELL), Verdict::Gone);
    }

    /// Kills: dropping the liveness re-check after the session read.
    #[test]
    fn a_pid_that_exits_while_its_session_is_read_is_gone() {
        let mut t = table((7, SHELL, Some(TTY)));
        t.exit_after_session_read = Some(AGENT);
        assert_eq!(judge(&t, agent(7), Some(BOOT), SHELL), Verdict::Gone);
    }

    /// A survivor of a cold restart (it ignored SIGHUP): its old session lost the terminal at
    /// hangup and is not the new shell's. Kills: comparing only the terminal or only the session.
    #[test]
    fn another_session_or_terminal_is_not_continuous() {
        assert_eq!(
            judge(&table((7, 555, Some(TTY))), agent(7), Some(BOOT), SHELL),
            Verdict::NotContinuous,
            "another session (setsid, tmux)"
        );
        assert_eq!(
            judge(
                &table((7, SHELL, Some(TTY + 1))),
                agent(7),
                Some(BOOT),
                SHELL
            ),
            Verdict::NotContinuous,
            "another terminal"
        );
        assert_eq!(
            judge(&table((7, SHELL, None)), agent(7), Some(BOOT), SHELL),
            Verdict::NotContinuous,
            "no controlling terminal (hung up)"
        );
    }

    /// Kills: accepting a shell that leads no session or has no terminal.
    #[test]
    fn a_shell_that_leads_no_session_on_a_terminal_rules_it_out() {
        let mut t = table((7, SHELL, None));
        t.rows.insert(SHELL, (1, SHELL, None));
        assert_eq!(
            judge(&t, agent(7), Some(BOOT), SHELL),
            Verdict::NotContinuous
        );
        let mut t = table((7, 99, Some(TTY)));
        t.rows.insert(SHELL, (1, 99, Some(TTY)));
        assert_eq!(
            judge(&t, agent(7), Some(BOOT), SHELL),
            Verdict::NotContinuous
        );
    }

    /// The pane's own process that leads no session (it is not the shell Herdr started, or it
    /// left its session). Kills: dropping the `shell.session == shell_pid` check, the only check
    /// that decides when the fingerprint is the reported pid itself.
    #[test]
    fn the_pane_process_that_leads_no_session_is_not_continuous() {
        let mut t = booted();
        t.rows.insert(AGENT, (7, 999, Some(TTY)));
        assert_eq!(
            judge(&t, agent(7), Some(BOOT), AGENT),
            Verdict::NotContinuous
        );
    }

    fn candidate(seat: &str, binding: &str, pid: u32, start: u64) -> ProcessContinuityCandidate {
        use crate::protocol::ids::{
            ExecutionId, HostLocator, HostTargetId, NativeSessionId, SeatId,
        };
        ProcessContinuityCandidate {
            seat: SeatId::new(seat),
            seat_generation: 2,
            binding_id: binding.into(),
            target: HostTargetId::new("w1:p1"),
            host: HostLocator::parse("/tmp/h.sock").unwrap(),
            harness: crate::harness::registry::OccupantHarness::Agent(
                crate::harness::registry::builtins().agent("codex").unwrap(),
            ),
            native_session: NativeSessionId::new(format!("session-{binding}")),
            execution: ExecutionId::new(format!("exec-{binding}")),
            liveness: LivenessFingerprint { pid, start },
            liveness_boot: Some(BOOT.into()),
        }
    }

    /// Kills: picking an older binding over the seat's newest matching one, judging a declined
    /// candidate again, or remembering a gone process as ruled out.
    #[test]
    fn a_seat_matches_its_newest_continuous_binding_and_declines_only_ruled_out_ones() {
        let mut t = table((7, SHELL, Some(TTY)));
        t.rows.insert(300, (5, 4444, Some(TTY))); // another session: ruled out
        t.rows.insert(301, (6, SHELL, Some(TTY))); // continuous, older
        let group = [
            candidate("s1", "newest-other", 300, 5),
            candidate("s1", "gone", 302, 1),
            candidate("s1", "match", AGENT, 7),
            candidate("s1", "older", 301, 6),
        ];
        let judged = judge_seat(&group, &t, Some(SHELL), &|_| false);
        assert_eq!(judged.matched.map(|c| c.binding_id.as_str()), Some("match"));
        assert_eq!(judged.declines, vec![decline_key(&group[0])]);
        let skip_match = decline_key(&group[2]);
        let judged = judge_seat(&group, &t, Some(SHELL), &|key| *key == skip_match);
        assert_eq!(judged.matched.map(|c| c.binding_id.as_str()), Some("older"));
        let judged = judge_seat(&group, &t, None, &|_| false);
        assert_eq!(
            judged,
            SeatJudgment {
                matched: None,
                declines: Vec::new()
            },
            "an unread pane decides and remembers nothing"
        );
    }

    /// Kills: ignoring another seat's process in the pane, counting the seat's own, a dead rival
    /// or one alive outside the pane (a cold-restart survivor), or deciding past an unreadable one.
    #[test]
    fn only_another_seats_process_in_the_pane_is_a_rival() {
        let mut t = table((7, SHELL, Some(TTY)));
        t.rows.insert(400, (3, 4000, None)); // a nohup survivor: alive, no terminal
        t.rows.insert(401, (3, SHELL, Some(TTY))); // another seat's process in this pane
        let own = candidate("s1", "a", AGENT, 7);
        let dead = candidate("s2", "b", 999, 1);
        let elsewhere = candidate("s2", "c", 400, 3);
        let seat = own.seat.clone();
        let quiet = [own.clone(), dead.clone(), elsewhere.clone()];
        assert!(!has_rival_in_pane(&quiet, &seat, &t, SHELL));
        let in_pane = candidate("s3", "d", 401, 3);
        assert!(has_rival_in_pane(
            &[own.clone(), dead, elsewhere, in_pane],
            &seat,
            &t,
            SHELL
        ));
        let mut unreadable = table((7, SHELL, Some(TTY)));
        unreadable.rows.insert(402, (3, SHELL, Some(TTY)));
        unreadable.rows.remove(&SHELL);
        let maybe = candidate("s4", "e", 402, 3);
        assert!(
            has_rival_in_pane(&[own, maybe], &seat, &unreadable, SHELL),
            "an unreadable rival blocks"
        );
    }

    /// Kills: dropping a seat whose rows a full page may have cut short when other seats share the
    /// page, dropping the only seat of a full page (no progress), or calling a full page the end.
    #[test]
    fn a_full_page_leaves_its_last_seat_for_the_next_page() {
        let rows = || {
            vec![
                candidate("s1", "a", 1, 1),
                candidate("s1", "b", 2, 1),
                candidate("s2", "c", 3, 1),
            ]
        };
        let (groups, complete) = page_groups(rows(), true);
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].len(), 2);
        assert!(!complete);
        let (groups, complete) = page_groups(rows(), false);
        assert_eq!(groups.len(), 2);
        assert!(complete);
        let (groups, complete) = page_groups(rows()[..2].to_vec(), true);
        assert_eq!(
            groups.len(),
            1,
            "a full page of one seat still makes progress"
        );
        assert!(!complete);
    }

    /// After a reboot a process can repeat the recorded pid and start tick on the same terminal.
    /// Kills: judging without the kernel boot id, or taking an unknown one as a decision.
    #[test]
    fn a_live_process_recorded_under_another_kernel_boot_is_gone() {
        let mut t = table((7, SHELL, Some(TTY)));
        assert_eq!(judge(&t, agent(7), Some(BOOT), SHELL), Verdict::Continuous);
        t.boot = Some("boot-b");
        assert_eq!(judge(&t, agent(7), Some(BOOT), SHELL), Verdict::Gone);
        let mut stale = candidate("s1", "a", AGENT, 7);
        let rival = candidate("s2", "b", AGENT, 7);
        let judged = judge_seat(std::slice::from_ref(&stale), &t, Some(SHELL), &|_| false);
        assert_eq!(
            judged,
            SeatJudgment {
                matched: None,
                declines: Vec::new()
            }
        );
        assert!(!has_rival_in_pane(
            &[stale.clone(), rival],
            &stale.seat,
            &t,
            SHELL
        ));
        t.boot = Some(BOOT);
        assert_eq!(judge(&t, agent(7), None, SHELL), Verdict::Unknown);
        stale.liveness_boot = None;
        let judged = judge_seat(std::slice::from_ref(&stale), &t, Some(SHELL), &|_| false);
        assert_eq!(
            judged.matched, None,
            "an unrecorded boot id matches nothing"
        );
        t.boot = None;
        assert_eq!(judge(&t, agent(7), Some(BOOT), SHELL), Verdict::Unknown);
        let mut dead = table((7, SHELL, Some(TTY)));
        dead.rows.remove(&AGENT);
        dead.boot = None;
        assert_eq!(
            judge(&dead, agent(7), Some(BOOT), SHELL),
            Verdict::Gone,
            "a process that is not running is gone in any boot"
        );
    }

    /// Kills: treating an unreadable fact as a match or as a definitive refusal.
    #[test]
    fn an_unreadable_fact_is_unknown() {
        let mut t = table((7, SHELL, Some(TTY)));
        t.rows.remove(&SHELL);
        assert_eq!(judge(&t, agent(7), Some(BOOT), SHELL), Verdict::Unknown);
    }
}
