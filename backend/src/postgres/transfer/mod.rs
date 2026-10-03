//! File-backed bounded CSV transfers. Separate from native pg_dump/restore jobs.
pub(crate) mod csv;
mod files;
pub(crate) mod manager;
pub(crate) mod native;
pub(crate) mod protocol;
pub(crate) mod runner;
pub(crate) use manager::TransferManager;

#[cfg(test)]
mod csv_tests;
