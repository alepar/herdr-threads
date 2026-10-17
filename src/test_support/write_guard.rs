//! The write-path closure for replicated tables: a SQLite authorizer that
//! fails any write to a replicated table outside an apply savepoint or a
//! `materialize::local_only` scope (fact-kinds design, Testing "Write-path
//! closure").
//!
//! Responsibility
//! - Owns: [`REPLICATED_TABLES`] (the replicated-table list) and the rule
//!   "writes only inside an apply savepoint or a `local_only` scope", enforced
//!   by [`arm`] (the authorizer) with [`enter_replicated_write_scope`] as the
//!   marker the two legal write paths raise (`store::apply::apply_entry` and
//!   `materialize::local_only`).
//! - Must not: exist outside `test-support` builds (the module and both scope
//!   call sites are `cfg`'d), or change behaviour when disarmed (a [`Guard`]
//!   that is dropped or never armed leaves the connection untouched; the scope
//!   marker alone is a thread-local counter nobody reads). One residue: arming
//!   leaves the connection's prepared-statement cache off.
//! - Where a new replicated table goes: add its name to [`REPLICATED_TABLES`]
//!   here and its replicated columns to `journal_oracle::CONVERTED_TABLES`; the
//!   two are the two sides of one table set (a test keeps the projection's
//!   tables inside this list).
//! - Role: test harness. Each fact-kind leaf arms the guard on the writer
//!   around the commands it converts.
//!
//! Limits, by construction of the SQLite authorizer (it sees table and column
//! names at statement prepare time, never row values or savepoint names):
//! - `seats`: an UPDATE of `state` or a `retired_*` column is denied outside
//!   scope; an INSERT cannot be told local from remote, so it is denied only
//!   under [`Guard::strict_seat_inserts`] (for tests that never allocate a
//!   local seat).
//! - `messages`: `kind` is invisible, so every write is denied outside scope;
//!   a test that writes service or notice messages runs them inside a
//!   `local_only` scope or exempts the table with [`Guard::allow`].
//! - The check runs when a statement is prepared. [`arm`] therefore turns the
//!   connection's prepared-statement cache off, so a statement
//!   prepared before arming (or inside a scope) cannot be reused past the
//!   check.
//! - The scope counter is thread-local: arm the connection on the thread that
//!   runs the commands under test.

use std::cell::Cell;
use std::ffi::{CStr, c_char, c_int, c_void};
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};

use rusqlite::{Connection, ffi};

/// The replicated tables (`seats` with the limits above). One side of the
/// table set; `journal_oracle::CONVERTED_TABLES` is the other.
pub const REPLICATED_TABLES: &[&str] = &[
    "seats",
    "threads",
    "direct_threads",
    "invitations",
    "invitation_rejections",
    "memberships",
    "membership_intervals",
    "messages",
    "send_manifests",
    "prepared_recipients",
    "receipt_state",
    "thread_registers",
];

/// The `seats` columns whose UPDATE counts as a replicated write.
fn seat_column_replicated(column: &str) -> bool {
    column == "state" || column.starts_with("retired_")
}

thread_local! {
    static SCOPE_DEPTH: Cell<usize> = const { Cell::new(0) };
}

/// Marks the current thread as inside a legal replicated write path until
/// dropped. Nests.
///
/// Owns: one increment of the thread-local depth. Must not: be moved to
/// another thread or outlive the write it covers.
pub struct ScopeGuard(());

impl Drop for ScopeGuard {
    fn drop(&mut self) {
        SCOPE_DEPTH.with(|d| d.set(d.get().saturating_sub(1)));
    }
}

/// Raised by `apply_entry` around the handler and by `materialize::local_only`
/// for the lifetime of its input.
pub fn enter_replicated_write_scope() -> ScopeGuard {
    SCOPE_DEPTH.with(|d| d.set(d.get() + 1));
    ScopeGuard(())
}

fn in_scope() -> bool {
    SCOPE_DEPTH.with(|d| d.get() > 0)
}

struct Config {
    /// Cleared when the [`Guard`] drops; the callback then checks nothing.
    armed: AtomicBool,
    settings: Mutex<Settings>,
}

#[derive(Default)]
struct Settings {
    allowed: Vec<String>,
    strict_seat_inserts: bool,
}

/// An armed authorizer on one connection; dropping it disarms.
///
/// Holds no borrow of the connection, so the commands under test can take it
/// `&mut`; it must simply not be used after the connection is gone (a dropped
/// connection takes its authorizer with it).
pub struct Guard {
    config: &'static Config,
}

/// Arms the write guard on `conn` (see the module doc for what it checks and
/// its limits). The connection's statement cache is turned off for good.
pub fn arm(conn: &Connection) -> Guard {
    conn.set_prepared_statement_cache_capacity(0);
    // The config is leaked (a few dozen bytes per arm, test-support only) so
    // the callback's data stays valid however the connection and guard drop.
    let config: &'static Config = Box::leak(Box::new(Config {
        armed: AtomicBool::new(true),
        settings: Mutex::new(Settings::default()),
    }));
    let data: *mut c_void = (config as *const Config).cast_mut().cast();
    // SAFETY: `data` is a leaked, therefore never freed, `Config`; the
    // connection handle is live for this call.
    unsafe {
        ffi::sqlite3_set_authorizer(conn.handle(), Some(authorizer), data);
    }
    Guard { config }
}

impl Guard {
    /// Exempts tables from the check (see the `messages` limit).
    pub fn allow(&mut self, tables: &[&str]) -> &mut Self {
        let mut settings = self.config.settings.lock().expect("guard settings");
        settings
            .allowed
            .extend(tables.iter().map(|t| (*t).to_owned()));
        drop(settings);
        self
    }

    /// Also denies every INSERT into `seats` outside scope.
    pub fn strict_seat_inserts(&mut self) -> &mut Self {
        self.config
            .settings
            .lock()
            .expect("guard settings")
            .strict_seat_inserts = true;
        self
    }
}

impl Drop for Guard {
    fn drop(&mut self) {
        self.config.armed.store(false, Ordering::SeqCst);
    }
}

unsafe fn text<'a>(ptr: *const c_char) -> Option<&'a str> {
    if ptr.is_null() {
        None
    } else {
        // SAFETY: SQLite passes NUL-terminated strings valid for the call.
        unsafe { CStr::from_ptr(ptr) }.to_str().ok()
    }
}

unsafe extern "C" fn authorizer(
    data: *mut c_void,
    action: c_int,
    arg1: *const c_char,
    arg2: *const c_char,
    _db: *const c_char,
    _trigger: *const c_char,
) -> c_int {
    if !matches!(
        action,
        ffi::SQLITE_INSERT | ffi::SQLITE_UPDATE | ffi::SQLITE_DELETE
    ) || in_scope()
    {
        return ffi::SQLITE_OK;
    }
    // SAFETY: `data` is the leaked `Config` `arm` handed in.
    let config = unsafe { &*(data as *const Config) };
    if !config.armed.load(Ordering::SeqCst) {
        return ffi::SQLITE_OK;
    }
    let Ok(config) = config.settings.lock() else {
        return ffi::SQLITE_OK;
    };
    // SAFETY: see `text`.
    let (Some(table), column) = (unsafe { text(arg1) }, unsafe { text(arg2) }) else {
        return ffi::SQLITE_OK;
    };
    let Some(listed) = REPLICATED_TABLES
        .iter()
        .find(|t| t.eq_ignore_ascii_case(table))
    else {
        return ffi::SQLITE_OK;
    };
    if config
        .allowed
        .iter()
        .any(|a| a.eq_ignore_ascii_case(listed))
    {
        return ffi::SQLITE_OK;
    }
    if *listed == "seats" {
        let counts = match action {
            ffi::SQLITE_UPDATE => column.is_some_and(seat_column_replicated),
            ffi::SQLITE_INSERT => config.strict_seat_inserts,
            _ => false,
        };
        if !counts {
            return ffi::SQLITE_OK;
        }
    }
    ffi::SQLITE_DENY
}

#[cfg(test)]
#[path = "../../tests/store/write_guard.rs"]
mod tests;
