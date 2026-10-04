//! Read-only compact operator snapshot.

pub mod escape;

use crate::protocol::{
    commands::{Command, DiagnosticsQuery, DirectoryMembership, DirectoryQuery},
    output::{OutputFormat, OutputSpec, encode_selected, format_command_argv},
    pagination::{Consistency, MAX_PAGE_BYTES, Page, PageRequest, StopReason},
    results::{ApiError, CommandResult, Diagnostic, ErrorCode, Health, ThreadSummary},
    time::{CallBudget, Clock},
};

pub trait ViewReader {
    /// Every child read receives the same outer deadline and cancellation;
    /// implementations must stop when either is exhausted.
    fn read(
        &mut self,
        command: Command,
        output: &OutputSpec,
        budget: &CallBudget,
    ) -> Result<CommandResult, ApiError>;
}

pub struct ViewSnapshot {
    pub health: Result<Health, ApiError>,
    pub threads: Result<Page<ThreadSummary>, ApiError>,
    pub overdue: Result<Page<Diagnostic>, ApiError>,
    pub overdue_preview_omitted: bool,
}

/// All three queries are read-only. The instance-wide diagnostics page is the
/// bounded overdue discovery route and keeps its own continuation.
#[doc(hidden)]
pub fn load_snapshot<R: ViewReader>(
    reader: &mut R,
    page: &PageRequest,
    spec: &OutputSpec,
    budget: &CallBudget,
) -> ViewSnapshot {
    let health = match reader.read(Command::Health, spec, budget) {
        Ok(CommandResult::Health(value)) => Ok(value),
        Ok(_) => Err(wrong_result()),
        Err(error) => Err(error),
    };
    let overdue = match reader.read(
        Command::Diagnostics(DiagnosticsQuery {
            seat: None,
            thread: None,
            page: PageRequest {
                cursor: None,
                limit: 1,
                max_bytes: page.max_bytes.min(1024),
            },
        }),
        spec,
        budget,
    ) {
        Ok(CommandResult::Diagnostics(value)) => {
            value.validate().map(|_| value).map_err(|_| wrong_result())
        }
        Ok(_) => Err(wrong_result()),
        Err(error) => Err(error),
    };
    let allowance = directory_allowance(&health, &overdue, page, spec);
    let threads = read_directory(
        reader,
        &PageRequest {
            max_bytes: allowance,
            ..page.clone()
        },
        spec,
        budget,
    );
    ViewSnapshot {
        health,
        threads,
        overdue,
        overdue_preview_omitted: false,
    }
}

fn directory_allowance(
    health: &Result<Health, ApiError>,
    overdue: &Result<Page<Diagnostic>, ApiError>,
    page: &PageRequest,
    spec: &OutputSpec,
) -> u32 {
    let empty = Page::<ThreadSummary> {
        items: vec![],
        next_cursor: None,
        next_argv: None,
        high_water_ordinal: 0,
        scope_revision: None,
        has_more: false,
        stop_reason: StopReason::Complete,
        consistency: Consistency::BoundedLive,
    };
    let snapshot = ViewSnapshot {
        health: health.clone(),
        threads: Ok(empty.clone()),
        overdue: overdue.clone(),
        overdue_preview_omitted: false,
    };
    let fixed = match render_snapshot(&snapshot, page, spec) {
        Ok(bytes) => bytes.len(),
        Err(error) if error.code == ErrorCode::InvalidBudget => {
            error.required_minimum_bytes.unwrap_or(u32::MAX) as usize
        }
        Err(_) => return page.max_bytes,
    };
    let child = encode_selected(
        &CommandResult::Directory(empty),
        &OutputSpec {
            format: OutputFormat::Text,
            context: spec.context.clone(),
        },
    )
    .map(|bytes| bytes.len())
    .unwrap_or(0);
    page.max_bytes
        .saturating_sub(u32::try_from(fixed).unwrap_or(u32::MAX))
        .saturating_add(u32::try_from(child).unwrap_or(u32::MAX))
        .clamp(256, MAX_PAGE_BYTES)
}

fn read_directory<R: ViewReader>(
    reader: &mut R,
    page: &PageRequest,
    spec: &OutputSpec,
    budget: &CallBudget,
) -> Result<Page<ThreadSummary>, ApiError> {
    match reader.read(
        Command::Directory(DirectoryQuery {
            recent: false,
            membership: None,
            membership_filter: DirectoryMembership::All,
            topic_contains: None,
            page: page.clone(),
        }),
        spec,
        budget,
    ) {
        Ok(CommandResult::Directory(value)) => {
            value.validate().map_err(|_| wrong_result())?;
            if value.has_more && value.next_cursor.as_deref() == page.cursor.as_deref() {
                Err(wrong_result())
            } else {
                Ok(value)
            }
        }
        Ok(_) => Err(wrong_result()),
        Err(error) => Err(error),
    }
}

/// Fit complete source pages against the final view bytes. Retrying always
/// starts from the caller's cursor, so the source generates the only cursor
/// that can identify the first withheld row.
pub fn render_view<R: ViewReader>(
    reader: &mut R,
    page: &PageRequest,
    spec: &OutputSpec,
    budget: &CallBudget,
    clock: &dyn Clock,
) -> Result<Vec<u8>, ApiError> {
    page.validate().map_err(ApiError::invalid_budget)?;
    spec.validate().map_err(ApiError::invalid_request)?;
    let mut limit = page.limit;
    if budget.is_exhausted(clock) {
        return Err(exhausted(budget));
    }
    let mut snapshot = load_snapshot(reader, page, spec, budget);
    let allowance = directory_allowance(&snapshot.health, &snapshot.overdue, page, spec);
    loop {
        if budget.is_exhausted(clock) {
            return Err(exhausted(budget));
        }
        if snapshot
            .threads
            .as_ref()
            .is_err_and(|error| error.code == ErrorCode::InvalidBudget)
        {
            // A 256-byte child allowance can be too small to return its
            // first row even when the caller's view budget is usable. Probe
            // one row with the largest legal child bound so the view reports
            // the measured complete minimum, including its action.
            snapshot.threads = read_directory(
                reader,
                &PageRequest {
                    limit: 1,
                    max_bytes: crate::protocol::pagination::MAX_PAGE_BYTES,
                    ..page.clone()
                },
                spec,
                budget,
            );
            if budget.is_exhausted(clock) {
                return Err(exhausted(budget));
            }
            limit = 1;
        }
        // Source errors other than temporary unavailability cannot be
        // mistaken for a completed directory traversal.
        if let Err(error) = &snapshot.threads
            && error.code != ErrorCode::HostUnavailable
        {
            return Err(error.clone());
        }
        match render_snapshot(&snapshot, page, spec) {
            Ok(bytes) => return Ok(bytes),
            Err(error) if error.code == ErrorCode::InvalidBudget && limit > 1 => {
                limit -= 1;
                if budget.is_exhausted(clock) {
                    return Err(exhausted(budget));
                }
                snapshot.threads = read_directory(
                    reader,
                    &PageRequest {
                        limit,
                        max_bytes: allowance,
                        ..page.clone()
                    },
                    spec,
                    budget,
                );
            }
            Err(error)
                if error.code == ErrorCode::InvalidBudget
                    && !snapshot.overdue_preview_omitted
                    && snapshot.overdue.is_ok() =>
            {
                snapshot.overdue_preview_omitted = true;
            }
            Err(mut error) if error.code == ErrorCode::InvalidBudget => {
                // The budget value is itself printed in refresh/continuation
                // commands. Find the stable byte minimum for this smallest
                // complete candidate before suggesting a legal retry.
                let mut minimum = error.required_minimum_bytes.unwrap_or(u32::MAX);
                while minimum <= crate::protocol::pagination::MAX_PAGE_BYTES {
                    let candidate = PageRequest {
                        max_bytes: minimum,
                        ..page.clone()
                    };
                    match render_snapshot(&snapshot, &candidate, spec) {
                        Ok(_) => break,
                        Err(next) if next.code == ErrorCode::InvalidBudget => {
                            let next_minimum = next.required_minimum_bytes.unwrap_or(u32::MAX);
                            if next_minimum <= minimum {
                                break;
                            }
                            minimum = next_minimum;
                        }
                        Err(next) => return Err(next),
                    }
                }
                error.required_minimum_bytes = Some(minimum);
                return Err(error);
            }
            Err(error) => return Err(error),
        }
    }
}

fn exhausted(budget: &CallBudget) -> ApiError {
    ApiError::new(
        if budget.cancellation.is_cancelled() {
            ErrorCode::Cancelled
        } else {
            ErrorCode::DeadlineExceeded
        },
        "view read budget exhausted",
    )
}

fn wrong_result() -> ApiError {
    ApiError::invalid_request("unexpected view read result")
}

fn command_argv(spec: &OutputSpec, tail: &[String]) -> Vec<String> {
    let mut argv = vec!["herdr-threads".to_string()];
    if let Some(path) = &spec.context.state_dir {
        argv.extend(["--state-dir".into(), path.clone()]);
    }
    if let Some(host) = &spec.context.host {
        argv.extend(["--host-endpoint".into(), host.as_str().into()]);
    }
    if spec.format == OutputFormat::Json {
        argv.push("--json".into());
    }
    argv.extend_from_slice(tail);
    argv
}

pub fn view_argv(spec: &OutputSpec, page: &PageRequest, cursor: Option<&str>) -> Vec<String> {
    // The one-shot CLI renders only `view --once`; refresh and continuation
    // links must be directly runnable.
    let mut tail = vec![
        "view".into(),
        "--once".into(),
        "--limit".into(),
        page.limit.to_string(),
        "--max-bytes".into(),
        page.max_bytes.to_string(),
    ];
    if let Some(cursor) = cursor {
        tail.extend(["--cursor".into(), cursor.into()]);
    }
    command_argv(spec, &tail)
}

/// Rendering reuses the shared selected encoder for every peer-controlled
/// field and measures the complete final snapshot before returning any bytes.
pub fn render_snapshot(
    snapshot: &ViewSnapshot,
    page: &PageRequest,
    spec: &OutputSpec,
) -> Result<Vec<u8>, ApiError> {
    let text_spec = OutputSpec {
        format: OutputFormat::Text,
        context: spec.context.clone(),
    };
    let mut bytes = b"herdr-threads view\n".to_vec();
    for (name, result) in [
        (
            "health",
            snapshot
                .health
                .as_ref()
                .map(|value| CommandResult::Health(value.clone())),
        ),
        (
            "threads",
            snapshot
                .threads
                .as_ref()
                .map(|value| CommandResult::Directory(value.clone())),
        ),
        (
            "overdue",
            snapshot
                .overdue
                .as_ref()
                .map(|value| CommandResult::Diagnostics(value.clone())),
        ),
    ] {
        bytes.extend_from_slice(name.as_bytes());
        bytes.push(b'\n');
        if name == "overdue" && snapshot.overdue_preview_omitted {
            // Omission only ever shrinks the view: a compact preview no
            // longer than the placeholder is kept.
            const OMITTED: &[u8] = b"preview omitted: use overdue action\n";
            match result
                .as_ref()
                .map(|result| encode_selected(result, &text_spec))
            {
                Ok(Ok(full)) if full.len() <= OMITTED.len() => bytes.extend(full),
                _ => bytes.extend_from_slice(OMITTED),
            }
            continue;
        }
        match result {
            Ok(result) => bytes.extend(encode_selected(&result, &text_spec)?),
            Err(error) => bytes.extend(format!("unavailable: {:?}\n", error.code).into_bytes()),
        }
    }
    bytes.extend_from_slice(b"actions\n");
    append_action(&mut bytes, "refresh", &view_argv(spec, page, None));
    append_action(
        &mut bytes,
        "health",
        &command_argv(spec, &["daemon".into(), "health".into()]),
    );
    append_action(
        &mut bytes,
        "overdue",
        &command_argv(
            spec,
            &[
                "overdue".into(),
                "--limit".into(),
                page.limit.to_string(),
                "--max-bytes".into(),
                page.max_bytes.to_string(),
            ],
        ),
    );
    if let Ok(threads) = &snapshot.threads {
        for thread in &threads.items {
            append_action(
                &mut bytes,
                &format!("thread {}", thread.thread.as_str()),
                &command_argv(
                    spec,
                    &[
                        "thread".into(),
                        "show".into(),
                        thread.thread.as_str().into(),
                    ],
                ),
            );
        }
        if threads.has_more
            && let Some(cursor) = &threads.next_cursor
        {
            append_action(
                &mut bytes,
                "next view",
                &view_argv(spec, page, Some(cursor)),
            );
        }
    }
    if bytes.len() > page.max_bytes as usize {
        return Err(ApiError::invalid_budget("view exceeds byte budget")
            .with_required_minimum_bytes(bytes.len().try_into().unwrap_or(u32::MAX)));
    }
    Ok(bytes)
}

fn append_action(bytes: &mut Vec<u8>, label: &str, argv: &[String]) {
    bytes.extend_from_slice(label.as_bytes());
    bytes.extend_from_slice(b": ");
    bytes.extend_from_slice(format_command_argv(argv).as_bytes());
    bytes.push(b'\n');
}
