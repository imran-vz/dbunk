//! File-backed bounded CSV transfers. Separate from native pg_dump/restore jobs.
pub(crate) mod csv;
mod files;
pub(crate) mod manager;
pub(crate) mod protocol;
pub(crate) mod runner;
pub(crate) use manager::TransferManager;

// Drives the transfer commands, whose service is not extracted yet.
#[cfg(all(test, feature = "tauri-host"))]
mod runner_tests_live;

#[cfg(test)]
mod csv_tests;
