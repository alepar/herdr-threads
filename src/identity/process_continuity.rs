//! TRUST-POLICY C1 process continuity (ht-hsyg): whether a binding's own process lives in the
//! pane Herdr reports, judged from kernel facts only.
//!
//! A new Herdr incarnation proves nothing about old terminal ids (C2), but it can keep the pane
//! processes alive: a live handoff passes each pane's terminal to a new server process. The
//! evidence is the binding's liveness fingerprint (pid and start stamp, recorded from the
//! process's own claim) together with the pane's shell as Herdr reports it: the process is that
//! shell or belongs to the session the shell leads, on the shell's controlling terminal. After a
//! cold restart the old shells and terminals are gone, so it cannot match.
use crate::{ports::ProcessInfo, protocol::authority::LivenessFingerprint};

/// The judgment for one binding at one pane.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    /// The fingerprinted process is the pane's shell or lives in its session on its terminal.
    Continuous,
    /// The facts were read and they rule it out (the process is gone, in another session or on
    /// another terminal, or the shell leads no session on a terminal).
    NotContinuous,
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
        return Verdict::NotContinuous;
    }
    let Some(process) = processes.terminal_session(fingerprint.pid) else {
        return Verdict::Unknown;
    };
    // The pid may have exited and been reused while its session was read.
    if !fingerprint.is_alive(processes) {
        return Verdict::NotContinuous;
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

    /// Kills: skipping the start-stamp check (pid reuse) or the liveness check.
    #[test]
    fn a_gone_or_reused_pid_is_not_continuous() {
        assert_eq!(
            judge(&table((8, SHELL, Some(TTY))), agent(7), SHELL),
            Verdict::NotContinuous
        );
        let mut t = table((7, SHELL, Some(TTY)));
        t.rows.remove(&AGENT);
        assert_eq!(judge(&t, agent(7), SHELL), Verdict::NotContinuous);
    }

    /// Kills: dropping the liveness re-check after the session read.
    #[test]
    fn a_pid_that_exits_while_its_session_is_read_is_not_continuous() {
        let mut t = table((7, SHELL, Some(TTY)));
        t.exit_after_session_read = Some(AGENT);
        assert_eq!(judge(&t, agent(7), SHELL), Verdict::NotContinuous);
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

    /// Kills: treating an unreadable fact as a match or as a definitive refusal.
    #[test]
    fn an_unreadable_fact_is_unknown() {
        let mut t = table((7, SHELL, Some(TTY)));
        t.rows.remove(&SHELL);
        assert_eq!(judge(&t, agent(7), SHELL), Verdict::Unknown);
    }
}
