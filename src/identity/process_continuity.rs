//! TRUST-POLICY C1 process continuity (ht-hsyg): whether a binding's own process lives in the
//! pane Herdr reports, judged from kernel facts only.
//!
//! A new Herdr incarnation proves nothing about old terminal ids (C2), but it can keep the pane
//! processes alive: a live handoff passes each pane's terminal to a new server process. The
//! evidence is the binding's liveness fingerprint (pid and start stamp, recorded from the
//! process's own claim) together with the pane's shell as Herdr reports it: the process is that
//! shell or belongs to the session the shell leads, on the shell's controlling terminal. After a
//! cold restart the old shells and terminals are gone, so it cannot match.
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

/// Judge `fingerprint` against the pane whose process Herdr reports as `shell_pid`.
pub fn judge(
    processes: &dyn ProcessInfo,
    fingerprint: LivenessFingerprint,
    shell_pid: u32,
) -> Verdict {
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
        match judge(processes, candidate.liveness, shell_pid) {
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

/// Whether another seat saved at the same target (`rows`: every candidate there) still has a
/// live fingerprinted process: then the pane's process could be either seat's, so nothing is
/// decided (seats are never chosen between on evidence that fits two).
pub fn has_live_rival(
    rows: &[ProcessContinuityCandidate],
    seat: &SeatId,
    processes: &dyn ProcessInfo,
) -> bool {
    rows.iter()
        .any(|row| &row.seat != seat && row.liveness.is_alive(processes))
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
    }
    impl ProcessInfo for Table {
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

    fn table(agent: (u64, u32, Option<u64>)) -> Table {
        let mut t = Table::default();
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
            judge(&table((7, SHELL, Some(TTY))), agent(7), SHELL),
            Verdict::Continuous
        );
    }

    /// A harness Herdr started as the pane's own process is the shell itself.
    #[test]
    fn the_pane_process_itself_is_continuous() {
        let mut t = Table::default();
        t.rows.insert(AGENT, (7, AGENT, Some(TTY)));
        assert_eq!(judge(&t, agent(7), AGENT), Verdict::Continuous);
    }

    /// Kills: skipping the start-stamp check (pid reuse) or the liveness check, or remembering
    /// an unreadable process as ruled out.
    #[test]
    fn a_gone_or_reused_pid_is_gone() {
        assert_eq!(
            judge(&table((8, SHELL, Some(TTY))), agent(7), SHELL),
            Verdict::Gone
        );
        let mut t = table((7, SHELL, Some(TTY)));
        t.rows.remove(&AGENT);
        assert_eq!(judge(&t, agent(7), SHELL), Verdict::Gone);
    }

    /// Kills: dropping the liveness re-check after the session read.
    #[test]
    fn a_pid_that_exits_while_its_session_is_read_is_gone() {
        let mut t = table((7, SHELL, Some(TTY)));
        t.exit_after_session_read = Some(AGENT);
        assert_eq!(judge(&t, agent(7), SHELL), Verdict::Gone);
    }

    /// A survivor of a cold restart (it ignored SIGHUP): its old session lost the terminal at
    /// hangup and is not the new shell's. Kills: comparing only the terminal or only the session.
    #[test]
    fn another_session_or_terminal_is_not_continuous() {
        assert_eq!(
            judge(&table((7, 555, Some(TTY))), agent(7), SHELL),
            Verdict::NotContinuous,
            "another session (setsid, tmux)"
        );
        assert_eq!(
            judge(&table((7, SHELL, Some(TTY + 1))), agent(7), SHELL),
            Verdict::NotContinuous,
            "another terminal"
        );
        assert_eq!(
            judge(&table((7, SHELL, None)), agent(7), SHELL),
            Verdict::NotContinuous,
            "no controlling terminal (hung up)"
        );
    }

    /// Kills: accepting a shell that leads no session or has no terminal.
    #[test]
    fn a_shell_that_leads_no_session_on_a_terminal_rules_it_out() {
        let mut t = table((7, SHELL, None));
        t.rows.insert(SHELL, (1, SHELL, None));
        assert_eq!(judge(&t, agent(7), SHELL), Verdict::NotContinuous);
        let mut t = table((7, 99, Some(TTY)));
        t.rows.insert(SHELL, (1, 99, Some(TTY)));
        assert_eq!(judge(&t, agent(7), SHELL), Verdict::NotContinuous);
    }

    /// The pane's own process that leads no session (it is not the shell Herdr started, or it
    /// left its session). Kills: dropping the `shell.session == shell_pid` check, the only check
    /// that decides when the fingerprint is the reported pid itself.
    #[test]
    fn the_pane_process_that_leads_no_session_is_not_continuous() {
        let mut t = Table::default();
        t.rows.insert(AGENT, (7, 999, Some(TTY)));
        assert_eq!(judge(&t, agent(7), AGENT), Verdict::NotContinuous);
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

    /// Kills: ignoring another seat's live process at the same target, or counting the seat's own
    /// or a dead rival.
    #[test]
    fn only_another_seats_live_process_is_a_rival() {
        let t = table((7, SHELL, Some(TTY)));
        let own = candidate("s1", "a", AGENT, 7);
        let dead = candidate("s2", "b", 999, 1);
        let live = candidate("s2", "c", SHELL, 1);
        let seat = own.seat.clone();
        assert!(!has_live_rival(&[own.clone(), dead.clone()], &seat, &t));
        assert!(has_live_rival(&[own, dead, live], &seat, &t));
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

    /// Kills: treating an unreadable fact as a match or as a definitive refusal.
    #[test]
    fn an_unreadable_fact_is_unknown() {
        let mut t = table((7, SHELL, Some(TTY)));
        t.rows.remove(&SHELL);
        assert_eq!(judge(&t, agent(7), SHELL), Verdict::Unknown);
    }
}
