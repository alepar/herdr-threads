//! Local, read-only terminal discovery. Directory requests remain bounded pages.
use super::{RunError, commands::ParsedCli, output::Presentation};
use crate::{
    protocol::{
        ids::ThreadId,
        output::OutputFormat,
        pagination::PageRequest,
        results::{ApiError, ErrorCode, MessageKind, PickerPage, PickerThread},
        time::Cancellation,
    },
    view::escape::{Context, escape_for_terminal},
};
use std::{
    cell::RefCell,
    collections::HashMap,
    io::{self, IsTerminal, Read, Write},
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    time::{Duration, Instant},
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
            "bare read/follow requires a human terminal (stdin, stdout and stderr); use read THREAD, follow THREAD or thread list --recent --all",
        ));
    }
    Ok(())
}
fn matches(query: &str, row: &PickerThread) -> bool {
    let text = format!("{} {}", row.name.as_deref().unwrap_or(""), row.topic_data).to_lowercase();
    let mut chars = text.chars();
    query
        .to_lowercase()
        .chars()
        .all(|wanted| chars.by_ref().any(|got| got == wanted))
}
fn safe(text: &str, max: usize) -> String {
    let escaped = escape_for_terminal(text, Context::SingleLine);
    if unicode_width::UnicodeWidthStr::width(escaped.as_ref()) <= max {
        return escaped.into_owned();
    }
    let mut end = 0;
    for (index, ch) in escaped.char_indices() {
        let next = index + ch.len_utf8();
        // Sequence width matters: emoji variation selectors and keycaps can
        // widen a preceding character without occupying cells themselves.
        if unicode_width::UnicodeWidthStr::width(&escaped[..next]) > max {
            break;
        }
        end = next;
    }
    escaped[..end].to_owned()
}

#[derive(Clone, Copy, PartialEq, Eq)]
struct Size {
    width: usize,
    height: usize,
}
fn terminal_size() -> Size {
    // SAFETY: ioctl initializes the owned stack winsize using the stderr TTY.
    let mut size: libc::winsize = unsafe { std::mem::zeroed() };
    if unsafe { libc::ioctl(2, libc::TIOCGWINSZ, &mut size) } == 0 {
        Size {
            width: usize::from(size.ws_col),
            height: usize::from(size.ws_row),
        }
    } else {
        Size {
            width: 80,
            height: 24,
        }
    }
}
#[derive(Default)]
struct Renderer {
    previous: Vec<String>,
    size: Option<Size>,
}
impl Renderer {
    fn render(&mut self, lines: Vec<String>, size: Size, out: &mut dyn Write) -> io::Result<()> {
        if size.width <= 1 || size.height <= 1 {
            self.size = Some(size);
            return Ok(());
        }
        let resized = self.size != Some(size);
        let mut delta = Vec::new();
        for index in 0..self
            .previous
            .len()
            .max(lines.len())
            .min(size.height.saturating_sub(1))
        {
            let line = lines.get(index).map(String::as_str).unwrap_or("");
            if resized || self.previous.get(index).map(String::as_str) != Some(line) {
                write!(delta, "\x1b[{};1H\x1b[2K{}", index + 1, line)?;
            }
        }
        if !delta.is_empty() {
            out.write_all(&delta)?;
            out.flush()?;
        }
        self.size = Some(size);
        self.previous = lines;
        Ok(())
    }
}

fn color_enabled(term: Option<&str>, no_color: bool) -> bool {
    !no_color
        && term.is_some_and(|term| {
            [
                "xterm",
                "screen",
                "tmux",
                "rxvt",
                "alacritty",
                "wezterm",
                "foot",
                "konsole",
                "st",
            ]
            .into_iter()
            .any(|prefix| {
                term == prefix
                    || term
                        .strip_prefix(prefix)
                        .is_some_and(|suffix| suffix.starts_with('-'))
            }) || matches!(term, "linux" | "ansi")
        })
}
fn process_color() -> bool {
    color_enabled(
        std::env::var("TERM").ok().as_deref(),
        std::env::var_os("NO_COLOR").is_some(),
    )
}

// Split the rational score into an integer and a remainder. Only the latter
// needs cross multiplication; each factor fits u64, avoiding 192-bit products.
fn score(row: &PickerThread) -> (u128, u64, u64) {
    let window = row.activity_window_ms.max(60_000);
    let activity =
        u128::from(row.participant_count) * u128::from(row.recent_ordinary_count.min(512)) * 60_000;
    (
        u128::from(row.participant_count) + activity / u128::from(window),
        (activity % u128::from(window)) as u64,
        window,
    )
}
fn compare_rows(a: &PickerThread, b: &PickerThread) -> std::cmp::Ordering {
    let (a_integer, a_remainder, a_window) = score(a);
    let (b_integer, b_remainder, b_window) = score(b);
    a.archived
        .cmp(&b.archived)
        .then_with(|| b_integer.cmp(&a_integer))
        .then_with(|| {
            (u128::from(b_remainder) * u128::from(a_window))
                .cmp(&(u128::from(a_remainder) * u128::from(b_window)))
        })
        .then_with(|| b.participant_count.cmp(&a.participant_count))
        .then_with(|| b.last_activity.cmp(&a.last_activity))
        .then_with(|| a.thread.as_str().cmp(b.thread.as_str()))
}

/// Restores termios and the display on every ordinary exit, including I/O errors.
struct Terminal {
    saved: libc::termios,
    signals: Vec<(libc::c_int, libc::sigaction)>,
    color: bool,
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
            color: process_color(),
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
        if self.color {
            let _ = stderr.write_all(b"\x1b[0m");
        }
        let _ = stderr.write_all(b"\x1b[?25h\x1b[?1049l");
        let _ = stderr.flush();
    }
}

struct Model {
    rows: Vec<PickerThread>,
    query: String,
    selected: usize,
    selected_id: Option<ThreadId>,
    done: bool,
    renderer: RefCell<Renderer>,
}
impl Model {
    fn filtered(&self) -> Vec<usize> {
        let mut rows: Vec<_> = self
            .rows
            .iter()
            .enumerate()
            .filter_map(|(i, row)| matches(&self.query, row).then_some(i))
            .collect();
        rows.sort_by(|&a, &b| compare_rows(&self.rows[a], &self.rows[b]));
        rows
    }
    fn position(&self, filtered: &[usize]) -> usize {
        self.selected_id
            .as_ref()
            .and_then(|id| filtered.iter().position(|&i| self.rows[i].thread == *id))
            .unwrap_or(self.selected.min(filtered.len().saturating_sub(1)))
    }
    fn move_by(&mut self, delta: isize) {
        let filtered = self.filtered();
        self.selected = self
            .position(&filtered)
            .saturating_add_signed(delta)
            .min(filtered.len().saturating_sub(1));
        if let Some(&i) = filtered.get(self.selected) {
            self.selected_id = Some(self.rows[i].thread.clone());
        }
    }
    fn selected(&self) -> Option<ThreadId> {
        let filtered = self.filtered();
        filtered
            .get(self.position(&filtered))
            .map(|&i| self.rows[i].thread.clone())
    }
    fn replace(&mut self, rows: Vec<PickerThread>) {
        self.selected_id = self.selected();
        self.rows = rows;
        self.move_by(0);
    }
    fn extend(&mut self, rows: &[PickerThread]) {
        self.selected_id = self.selected();
        let mut indices: HashMap<_, _> = self
            .rows
            .iter()
            .enumerate()
            .map(|(index, row)| (row.thread.clone(), index))
            .collect();
        for row in rows {
            if let Some(&index) = indices.get(&row.thread) {
                self.rows[index] = row.clone();
            } else {
                indices.insert(row.thread.clone(), self.rows.len());
                self.rows.push(row.clone());
            }
        }
        self.move_by(0);
    }
    fn frame(&self, size: Size, color: bool) -> Vec<String> {
        let width = size.width.saturating_sub(1);
        let height = size.height.saturating_sub(1);
        if width == 0 || height == 0 {
            return Vec::new();
        }
        let mut lines = vec![
            safe(
                "Channels — type to filter; ↑/↓ Ctrl-P/N; Enter read; Esc cancel",
                width,
            ),
            safe(&format!("> {}", self.query), width),
        ];
        let filtered = self.filtered();
        let selected = self.position(&filtered);
        let capacity = height.saturating_sub(3) / 2;
        let first = selected
            .saturating_sub(capacity / 2)
            .min(filtered.len().saturating_sub(capacity));
        for (position, &i) in filtered.iter().enumerate().skip(first).take(capacity) {
            let row = &self.rows[i];
            let rate = u128::from(row.recent_ordinary_count.min(512)) * 600_000
                / u128::from(row.activity_window_ms.max(60_000));
            let text = safe(
                &format!(
                    "{} [{}] {} · {} participants · {}.{}/min · {}",
                    if position == selected { ">" } else { " " },
                    if row.archived { "archived" } else { "active" },
                    safe(
                        row.name.as_deref().unwrap_or("unnamed"),
                        (width / 3).min(24)
                    ),
                    row.participant_count,
                    rate / 10,
                    rate % 10,
                    row.thread.as_str()
                ),
                width,
            );
            let latest = match &row.last_message {
                None => "  latest: no messages".to_owned(),
                Some(message) => {
                    let escaped = escape_for_terminal(&message.preview_data, Context::SingleLine);
                    let preview_width = (width / 3).min(32);
                    let clipped =
                        unicode_width::UnicodeWidthStr::width(escaped.as_ref()) > preview_width;
                    format!(
                        "  latest [{} #{}]: {}{}",
                        match message.kind {
                            MessageKind::Ordinary => "ordinary",
                            MessageKind::Info => "info",
                            MessageKind::Warn => "warn",
                        },
                        message.sequence,
                        safe(&message.preview_data, preview_width),
                        if message.preview_omitted || clipped {
                            " …"
                        } else {
                            ""
                        }
                    )
                }
            };
            let style = if !color {
                ""
            } else if row.archived {
                "\x1b[2m"
            } else if row.participant_count >= 3 {
                "\x1b[1;36m"
            } else {
                "\x1b[32m"
            };
            let selection = if color && position == selected {
                "\x1b[7m"
            } else {
                ""
            };
            let reset = if color { "\x1b[0m" } else { "" };
            lines.push(format!("{style}{selection}{text}{reset}"));
            lines.push(format!(
                "{style}{selection}{}{reset}",
                safe(&format!("{latest} · topic:{}", row.topic_data), width)
            ));
        }
        lines.push(safe(
            &format!(
                "{} matches / {} loaded — {}",
                filtered.len(),
                self.rows.len(),
                if self.done {
                    "complete"
                } else {
                    "loading more…"
                }
            ),
            width,
        ));
        lines.truncate(height);
        lines
    }
    fn render(&self, out: &mut dyn Write) -> io::Result<()> {
        let size = terminal_size();
        self.renderer
            .borrow_mut()
            .render(self.frame(size, process_color()), size, out)
    }
}

struct Worker {
    requests: mpsc::Sender<Option<(PageRequest, bool)>>,
    results: mpsc::Receiver<Result<PickerPage, ApiError>>,
    cancellation: Cancellation,
}
impl Drop for Worker {
    fn drop(&mut self) {
        self.cancellation.cancel();
        let _ = self.requests.send(None);
    }
}
#[derive(Default)]
struct Scan {
    rows: Vec<PickerThread>,
    cursor: Option<String>,
    restarts: usize,
}
impl Scan {
    fn accept(&mut self, page: PickerPage, model: &mut Model) -> Result<bool, ApiError> {
        page.validate().map_err(ApiError::store_corrupt)?;
        if !model.done {
            model.extend(&page.items);
        }
        self.rows.extend(page.items);
        self.cursor = page.next_cursor;
        if !page.has_more {
            model.replace(std::mem::take(&mut self.rows));
            model.done = true;
            return Ok(true);
        }
        Ok(false)
    }
}

pub(crate) fn run(
    mut fetch: impl FnMut(PageRequest, bool, Cancellation) -> Result<PickerPage, ApiError> + Send,
) -> Result<Option<ThreadId>, RunError> {
    let terminal = Terminal::enter()?;
    std::thread::scope(|scope| {
        let (requests, request_rx) = mpsc::channel();
        let (result_tx, results) = mpsc::channel();
        let cancellation = Cancellation::default();
        let worker_cancellation = cancellation.clone();
        scope.spawn(move || {
            while let Ok(Some((page, refresh))) = request_rx.recv() {
                let result = fetch(page, refresh, worker_cancellation.clone());
                if result_tx.send(result).is_err() {
                    break;
                }
            }
        });
        let worker = Worker {
            requests,
            results,
            cancellation,
        };
        run_loop(terminal, worker)
    })
}
fn run_loop(terminal: Terminal, worker: Worker) -> Result<Option<ThreadId>, RunError> {
    let mut model = Model {
        rows: Vec::new(),
        query: String::new(),
        selected: 0,
        selected_id: None,
        done: false,
        renderer: RefCell::default(),
    };
    let mut scan = Scan::default();
    let mut active = true;
    let mut in_flight = false;
    let mut next_page = Instant::now();
    let mut next_refresh = Instant::now() + Duration::from_secs(5);
    let mut input = io::stdin().lock();
    let mut output = io::stderr().lock();
    let mut pending = Vec::new();
    let mut dirty = true;
    let mut last_size = terminal_size();
    loop {
        if INTERRUPTED.load(Ordering::SeqCst) {
            return Ok(None);
        }
        let size = terminal_size();
        if dirty || size != last_size {
            model.render(&mut output)?;
            dirty = false;
            last_size = size;
        }
        match worker.results.try_recv() {
            Ok(result) => {
                in_flight = false;
                if INTERRUPTED.load(Ordering::SeqCst) {
                    return Ok(None);
                }
                match result {
                    Ok(page) => {
                        if scan.accept(page, &mut model)? {
                            active = false;
                            next_refresh = Instant::now() + Duration::from_secs(5);
                        }
                        next_page = Instant::now() + Duration::from_millis(100);
                    }
                    Err(error) if error.code == ErrorCode::CursorStale && scan.restarts < 3 => {
                        scan.rows.clear();
                        scan.cursor = None;
                        scan.restarts += 1;
                        next_page = Instant::now() + Duration::from_millis(100);
                    }
                    Err(_) if model.done => {
                        active = false;
                        next_refresh = Instant::now() + Duration::from_secs(5);
                    }
                    Err(error) => return Err(error.into()),
                }
                model.render(&mut output)?;
            }
            Err(mpsc::TryRecvError::Empty) => {}
            Err(mpsc::TryRecvError::Disconnected) => {
                return Err(ApiError::store_corrupt("picker worker stopped").into());
            }
        }
        if !active && Instant::now() >= next_refresh {
            scan = Scan::default();
            active = true;
            next_page = Instant::now();
        }
        if active && !in_flight && Instant::now() >= next_page {
            worker
                .requests
                .send(Some((
                    PageRequest {
                        cursor: scan.cursor.clone(),
                        limit: 100,
                        max_bytes: 65536,
                    },
                    model.done,
                )))
                .map_err(|_| ApiError::store_corrupt("picker worker stopped"))?;
            in_flight = true;
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
        dirty = true;
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
    fn row(id: &str, name: &str, topic: &str) -> PickerThread {
        PickerThread {
            thread: ThreadId::new(id),
            name: Some(name.into()),
            topic_data: topic.into(),
            archived: false,
            participant_count: 0,
            last_activity: crate::protocol::time::UtcMillis(0),
            recent_ordinary_count: 0,
            activity_window_ms: 60_000,
            sample_positions: 0,
            last_message: None,
        }
    }
    #[test]
    fn recent_picker_escapes_before_clipping_to_terminal_cells() {
        assert_eq!(safe("界界界", 4), "界界");
        assert_eq!(safe("e\u{301}x", 1), "e\u{301}");
        assert!(unicode_width::UnicodeWidthStr::width(safe("1\u{fe0f}\u{20e3}", 1).as_str()) <= 1);
        assert!(unicode_width::UnicodeWidthStr::width(safe("❤\u{fe0f}", 1).as_str()) <= 1);
        assert!(unicode_width::UnicodeWidthStr::width(safe("\x1b\n界", 5).as_str()) <= 5);
    }
    #[test]
    fn recent_picker_updates_without_clearing_the_whole_screen() {
        let model = Model {
            rows: vec![row("t1", "alpha", "topic")],
            query: String::new(),
            selected: 0,
            selected_id: None,
            done: true,
            renderer: RefCell::default(),
        };
        let mut out = Vec::new();
        model.render(&mut out).unwrap();
        assert!(
            !out.windows(4).any(|bytes| bytes == b"\x1b[2J"),
            "whole-screen erase causes visible flicker"
        );
    }
    #[test]
    fn recent_picker_identical_idle_frame_writes_no_bytes() {
        let model = Model {
            rows: vec![row("t1", "alpha", "topic")],
            query: String::new(),
            selected: 0,
            selected_id: None,
            done: true,
            renderer: RefCell::default(),
        };
        let mut out = Vec::new();
        model.render(&mut out).unwrap();
        out.clear();
        model.render(&mut out).unwrap();
        assert!(out.is_empty(), "an idle picker must not redraw");
    }
    #[test]
    fn recent_picker_preserves_exact_selection_when_earlier_rows_arrive() {
        let mut model = Model {
            rows: vec![row("t1", "alpha", "topic"), row("t2", "beta", "topic")],
            query: String::new(),
            selected: 0,
            selected_id: None,
            done: true,
            renderer: RefCell::default(),
        };
        model.move_by(1);
        assert_eq!(model.selected(), Some(ThreadId::new("t2")));
        model.rows.insert(0, row("t0", "new", "topic"));
        model.move_by(0);
        assert_eq!(model.selected(), Some(ThreadId::new("t2")));
    }
    #[test]
    fn recent_picker_rows_include_readable_status_participants_rate_and_latest() {
        let model = Model {
            rows: vec![row("t1", "alpha", "topic")],
            query: String::new(),
            selected: 0,
            selected_id: None,
            done: true,
            renderer: RefCell::default(),
        };
        let mut out = Vec::new();
        model.render(&mut out).unwrap();
        let shown = String::from_utf8(out).unwrap();
        assert!(shown.contains("[active]"));
        assert!(shown.contains("0 participants"));
        assert!(shown.contains("0.0/min"));
        assert!(shown.contains("latest:"));
    }
    #[test]
    fn recent_picker_ranks_active_weighted_participation_and_deterministic_ties() {
        let mut archived = row("archive", "archived", "");
        archived.archived = true;
        archived.participant_count = 100;
        let mut busy = row("busy", "busy", "");
        busy.participant_count = 2;
        busy.recent_ordinary_count = 4;
        let mut many = row("many", "many", "");
        many.participant_count = 4;
        many.recent_ordinary_count = 1;
        let mut ties = row("ties", "ties", "");
        ties.participant_count = 8;
        let mut newer = ties.clone();
        newer.thread = ThreadId::new("newer");
        newer.last_activity = crate::protocol::time::UtcMillis(1);
        let mut canonical = newer.clone();
        canonical.thread = ThreadId::new("aaa");
        let model = Model {
            rows: vec![archived, ties, many, newer, canonical, busy],
            query: String::new(),
            selected: 0,
            selected_id: None,
            done: true,
            renderer: RefCell::default(),
        };
        let ordered: Vec<_> = model
            .filtered()
            .iter()
            .map(|&i| model.rows[i].thread.as_str())
            .collect();
        assert_eq!(ordered, ["busy", "aaa", "newer", "ties", "many", "archive"]);
    }
    #[test]
    fn recent_picker_ranking_keeps_exact_fractional_scores_at_u64_bounds() {
        let mut lower = row("a", "a", "");
        lower.participant_count = u64::MAX;
        lower.recent_ordinary_count = 512;
        lower.activity_window_ms = u64::MAX;
        let mut higher = lower.clone();
        higher.thread = ThreadId::new("z");
        higher.activity_window_ms = u64::MAX - 1;
        let model = Model {
            rows: vec![lower, higher],
            query: String::new(),
            selected: 0,
            selected_id: None,
            done: true,
            renderer: RefCell::default(),
        };
        assert_eq!(model.selected(), Some(ThreadId::new("z")));
    }
    #[test]
    fn recent_picker_color_plain_and_small_frames_keep_readable_metadata() {
        let mut active = row("a", "active", "");
        active.participant_count = 1;
        let mut popular = row("b", "popular", "");
        popular.participant_count = 3;
        let mut archived = row("c", "archived", "");
        archived.archived = true;
        let model = Model {
            rows: vec![active, popular, archived],
            query: "".into(),
            selected: 0,
            selected_id: None,
            done: true,
            renderer: RefCell::default(),
        };
        let colored = model
            .frame(
                Size {
                    width: 120,
                    height: 20,
                },
                true,
            )
            .join("\n");
        assert!(colored.contains("\x1b[32m"));
        assert!(colored.contains("\x1b[1;36m"));
        assert!(colored.contains("\x1b[2m"));
        assert!(colored.contains("\x1b[7m"));
        let plain = model
            .frame(
                Size {
                    width: 120,
                    height: 20,
                },
                false,
            )
            .join("\n");
        assert!(!plain.contains('\x1b'));
        assert!(plain.contains("[active]"));
        assert!(plain.contains("[archived]"));
        assert!(plain.contains("3 participants"));
        assert!(plain.contains("> [active]"));
        for (width, height) in [(0, 0), (0, 10), (10, 0), (1, 1), (2, 2), (5, 3), (12, 8)] {
            let frame = model.frame(Size { width, height }, false);
            assert!(frame.len() <= height.saturating_sub(1));
            assert!(
                frame
                    .iter()
                    .all(|line| unicode_width::UnicodeWidthStr::width(line.as_str()) < width)
            );
        }
    }
    #[test]
    fn recent_picker_fuzzy_matches_full_topic_beyond_preview_bounds() {
        let model = Model {
            rows: vec![row(
                "a",
                "name",
                &format!("{} suffix-marker", "x".repeat(500)),
            )],
            query: "suffix-marker".into(),
            selected: 0,
            selected_id: None,
            done: true,
            renderer: RefCell::default(),
        };
        assert_eq!(model.selected(), Some(ThreadId::new("a")));
    }
    #[test]
    fn recent_picker_batches_changed_lines_and_flushes_once() {
        #[derive(Default)]
        struct Sink {
            bytes: Vec<u8>,
            writes: usize,
            flushes: usize,
        }
        impl Write for Sink {
            fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
                self.writes += 1;
                self.bytes.extend_from_slice(bytes);
                Ok(bytes.len())
            }
            fn flush(&mut self) -> io::Result<()> {
                self.flushes += 1;
                Ok(())
            }
        }
        let mut renderer = Renderer::default();
        let mut sink = Sink::default();
        let size = Size {
            width: 80,
            height: 24,
        };
        renderer
            .render(vec!["first".into(), "second".into()], size, &mut sink)
            .unwrap();
        assert_eq!(sink.writes, 1, "present the complete delta as one write");
        assert_eq!(sink.flushes, 1);
        sink = Sink::default();
        renderer
            .render(vec!["first".into(), "changed".into()], size, &mut sink)
            .unwrap();
        assert_eq!(sink.writes, 1);
        assert_eq!(sink.flushes, 1);
        assert_eq!(sink.bytes, b"\x1b[2;1H\x1b[2Kchanged");
        sink = Sink::default();
        renderer
            .render(vec!["first".into(), "changed".into()], size, &mut sink)
            .unwrap();
        assert_eq!(sink.writes, 0);
        assert_eq!(sink.flushes, 0);
    }
    #[test]
    fn recent_picker_color_requires_known_support_and_honors_no_color() {
        for term in [
            "xterm",
            "xterm-256color",
            "screen",
            "screen-256color",
            "tmux-256color",
            "rxvt-unicode",
            "linux",
            "ansi",
        ] {
            assert!(color_enabled(Some(term), false), "{term}");
            assert!(!color_enabled(Some(term), true), "NO_COLOR: {term}");
        }
        for term in [None, Some(""), Some("dumb"), Some("vt100"), Some("unknown")] {
            assert!(!color_enabled(term, false));
        }
    }
    #[test]
    fn recent_picker_rejects_broken_page_continuations() {
        let mut model = Model {
            rows: vec![],
            query: String::new(),
            selected: 0,
            selected_id: None,
            done: false,
            renderer: RefCell::default(),
        };
        let mut scan = Scan::default();
        let page = PickerPage {
            items: vec![row("a", "a", "")],
            next_cursor: Some("unexpected".into()),
            high_water_ordinal: 1,
            scope_revision: None,
            has_more: false,
            stop_reason: crate::protocol::pagination::StopReason::Complete,
            consistency: crate::protocol::pagination::Consistency::BoundedLive,
        };
        assert!(scan.accept(page, &mut model).is_err());
        assert!(model.rows.is_empty());
    }
    #[test]
    fn recent_picker_resize_repaints_even_when_short_text_is_unchanged() {
        let mut renderer = Renderer::default();
        let mut out = Vec::new();
        renderer
            .render(
                vec!["short".into()],
                Size {
                    width: 80,
                    height: 24,
                },
                &mut out,
            )
            .unwrap();
        out.clear();
        renderer
            .render(
                vec!["short".into()],
                Size {
                    width: 70,
                    height: 24,
                },
                &mut out,
            )
            .unwrap();
        assert_eq!(out, b"\x1b[1;1H\x1b[2Kshort");
        out.clear();
        renderer
            .render(
                vec![],
                Size {
                    width: 0,
                    height: 0,
                },
                &mut out,
            )
            .unwrap();
        assert!(out.is_empty());
        renderer
            .render(
                vec!["short".into()],
                Size {
                    width: 70,
                    height: 24,
                },
                &mut out,
            )
            .unwrap();
        assert_eq!(out, b"\x1b[1;1H\x1b[2Kshort");
    }
    #[test]
    fn recent_picker_refresh_stages_pages_and_preserves_selection_after_restart() {
        let mut model = Model {
            rows: vec![row("a", "alpha", ""), row("z", "keeper", "old")],
            query: String::new(),
            selected: 0,
            selected_id: None,
            done: true,
            renderer: RefCell::default(),
        };
        model.move_by(1);
        let mut scan = Scan::default();
        let page = |items, has_more: bool| PickerPage {
            items,
            next_cursor: has_more.then(|| "next".into()),
            high_water_ordinal: 3,
            scope_revision: None,
            has_more,
            stop_reason: if has_more {
                crate::protocol::pagination::StopReason::Rows
            } else {
                crate::protocol::pagination::StopReason::Complete
            },
            consistency: crate::protocol::pagination::Consistency::BoundedLive,
        };
        assert!(
            !scan
                .accept(page(vec![row("a", "alpha", "new")], true), &mut model)
                .unwrap()
        );
        assert_eq!(model.selected(), Some(ThreadId::new("z")));
        assert_eq!(model.rows[1].topic_data, "old");
        // A stale restart discards only the in-progress scan; displayed data stays selectable.
        scan = Scan::default();
        assert!(
            !scan
                .accept(page(vec![row("0", "inserted", "")], true), &mut model)
                .unwrap()
        );
        assert_eq!(model.selected(), Some(ThreadId::new("z")));
        assert!(
            scan.accept(page(vec![row("z", "keeper", "new")], false), &mut model)
                .unwrap()
        );
        assert_eq!(model.selected(), Some(ThreadId::new("z")));
        assert_eq!(model.rows[1].topic_data, "new");
    }
    #[test]
    fn recent_picker_latest_preview_identifies_kind_and_local_truncation() {
        let mut channel = row("a", "alpha", "topic");
        channel.last_message = Some(crate::protocol::results::PickerMessagePreview {
            kind: MessageKind::Warn,
            sequence: 9,
            at: crate::protocol::time::UtcMillis(1),
            preview_data: format!("warning\x1b\n{}", "x".repeat(80)),
            preview_omitted: false,
        });
        let model = Model {
            rows: vec![channel],
            query: String::new(),
            selected: 0,
            selected_id: None,
            done: true,
            renderer: RefCell::default(),
        };
        let frame = model.frame(
            Size {
                width: 120,
                height: 20,
            },
            false,
        );
        let latest = &frame[3];
        assert!(latest.contains("[warn #9]"));
        assert!(latest.contains("\\u{001b}\\n"));
        assert!(latest.contains('…'));
        assert!(!latest.contains('\x1b'));
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
            selected_id: None,
            done: false,
            renderer: RefCell::default(),
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
