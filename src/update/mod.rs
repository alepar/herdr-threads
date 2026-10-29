//! `herdr-threads update` support shared by the command (`crate::cli::update`), the daemon's
//! daily update check and the CLI's update notice
//! (`docs/superpowers/specs/2026-10-28-update-command-design.md`).
//!
//! - [`install`]: the installer-managed installation the running executable belongs to (its
//!   marker) and the default state directories the installer scans.
//! - [`release`]: version parsing, the GitHub releases API document, `SHA256SUMS` and the
//!   bounded `curl` fetch.
//! - [`check`]: the node-local `update-check.json`, the check itself, its cadence and the
//!   notice line.
pub mod check;
pub mod install;
pub mod release;
