//! Local, read-only terminal discovery. Directory requests remain bounded pages.
use super::{RunError, commands::ParsedCli, output::Presentation};
use crate::{
    protocol::{
        ids::ThreadId,
        output::OutputFormat,
        pagination::{Page, PageRequest},
        results::{ApiError, ErrorCode, ThreadSummary},
    },
    view::escape::{Context, escape_for_terminal},
};
use std::{
    io::{self, IsTerminal, Read, Write},
    sync::atomic::{AtomicBool, Ordering},
};
static INTERRUPTED: AtomicBool = AtomicBool::new(false);
extern "C" fn on_interrupt(_: libc::c_int) {
    INTERRUPTED.store(true, Ordering::SeqCst);
}

pub(crate) fn require_terminal(parsed: &ParsedCli) -> Result<(), ApiError> {
    eligible(
        [
            io::stdin().is_terminal(),
            io::stdout().is_terminal(),
            io::stderr().is_terminal(),
        ],
        std::env::var("TERM").ok().as_deref(),
        parsed.presentation,
        parsed.output.format,
        parsed.cooperative.is_some()
            || super::agent_env_marker(
                std::env::vars_os().map(|(k, v)| (k.to_string_lossy().into_owned(), v)),
            )
            .is_some()
            || super::output::harness_marked(|key| std::env::var_os(key)),
    )
}
fn eligible(
    ttys: [bool; 3],
    term: Option<&str>,
    presentation: Presentation,
    format: OutputFormat,
    agent: bool,
) -> Result<(), ApiError> {
    if !ttys.into_iter().all(|tty| tty)
        || term.is_none_or(|term| term.is_empty() || term == "dumb")
        || presentation == Presentation::Machine
        || format != OutputFormat::Text
        || agent
    {
        return Err(ApiError::invalid_request(
            "bare read requires a human terminal (stdin, stdout and stderr); use read THREAD or thread list --recent --all",
        ));
    }
    Ok(())
}
fn matches(query: &str, row: &ThreadSummary) -> bool {
    let text = format!("{} {}", row.name.as_deref().unwrap_or(""), row.topic_data).to_lowercase();
    let mut chars = text.chars();
    query
        .to_lowercase()
        .chars()
        .all(|wanted| chars.by_ref().any(|got| got == wanted))
}
fn safe(text: &str, max: usize) -> String {
    escape_for_terminal(
        &text.chars().take(max).collect::<String>(),
        Context::SingleLine,
    )
    .into_owned()
}

/// Restores termios and the display on every ordinary exit, including I/O errors.
struct Terminal {
    saved: libc::termios,
    signals: Vec<(libc::c_int, libc::sigaction)>,
}
impl Terminal {
    fn enter() -> io::Result<Self> {
        // SAFETY: termios is initialized by tcgetattr on the owned stdin descriptor.
        let mut saved = unsafe { std::mem::zeroed() };
        if unsafe { libc::tcgetattr(0, &mut saved) } != 0 {
            return Err(io::Error::last_os_error());
        }
        let mut raw = saved;
        raw.c_lflag &= !(libc::ICANON | libc::ECHO | libc::ISIG);
        raw.c_iflag &= !(libc::IXON | libc::ICRNL);
        raw.c_cc[libc::VMIN] = 0;
        raw.c_cc[libc::VTIME] = 0;
        if unsafe { libc::tcsetattr(0, libc::TCSANOW, &raw) } != 0 {
            return Err(io::Error::last_os_error());
        }
        let mut guard = Self {
            saved,
            signals: Vec::new(),
        };
        INTERRUPTED.store(false, Ordering::SeqCst);
        for signal in [libc::SIGINT, libc::SIGTERM] {
            // SAFETY: initialized sigactions have an empty mask; handler only stores an atomic.
            let mut action: libc::sigaction = unsafe { std::mem::zeroed() };
            let mut previous = unsafe { std::mem::zeroed() };
            action.sa_sigaction = on_interrupt as *const () as libc::sighandler_t;
            unsafe {
                libc::sigemptyset(&mut action.sa_mask);
            }
            if unsafe { libc::sigaction(signal, &action, &mut previous) } != 0 {
                return Err(io::Error::last_os_error());
            }
            guard.signals.push((signal, previous));
        }
        let mut stderr = io::stderr().lock();
        stderr.write_all(b"\x1b[?1049h\x1b[?25l")?;
        stderr.flush()?;
        Ok(guard)
    }
}
impl Drop for Terminal {
    fn drop(&mut self) {
        // SAFETY: saved termios was read from this descriptor before changing it.
        unsafe {
            libc::tcsetattr(0, libc::TCSANOW, &self.saved);
            for (signal, previous) in &self.signals {
                libc::sigaction(*signal, previous, std::ptr::null_mut());
            }
        }
        let mut stderr = io::stderr().lock();
        let _ = stderr.write_all(b"\x1b[0m\x1b[?25h\x1b[?1049l");
        let _ = stderr.flush();
    }
}

struct Model {
    rows: Vec<ThreadSummary>,
    query: String,
    selected: usize,
    cursor: Option<String>,
    done: bool,
}
impl Model {
    fn filtered(&self) -> Vec<usize> {
        self.rows
            .iter()
            .enumerate()
            .filter_map(|(i, r)| matches(&self.query, r).then_some(i))
            .collect()
    }
    fn move_by(&mut self, delta: isize) {
        let len = self.filtered().len();
        self.selected = if len == 0 {
            0
        } else {
            self.selected.saturating_add_signed(delta).min(len - 1)
        };
    }
    fn selected(&self) -> Option<ThreadId> {
        self.filtered()
            .get(self.selected)
            .map(|&i| self.rows[i].thread.clone())
    }
    fn render(&self, out: &mut dyn Write) -> io::Result<()> {
        write!(
            out,
            "\x1b[H\x1b[2JRecent channels — type to filter; ↑/↓ Ctrl-P/N; Enter read; Esc cancel\r\n> {}\r\n",
            safe(&self.query, 256)
        )?;
        let filtered = self.filtered();
        let first = self.selected.saturating_sub(10);
        for (position, &i) in filtered.iter().enumerate().skip(first).take(20) {
            let row = &self.rows[i];
            write!(
                out,
                "{} {} {}{} [{}] — {}\r\n",
                if position == self.selected { ">" } else { " " },
                safe(row.thread.as_str(), 80),
                safe(row.name.as_deref().unwrap_or("unnamed"), 128),
                if row.archived { " [archived]" } else { "" },
                safe(
                    &super::human::timestamp(row.last_activity.unwrap_or(row.created_at)),
                    32,
                ),
                safe(&row.topic_data, 120)
            )?;
        }
        write!(
            out,
            "{} matches / {} loaded — {}\r\n",
            filtered.len(),
            self.rows.len(),
            if self.done {
                "complete"
            } else {
                "loading more…"
            }
        )?;
        out.flush()
    }
}

pub(crate) fn run(
    mut fetch: impl FnMut(PageRequest) -> Result<Page<ThreadSummary>, ApiError>,
) -> Result<Option<ThreadId>, RunError> {
    let terminal = Terminal::enter()?;
    let mut model = Model {
        rows: Vec::new(),
        query: String::new(),
        selected: 0,
        cursor: None,
        done: false,
    };
    let mut input = io::stdin().lock();
    let mut output = io::stderr().lock();
    let mut pending = Vec::new();
    loop {
        if INTERRUPTED.load(Ordering::SeqCst) {
            return Ok(None);
        }
        model.render(&mut output)?;
        if !model.done {
            match fetch(PageRequest {
                cursor: model.cursor.clone(),
                limit: 100,
                max_bytes: 65536,
            }) {
                Ok(page) => {
                    model.rows.extend(page.items);
                    model.cursor = page.next_cursor;
                    model.done = !page.has_more;
                    if !model.done && model.cursor.is_none() {
                        return Err(
                            ApiError::store_corrupt("picker page lacks continuation").into()
                        );
                    }
                    model.move_by(0);
                }
                Err(error) if error.code == ErrorCode::CursorStale => {
                    model.rows.clear();
                    model.cursor = None;
                    model.selected = 0;
                }
                Err(error) => return Err(error.into()),
            }
            model.render(&mut output)?;
        }
        if model.done && model.rows.is_empty() {
            if INTERRUPTED.load(Ordering::SeqCst) {
                return Ok(None);
            }
            // Leave the alternate screen before the notice so it survives the
            // picker. Cancellation paths stay quiet and never write stdout.
            drop(output);
            drop(input);
            drop(terminal);
            let mut output = io::stderr().lock();
            writeln!(output, "No channels.")?;
            output.flush()?;
            return Ok(None);
        }
        let mut fd = libc::pollfd {
            fd: 0,
            events: libc::POLLIN,
            revents: 0,
        };
        // SAFETY: poll receives a valid single owned stack descriptor.
        let polled = unsafe { libc::poll(&mut fd, 1, 100) };
        if polled < 0 {
            let error = io::Error::last_os_error();
            if error.kind() == io::ErrorKind::Interrupted {
                continue;
            }
            return Err(error.into());
        }
        if polled == 0 {
            continue;
        }
        let mut bytes = [0u8; 128];
        let read = input.read(&mut bytes)?;
        if read == 0 {
            return Ok(None);
        }
        pending.extend_from_slice(&bytes[..read]);
        while !pending.is_empty() {
            match pending[0] {
                3 => return Ok(None),
                27 => {
                    if pending.len() < 3 {
                        let mut fd = libc::pollfd {
                            fd: 0,
                            events: libc::POLLIN,
                            revents: 0,
                        };
                        // SAFETY: poll receives one valid stack descriptor.
                        if unsafe { libc::poll(&mut fd, 1, 30) } > 0 {
                            let count = input.read(&mut bytes)?;
                            pending.extend_from_slice(&bytes[..count]);
                        }
                    }
                    if pending.starts_with(b"\x1b[A") {
                        model.move_by(-1);
                        pending.drain(..3);
                    } else if pending.starts_with(b"\x1b[B") {
                        model.move_by(1);
                        pending.drain(..3);
                    } else {
                        return Ok(None);
                    }
                }
                14 => {
                    model.move_by(1);
                    pending.remove(0);
                }
                16 => {
                    model.move_by(-1);
                    pending.remove(0);
                }
                10 | 13 => {
                    pending.remove(0);
                    if let Some(thread) = model.selected() {
                        return Ok(Some(thread));
                    }
                    if model.done {
                        return Ok(None);
                    }
                }
                8 | 127 => {
                    model.query.pop();
                    model.selected = 0;
                    pending.remove(0);
                }
                byte if byte < 32 => {
                    pending.remove(0);
                }
                _ => {
                    let width = if pending[0] < 128 {
                        1
                    } else if pending[0] < 224 {
                        2
                    } else if pending[0] < 240 {
                        3
                    } else {
                        4
                    };
                    if pending.len() < width {
                        break;
                    }
                    if let Ok(text) = std::str::from_utf8(&pending[..width])
                        && model.query.len() + width <= 256
                    {
                        model.query.push_str(text);
                        model.selected = 0;
                    }
                    pending.drain(..width);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::commands::{CliAction, parse_argv};
    fn row(id: &str, name: &str, topic: &str) -> ThreadSummary {
        ThreadSummary {
            name: Some(name.into()),
            last_activity: None,
            thread: ThreadId::new(id),
            managed_owner: None,
            topic_data: topic.into(),
            topic_omitted: false,
            topic_detail_argv: None,
            archived: false,
            orphaned: true,
            message_count: 0,
            created_at: crate::protocol::time::UtcMillis(0),
            ordinary_count: 0,
            system_count: 0,
            joined_count: 0,
        }
    }
    #[test]
    fn recent_picker_terminal_boundary_refuses_every_nonhuman_mode() {
        for ttys in [
            [false, true, true],
            [true, false, true],
            [true, true, false],
        ] {
            assert!(
                eligible(
                    ttys,
                    Some("xterm"),
                    Presentation::Human,
                    OutputFormat::Text,
                    false
                )
                .is_err()
            );
        }
        for term in [None, Some(""), Some("dumb")] {
            assert!(
                eligible(
                    [true; 3],
                    term,
                    Presentation::Human,
                    OutputFormat::Text,
                    false
                )
                .is_err()
            );
        }
        assert!(
            eligible(
                [true; 3],
                Some("xterm"),
                Presentation::Machine,
                OutputFormat::Text,
                false
            )
            .is_err()
        );
        assert!(
            eligible(
                [true; 3],
                Some("xterm"),
                Presentation::Human,
                OutputFormat::Json,
                false
            )
            .is_err()
        );
        assert!(
            eligible(
                [true; 3],
                Some("xterm"),
                Presentation::Human,
                OutputFormat::Text,
                true
            )
            .is_err()
        );
        assert!(
            eligible(
                [true; 3],
                Some("xterm"),
                Presentation::Auto,
                OutputFormat::Text,
                false
            )
            .is_ok()
        );
    }
    #[test]
    fn recent_picker_fuzzy_navigation_escaping_and_no_implicit_selection() {
        let mut model = Model {
            rows: vec![
                row("t1", "architecture", "escape\x1b\n"),
                row("t2", "delivery", "worker"),
            ],
            query: "ARc".into(),
            selected: 0,
            cursor: Some("more".into()),
            done: false,
        };
        assert_eq!(model.selected(), Some(ThreadId::new("t1")));
        model.query = "t2".into();
        assert!(
            model.selected().is_none(),
            "canonical IDs are not fuzzy search fields"
        );
        model.query = "xyz".into();
        model.move_by(1);
        assert!(model.selected().is_none());
        // A page without matches remains incomplete; later matching rows are selectable.
        assert!(!model.done);
        model.rows.push(row("t3", "xyz", "late page"));
        assert_eq!(model.selected(), Some(ThreadId::new("t3")));
        model.query.clear();
        model.move_by(1);
        assert_eq!(model.selected(), Some(ThreadId::new("t2")));
        model.move_by(-1);
        assert_eq!(model.selected(), Some(ThreadId::new("t1")));
        let mut out = Vec::new();
        model.render(&mut out).unwrap();
        let shown = String::from_utf8(out).unwrap();
        assert!(!shown.contains("escape\x1b\n"));
        assert!(shown.contains("loading more"));
        assert!(
            safe("\x1b[2J\n\u{202e}", 100)
                .chars()
                .all(|c| c != '\x1b' && c != '\n' && c != '\u{202e}')
        );
    }
    #[test]
    fn recent_picker_read_options_select_only_canonical_read_actions() {
        let parsed = parse_argv(["ht", "read"]).unwrap();
        assert!(parsed.thread_selector.is_none());
        let CliAction::Picker(request) = parsed.action else {
            panic!("picker")
        };
        let CliAction::Wire(crate::protocol::commands::Command::History(history)) =
            request.selected(ThreadId::new("tExact"))
        else {
            panic!("history")
        };
        assert_eq!(history.thread.as_str(), "tExact");
        assert_eq!(
            history.initial,
            Some(crate::protocol::commands::HistoryRange::Recent { count: 20 })
        );
        assert!(parse_argv(["ht", "read", "--cursor", "c3:any"]).is_err());
        let parsed =
            parse_argv(["ht", "read", "--follow", "--after", "55", "--no-system"]).unwrap();
        let CliAction::Picker(request) = parsed.action else {
            panic!("picker")
        };
        let CliAction::Follow(follow) = request.selected(ThreadId::new("tExact")) else {
            panic!("follow")
        };
        assert_eq!(follow.after, Some(55));
        assert_eq!(follow.recent, 0);
        assert!(follow.no_system);
    }
}
