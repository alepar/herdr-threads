//! TRUST-POLICY C1 shell continuity (ht-ma6i.1): whether the pane shell Herdr reports is the very
//! process a seat's fingerprint recorded, judged from kernel facts only.
//!
//! A seat with no live agent (a bare pane, an exited agent, a launch that never checked in) has no
//! binding process for [`crate::identity::process_continuity`] to find, but its pane's shell
//! survives a live handoff. A cold restart cannot match: the old shells are gone.
use crate::{
    identity::process_continuity::{self, Verdict},
    ports::{ProcessInfo, ShellContinuityCandidate},
    protocol::{authority::LivenessFingerprint, ids::SeatId},
};

/// A pane shell's identity while its terminal stands: its pid and start stamp, the session it
/// leads, its controlling terminal's device number, and the kernel boot they belong to (a reboot
/// can repeat the others).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShellFingerprint {
    pub pid: u32,
    pub start: u64,
    pub session: u32,
    pub tty: u64,
    pub kernel_boot: String,
}

/// `None`: unreadable (the kernel boot id included), leads no session, or no controlling
/// terminal. Nothing is recorded.
pub fn read(processes: &dyn ProcessInfo, pid: u32) -> Option<ShellFingerprint> {
    let kernel_boot = processes.kernel_boot()?;
    let start = processes.process(pid)?.start;
    let session = processes.terminal_session(pid)?;
    let tty = session.tty?;
    // The pid may have exited and been reused while its session was read.
    (session.session == pid && processes.process(pid)?.start == start).then_some(ShellFingerprint {
        pid,
        start,
        session: session.session,
        tty,
        kernel_boot,
    })
}

/// Judge `saved` against the pane whose process Herdr reports as `shell_pid`.
pub fn judge(processes: &dyn ProcessInfo, saved: &ShellFingerprint, shell_pid: u32) -> Verdict {
    if !process_continuity::same_boot(processes, Some(&saved.kernel_boot)) {
        return if processes.kernel_boot().is_some() {
            Verdict::Gone
        } else {
            Verdict::Unknown
        };
    }
    if saved.pid != shell_pid {
        return Verdict::NotContinuous;
    }
    let fingerprint = LivenessFingerprint {
        pid: saved.pid,
        start: saved.start,
    };
    match process_continuity::judge(processes, fingerprint, Some(&saved.kernel_boot), shell_pid) {
        Verdict::Continuous => {}
        verdict => return verdict,
    }
    let Some(session) = processes.terminal_session(shell_pid) else {
        return Verdict::Unknown;
    };
    if session.session == shell_pid && session.tty == Some(saved.tty) {
        Verdict::Continuous
    } else {
        Verdict::NotContinuous
    }
}

/// What a remembered decline is keyed by: (seat, seat generation, fingerprint pid, start).
pub type DeclineKey = (SeatId, u64, u32, u64);

pub fn decline_key(candidate: &ShellContinuityCandidate) -> DeclineKey {
    (
        candidate.seat.clone(),
        candidate.seat_generation,
        candidate.fingerprint.pid,
        candidate.fingerprint.start,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ports::{ProcessRecord, TerminalSession};
    use std::collections::HashMap;

    /// A process table: pid -> (start, session, tty). A pid in `no_session` has a process record
    /// and an unreadable session.
    #[derive(Default)]
    struct Table {
        rows: HashMap<u32, (u64, u32, Option<u64>)>,
        no_session: Option<u32>,
        boot: Option<&'static str>,
    }
    impl ProcessInfo for Table {
        fn kernel_boot(&self) -> Option<String> {
            self.boot.map(str::to_owned)
        }
        fn process(&self, pid: u32) -> Option<ProcessRecord> {
            self.rows.get(&pid).map(|(start, _, _)| ProcessRecord {
                pid,
                parent: None,
                start: *start,
                argv: Vec::new(),
            })
        }
        fn terminal_session(&self, pid: u32) -> Option<TerminalSession> {
            if self.no_session == Some(pid) {
                return None;
            }
            self.rows
                .get(&pid)
                .map(|(_, session, tty)| TerminalSession {
                    session: *session,
                    tty: *tty,
                })
        }
    }

    impl Table {
        fn without_rows(mut self) -> Self {
            self.rows.clear();
            self
        }
    }

    const SHELL: u32 = 100;
    const TTY: u64 = 34817;
    const START: u64 = 7;

    fn saved() -> ShellFingerprint {
        ShellFingerprint {
            pid: SHELL,
            start: START,
            session: SHELL,
            tty: TTY,
            kernel_boot: BOOT.into(),
        }
    }

    const BOOT: &str = "boot-a";

    fn table(row: (u64, u32, Option<u64>)) -> Table {
        let mut t = Table {
            boot: Some(BOOT),
            ..Table::default()
        };
        t.rows.insert(SHELL, row);
        t
    }

    #[test]
    fn read_records_a_session_leader_on_a_terminal() {
        assert_eq!(
            read(&table((START, SHELL, Some(TTY))), SHELL),
            Some(saved())
        );
    }

    /// Kills: recording a shell that leads no session, has no terminal, or cannot be read.
    #[test]
    fn read_records_nothing_for_a_shell_with_no_session_or_terminal() {
        assert_eq!(read(&table((START, 999, Some(TTY))), SHELL), None);
        assert_eq!(read(&table((START, SHELL, None)), SHELL), None);
        assert_eq!(
            read(&table((START, SHELL, Some(TTY))).without_rows(), SHELL),
            None
        );
        let mut t = table((START, SHELL, Some(TTY)));
        t.no_session = Some(SHELL);
        assert_eq!(read(&t, SHELL), None);
    }

    #[test]
    fn the_same_shell_on_its_terminal_is_continuous() {
        assert_eq!(
            judge(&table((START, SHELL, Some(TTY))), &saved(), SHELL),
            Verdict::Continuous
        );
    }

    /// A cold restart gives the pane a new shell, and the old one is gone. Kills: judging only the
    /// saved pid's own facts, without comparing it with the pane's reported shell.
    #[test]
    fn a_new_shell_after_a_cold_restart_is_not_continuous() {
        let mut t = table((START, SHELL, Some(TTY)));
        t.rows.insert(200, (9, 200, Some(TTY)));
        assert_eq!(judge(&t, &saved(), 200), Verdict::NotContinuous);
    }

    /// The pane reports the saved pid and that process is dead. Kills: a dead shell read as a
    /// decline or a match.
    #[test]
    fn a_dead_pid_is_gone() {
        assert_eq!(
            judge(
                &table((START, SHELL, Some(TTY))).without_rows(),
                &saved(),
                SHELL
            ),
            Verdict::Gone
        );
    }

    /// Kills: skipping the start stamp, which accepts a reused pid.
    #[test]
    fn a_reused_pid_is_gone() {
        assert_eq!(
            judge(&table((START + 1, SHELL, Some(TTY))), &saved(), SHELL),
            Verdict::Gone
        );
    }

    /// Kills: dropping the recorded-tty comparison, the one fact `process_continuity` cannot
    /// check for a pane's own process.
    #[test]
    fn another_terminal_is_not_continuous() {
        assert_eq!(
            judge(&table((START, SHELL, Some(TTY + 1))), &saved(), SHELL),
            Verdict::NotContinuous
        );
        assert_eq!(
            judge(&table((START, SHELL, None)), &saved(), SHELL),
            Verdict::NotContinuous
        );
    }

    /// Kills: dropping the session-leader requirement.
    #[test]
    fn a_shell_that_no_longer_leads_its_session_is_not_continuous() {
        assert_eq!(
            judge(&table((START, 999, Some(TTY))), &saved(), SHELL),
            Verdict::NotContinuous
        );
    }

    /// After a reboot a restored pane's new shell can repeat the pid, start tick and terminal.
    /// Kills: judging without the kernel boot id, or taking an unreadable one as a decision.
    #[test]
    fn a_matching_shell_of_another_kernel_boot_is_gone() {
        let mut t = table((START, SHELL, Some(TTY)));
        t.boot = Some("boot-b");
        assert_eq!(judge(&t, &saved(), SHELL), Verdict::Gone);
        t.boot = None;
        assert_eq!(judge(&t, &saved(), SHELL), Verdict::Unknown);
        assert_eq!(
            read(&t, SHELL),
            None,
            "an unreadable boot id records nothing"
        );
    }

    /// Kills: an unreadable fact taken as a decision.
    #[test]
    fn an_unreadable_session_is_unknown() {
        let mut t = table((START, SHELL, Some(TTY)));
        t.no_session = Some(SHELL);
        assert_eq!(judge(&t, &saved(), SHELL), Verdict::Unknown);
    }

    /// Kills: a key that omits the fingerprint, which would decline a new shell along with the
    /// seat's old one, or the generation.
    #[test]
    fn the_decline_key_names_seat_generation_pid_and_start() {
        let candidate = ShellContinuityCandidate {
            seat: SeatId::new("s1"),
            seat_generation: 3,
            target: crate::protocol::ids::HostTargetId::new("w1:p1"),
            fingerprint: saved(),
        };
        assert_eq!(
            decline_key(&candidate),
            (SeatId::new("s1"), 3, SHELL, START)
        );
    }
}
