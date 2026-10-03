//! Page-fit helper (B1 nested spec D5). Every paged result sizes its page here:
//! an estimate from per-item single-item encodes, then an exact boundary found by
//! galloping and binary search. Page length is monotone in the item count, so the
//! result equals the greedy maximal prefix with O(n) single-item encodes and
//! O(log n) full-page encodes.
use super::{PageKind, longest_cursor_bytes};
use crate::protocol::{
    output::OutputSpec,
    results::{ApiError, CommandResult},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FitStop {
    Rows,
    Bytes,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Fit {
    pub accepted: usize,
    pub stop: FitStop,
    pub required_minimum: Option<u32>,
}

pub struct PageFit<'a> {
    pub output: &'a OutputSpec,
    pub max: usize,
    pub kind: PageKind,
}

// Counts encoder calls and bytes in tests (the encoder hook).
#[cfg(test)]
thread_local! {
    pub(crate) static ENCODES: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    pub(crate) static ENCODED_BYTES: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

fn record_encode(_len: usize) {
    #[cfg(test)]
    {
        ENCODES.with(|c| c.set(c.get() + 1));
        ENCODED_BYTES.with(|c| c.set(c.get() + _len));
    }
}

fn encoded_len(result: &CommandResult, output: &OutputSpec) -> Result<usize, ApiError> {
    let len = crate::protocol::output::encode_selected(result, output)?.len();
    record_encode(len);
    Ok(len)
}

/// Byte length of an internal page's fixed JSON representation. Internal pages
/// use their complete JSON for byte admission: both copies of a continuation
/// cursor and every candidate field, though they never become CLI output.
pub(super) fn internal_page_bytes<T: serde::Serialize>(
    page: &super::Page<T>,
) -> Result<usize, ApiError> {
    let len = serde_json::to_vec(page)
        .map(|bytes| bytes.len())
        .map_err(|_| {
            super::api_error(
                crate::protocol::results::ErrorCode::StoreCorrupt,
                "internal page encoding failed",
            )
        })?;
    record_encode(len);
    Ok(len)
}

impl<'a> PageFit<'a> {
    pub fn new(output: &'a OutputSpec, max: usize, kind: PageKind) -> Self {
        Self { output, max, kind }
    }

    /// A fit for a command-result page; only `max` and `output` matter (the
    /// kind only selects the internal-page cursor bound).
    pub fn for_command(output: &'a OutputSpec, max: usize) -> Self {
        Self::new(output, max, PageKind::Work)
    }

    /// The longest continuation cursor this page kind can emit.
    pub fn cursor_base_bytes(&self) -> usize {
        longest_cursor_bytes(self.kind)
    }

    /// The largest k such that `render(k)` encodes within `max`. Item sizes are
    /// estimated from `render(1)` alone; use `fit_with` for exact per-item deltas.
    pub fn fit<I>(
        &mut self,
        items: &[I],
        render: impl Fn(usize) -> Result<CommandResult, ApiError>,
    ) -> Result<Fit, ApiError> {
        let output = self.output;
        self.fit_sized(
            items.len(),
            |k| encoded_len(&render(k)?, output),
            |_| encoded_len(&render(1)?, output),
        )
    }

    /// `render(k)` is the page of the first k items with the continuation
    /// cursor of item k-1 (item 0's cursor for k = 0); `render_one(i)` is the
    /// page holding only item i with item i's cursor.
    pub fn fit_with<I>(
        &mut self,
        items: &[I],
        render: impl Fn(usize) -> Result<CommandResult, ApiError>,
        render_one: impl Fn(usize) -> Result<CommandResult, ApiError>,
    ) -> Result<Fit, ApiError> {
        let output = self.output;
        self.fit_sized(
            items.len(),
            |k| encoded_len(&render(k)?, output),
            |i| encoded_len(&render_one(i)?, output),
        )
    }

    /// The fit over byte lengths; see [`fit_counts`].
    pub fn fit_sized(
        &mut self,
        n: usize,
        len: impl Fn(usize) -> Result<usize, ApiError>,
        one: impl Fn(usize) -> Result<usize, ApiError>,
    ) -> Result<Fit, ApiError> {
        fit_counts(self.max, n, len, one)
    }
}

/// The exact maximal count k <= n with `len(k) <= max`, found by an estimate
/// (`len(0)` plus single-item deltas from `one(i)`, the encoded page holding
/// only item i), then galloping from the estimate and binary search. `len(k)`
/// is the encoded length of the page with the first k items.
pub fn fit_counts(
    max: usize,
    n: usize,
    len: impl Fn(usize) -> Result<usize, ApiError>,
    one: impl Fn(usize) -> Result<usize, ApiError>,
) -> Result<Fit, ApiError> {
    let known = std::cell::RefCell::new(std::collections::BTreeMap::<usize, usize>::new());
    let measure = |k: usize| -> Result<usize, ApiError> {
        if let Some(&cached) = known.borrow().get(&k) {
            return Ok(cached);
        }
        let measured = len(k)?;
        known.borrow_mut().insert(k, measured);
        Ok(measured)
    };
    // Zero items is the floor: greedy accepts 0 when even one item is over.
    let fits = |k: usize| -> Result<bool, ApiError> { Ok(k == 0 || measure(k)? <= max) };
    let base = measure(0)?;
    // Estimate the cut from single-item deltas; stop once the running sum
    // passes the budget, since later items cannot be inside the estimate.
    let mut running = base;
    let mut estimate = 0;
    for i in 0..n {
        let single = one(i)?;
        running = running.saturating_add(single.saturating_sub(base).max(1));
        if running > max {
            break;
        }
        estimate = i + 1;
    }
    // `lo` fits (0 is the floor); `hi` is the smallest count known not to
    // fit, or n + 1 when none is known.
    let mut hi = n + 1;
    let mut lo;
    if estimate > 0 && !fits(estimate)? {
        hi = estimate;
        let mut step = 1;
        loop {
            let probe = hi.saturating_sub(step);
            if probe == 0 || fits(probe)? {
                lo = probe;
                break;
            }
            hi = probe;
            step *= 2;
        }
    } else {
        lo = estimate;
        let mut step = 1;
        while lo < n {
            let probe = (lo + step).min(n);
            if fits(probe)? {
                lo = probe;
                step *= 2;
            } else {
                hi = probe;
                break;
            }
        }
    }
    while hi - lo > 1 {
        let mid = lo + (hi - lo) / 2;
        if fits(mid)? {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    let accepted = lo;
    let required_minimum = if accepted == 0 && n > 0 {
        Some(u32::try_from(measure(1)?).unwrap_or(u32::MAX))
    } else {
        None
    };
    Ok(Fit {
        accepted,
        stop: if accepted == n {
            FitStop::Rows
        } else {
            FitStop::Bytes
        },
        required_minimum,
    })
}

/// The fit tail of an internal (discovery) page: the largest prefix of `items`
/// whose fixed JSON page fits `max_bytes`. `positions[i]` is `(after before
/// item i, ordinal of item i)`; `full` builds the complete page (the walk's own
/// cursor and stop), `cut` builds a byte-cut page with a given `after`. A
/// single item that cannot fit alone is `InvalidBudget` with `detail`.
pub(super) fn fit_internal<T: serde::Serialize + Clone>(
    max_bytes: usize,
    items: Vec<T>,
    positions: &[(u64, u64)],
    full: impl Fn(Vec<T>) -> Result<super::Page<T>, ApiError>,
    cut: impl Fn(Vec<T>, u64) -> Result<super::Page<T>, ApiError>,
    detail: &'static str,
) -> Result<super::Page<T>, ApiError> {
    let n = items.len();
    let page_of = |k: usize| {
        if k == n {
            full(items.clone())
        } else {
            cut(items[..k].to_vec(), positions[k].0)
        }
    };
    if n == 0 {
        let page = full(Vec::new())?;
        let measured = internal_page_bytes(&page)?;
        return if measured <= max_bytes {
            Ok(page)
        } else {
            Err(super::internal_page_budget_error(detail, measured))
        };
    }
    let fit = fit_counts(
        max_bytes,
        n,
        |k| internal_page_bytes(&page_of(k)?),
        |i| internal_page_bytes(&cut(vec![items[i].clone()], positions[i].1)?),
    )?;
    if fit.accepted > 0 {
        return page_of(fit.accepted);
    }
    let single = cut(items[..1].to_vec(), positions[0].1)?;
    let minimum = internal_page_bytes(&single)?;
    if minimum <= max_bytes {
        Ok(single)
    } else {
        Err(super::internal_page_budget_error(detail, minimum))
    }
}

#[cfg(test)]
#[path = "../../tests/store/page_fit.rs"]
mod tests;
