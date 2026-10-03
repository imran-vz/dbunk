//! Ephemeral PostgreSQL file-job setup and bounded observation state. Nothing in
//! this module is workspace recovery data or permission to execute a job.
use std::{
    cell::Cell,
    ffi::OsStr,
    path::{Path, PathBuf},
    rc::Rc,
};

const WORKSPACE_BYTES: usize = 128 * 1024 * 1024;
const SETUP_BYTES: usize = 64 * 1024;
pub const MAX_PATH_BYTES: usize = dbunk_lib::backend::pg_tools::MAX_PG_TOOL_PATH_BYTES;

use dbunk_lib::backend::pg_tools::{
    MAX_PG_TOOL_ACTIVE, MAX_PG_TOOL_LIST_BYTES, MAX_PG_TOOL_TERMINAL, PgToolAttemptId,
    PgToolCleanup, PgToolEffect, PgToolIntent, PgToolJobList, PgToolObservation, PgToolPhase,
};
pub use dbunk_lib::backend::pg_tools::{
    PgToolFormat as Format, PgToolKind as Operation, PgToolScope as Scope,
};

/// Captures one exact setup revision, including its connection generation.
/// Tokens from a closed/recreated setup never match, even at the same revision.
pub struct SetupToken {
    owner: uuid::Uuid,
    revision: u64,
}

pub struct Setup {
    owner: uuid::Uuid,
    revision: u64,
    connection: String,
    generation: u64,
    operation: Operation,
    format: Format,
    scope: Scope,
    clean: bool,
    path: Option<PathBuf>,
    budget: Rc<Cell<usize>>,
}
impl Setup {
    pub fn new(
        connection: String,
        generation: u64,
        operation: Operation,
        context: Option<(String, String)>,
        budget: Rc<Cell<usize>>,
    ) -> Result<Self, &'static str> {
        let scope = context.map_or(Scope::Database, |(schema, table)| Scope::Table {
            schema,
            table,
        });
        if !valid_connection(&connection) || scope.checked_heap_bytes().is_none() {
            return Err("Invalid or oversized PostgreSQL job target");
        }
        if SETUP_BYTES > WORKSPACE_BYTES.saturating_sub(budget.get()) {
            return Err("Job setup needs 64 KiB of shared allowance");
        }
        budget.set(budget.get() + SETUP_BYTES);
        Ok(Self {
            owner: uuid::Uuid::new_v4(),
            revision: 0,
            connection,
            generation,
            operation,
            format: Format::Custom,
            scope,
            clean: false,
            path: None,
            budget,
        })
    }
    pub fn connection(&self) -> &str {
        &self.connection
    }
    pub fn generation(&self) -> u64 {
        self.generation
    }
    pub fn operation(&self) -> Operation {
        self.operation
    }
    pub fn format(&self) -> Format {
        self.format
    }
    #[cfg(test)]
    pub fn scope(&self) -> &Scope {
        &self.scope
    }
    pub fn clean(&self) -> bool {
        self.clean && self.clean_enabled()
    }
    pub fn path(&self) -> Option<&Path> {
        self.path.as_deref()
    }
    pub fn filename(&self) -> Option<&OsStr> {
        self.path()?.file_name()
    }
    pub fn clean_enabled(&self) -> bool {
        matches!(
            (self.operation, self.format),
            (Operation::Backup, Format::Plain) | (Operation::Restore, Format::Custom)
        )
    }
    pub fn token(&self) -> SetupToken {
        SetupToken {
            owner: self.owner,
            revision: self.revision,
        }
    }
    pub fn is_current(&self, token: &SetupToken) -> bool {
        token.owner == self.owner && token.revision == self.revision
    }
    fn invalidate(&mut self) -> Result<(), &'static str> {
        self.revision = self
            .revision
            .checked_add(1)
            .ok_or("Job setup revision exhausted; reopen setup")?;
        Ok(())
    }
    pub fn set_operation(&mut self, operation: Operation) -> Result<(), &'static str> {
        if self.operation != operation {
            self.invalidate()?;
            self.operation = operation;
            self.path = None;
            self.clean = false;
        }
        Ok(())
    }
    pub fn set_format(&mut self, format: Format) -> Result<(), &'static str> {
        if self.format != format {
            self.invalidate()?;
            self.format = format;
            self.path = None;
            self.clean = false;
        }
        Ok(())
    }
    pub fn set_scope(&mut self, scope: Scope) -> Result<(), &'static str> {
        if scope.checked_heap_bytes().is_none() {
            return Err("Choose a valid database, schema or table scope");
        }
        if self.scope != scope {
            self.invalidate()?;
            self.scope = scope;
        }
        Ok(())
    }
    pub fn set_clean(&mut self, clean: bool) -> Result<(), &'static str> {
        if clean && !self.clean_enabled() {
            return Err("Cleanup is unavailable for this operation and format");
        }
        if self.clean != clean {
            self.invalidate()?;
            self.clean = clean;
        }
        Ok(())
    }
    /// Same-ID connection edits still invalidate all file and review state.
    pub fn retarget(&mut self, connection: String, generation: u64) -> Result<(), &'static str> {
        if !valid_connection(&connection) {
            return Err("Invalid or oversized PostgreSQL job target");
        }
        if self.connection != connection || self.generation != generation {
            self.invalidate()?;
            self.connection = connection;
            self.generation = generation;
            self.path = None;
            self.scope = Scope::Database;
            self.clean = false;
        }
        Ok(())
    }
    /// Selecting a path does not inspect its contents or authorize execution.
    /// Refusal preserves the previous file and never normalizes path bytes.
    pub fn accept_path(&mut self, token: &SetupToken, path: PathBuf) -> Result<(), &'static str> {
        if !self.is_current(token) {
            return Err("File selection belongs to an older setup");
        }
        if path.to_str().is_none() {
            return Err("Select an absolute local Unicode file path");
        }
        if !path.is_absolute()
            || path.file_name().is_none()
            || path.file_name().is_some_and(|name| name.len() > 1024)
            || path.as_os_str().as_encoded_bytes().contains(&0)
            || path.as_os_str().len() > MAX_PATH_BYTES
            || path.capacity() > MAX_PATH_BYTES
        {
            return Err("Selected file path exceeds bounds or is invalid");
        }
        self.invalidate()?;
        self.path = Some(path);
        Ok(())
    }
    pub fn intent(&self) -> Result<PgToolIntent, &'static str> {
        let path = self.path.clone().ok_or("Select a file first")?;
        match self.operation {
            Operation::Backup => {
                PgToolIntent::backup(path, self.format, self.scope.clone(), self.clean())
            }
            Operation::Restore => PgToolIntent::restore(path, self.format, self.clean()),
        }
        .map_err(|_| "Selected job inputs exceed the supported bounds")
    }
}
impl Drop for Setup {
    fn drop(&mut self) {
        self.budget
            .set(self.budget.get().saturating_sub(SETUP_BYTES));
    }
}
fn valid_connection(value: &String) -> bool {
    !value.is_empty()
        && value.len() <= 256
        && value.capacity() <= 1024
        && !value.chars().any(char::is_control)
}

/// Shared observer ordering. A release/fence invalidates older full-list replies;
/// a missing row never establishes whether a queued admission happened.
#[derive(Default)]
pub struct ObservationOrder {
    issued: u64,
    applied: u64,
}
impl ObservationOrder {
    pub fn issue(&mut self) -> Result<u64, &'static str> {
        self.issued = self
            .issued
            .checked_add(1)
            .ok_or("Job observation identity exhausted")?;
        Ok(self.issued)
    }
    pub fn accept(&mut self, sequence: u64) -> bool {
        if sequence == 0 || sequence > self.issued || sequence < self.applied {
            return false;
        }
        self.applied = sequence;
        true
    }
    pub fn fence(&mut self) -> Result<(), &'static str> {
        self.applied = self.issue()?;
        Ok(())
    }
}

const CAPTURE_BYTES: usize = 1024 * 1024;

/// One app-owned capture is shared by all setup views. Admission happens before
/// replacing the last good capture; views borrow rows instead of cloning lists.
pub struct Capture {
    data: PgToolJobList,
    budget: Rc<Cell<usize>>,
}
impl Capture {
    pub fn new(data: PgToolJobList, budget: Rc<Cell<usize>>) -> Result<Self, &'static str> {
        if data
            .checked_heap_bytes()
            .is_none_or(|bytes| bytes > MAX_PG_TOOL_LIST_BYTES)
            || data
                .encoded_bytes()
                .is_none_or(|bytes| bytes > MAX_PG_TOOL_LIST_BYTES)
        {
            return Err("Job observation exceeds its bounds; previous observation retained");
        }
        let active = data.jobs.iter().filter(|job| !releasable(job)).count();
        if active > MAX_PG_TOOL_ACTIVE
            || data.jobs.len() - active > MAX_PG_TOOL_TERMINAL
            || data.jobs.iter().enumerate().any(|(index, job)| {
                !releasable(job)
                    && data.jobs[..index]
                        .iter()
                        .any(|other| !releasable(other) && other.connection_id == job.connection_id)
            })
        {
            return Err("Job observation violates admission limits; previous observation retained");
        }
        if CAPTURE_BYTES > WORKSPACE_BYTES.saturating_sub(budget.get()) {
            return Err(
                "Job observation needs 1 MiB of shared allowance; previous observation retained",
            );
        }
        budget.set(budget.get() + CAPTURE_BYTES);
        Ok(Self { data, budget })
    }
    pub fn matches(&self, data: &PgToolJobList) -> bool {
        self.data == *data
    }
    pub fn rows(&self) -> &[PgToolObservation] {
        &self.data.jobs
    }
    pub fn restore_revision(&self) -> u64 {
        self.data.restore_change_revision
    }
    pub fn restore_invalidation<'a>(
        &'a self,
        tracker: &RestoreChanges,
    ) -> Option<RestoreInvalidation<'a>> {
        let mut revisions = [(0, ""); dbunk_lib::backend::pg_tools::MAX_PG_TOOL_JOBS];
        let mut count = 0;
        for job in &self.data.jobs {
            if let Some(revision) = job.restore_change_revision {
                revisions[count] = (revision, job.connection_id.as_str());
                count += 1;
            }
        }
        tracker.plan(self.restore_revision(), &revisions[..count])
    }
    pub fn row(&self, index: usize) -> Option<&PgToolObservation> {
        self.data.jobs.get(index)
    }
    pub fn key(&self, index: usize) -> Option<PgToolAttemptId> {
        Some(self.row(index)?.attempt_id)
    }
    pub fn index_for_key(&self, key: PgToolAttemptId) -> Option<usize> {
        self.data.jobs.iter().position(|job| job.attempt_id == key)
    }
    pub fn active_on(&self, connection: &str) -> bool {
        self.data
            .jobs
            .iter()
            .any(|job| job.connection_id == connection && !releasable(job))
    }
    pub fn has_active(&self) -> bool {
        self.data.jobs.iter().any(|job| !releasable(job))
    }
    pub fn row_label(&self, index: usize) -> Option<String> {
        let job = self.row(index)?;
        Some(format!(
            "{} · {} · {}",
            job.file_name,
            operation_label(job.kind),
            phase_label(job.phase)
        ))
    }
    pub fn details(&self, index: usize) -> Option<String> {
        let job = self.row(index)?;
        let effect = match job.effect {
            PgToolEffect::NotStarted => "Execution has not started",
            PgToolEffect::Pending => "Execution outcome is pending",
            PgToolEffect::Succeeded => "Successful execution observed",
            PgToolEffect::Unknown => {
                "Execution outcome is unknown; inspect the target before another operation"
            }
        };
        let cleanup = match job.cleanup {
            PgToolCleanup::Pending => "Cleanup pending; admission remains held",
            PgToolCleanup::Complete => "Cleanup complete",
            PgToolCleanup::Failed => "Cleanup could not be established; admission remains held",
        };
        let scope = match &job.scope {
            Scope::Database => "Entire database".into(),
            Scope::Schema { schema } => format!("Schema: {schema}"),
            Scope::Table { schema, table } => format!("Table: {schema}.{table}"),
        };
        Some(format!(
            "{} · {}\nConnection: {}\nAttempt: {}\nState: {}\nFormat: {}\nScope: {}\nCleanup option: {}\nStarted: {}\nFinished: {}\nWritten: {}\nSource size: {}\nClient: {}\n{}\n{}\n{}\n{}",
            operation_label(job.kind),
            job.file_name,
            job.connection_id,
            job.attempt_id,
            phase_label(job.phase),
            match job.format {
                Format::Plain => "Plain SQL",
                Format::Custom => "Custom archive",
            },
            scope,
            if job.clean { "Enabled" } else { "Disabled" },
            job.started_at,
            job.finished_at.as_deref().unwrap_or("Not observed"),
            bytes_label(job.bytes_processed),
            bytes_label(job.source_bytes),
            job.tool_version.as_deref().unwrap_or("Unavailable"),
            effect,
            cleanup,
            job.diagnostic.as_ref().map_or_else(
                || job
                    .failure
                    .map_or_else(String::new, |error| error.to_string()),
                |diagnostic| format!(
                    "{}\nTool: {}\nExit code: {}\nOperation: {}\n{}",
                    job.failure
                        .map_or_else(String::new, |error| error.to_string()),
                    diagnostic.tool.as_deref().unwrap_or("Unavailable"),
                    diagnostic
                        .exit_code
                        .map_or_else(|| "Unavailable".into(), |code| code.to_string()),
                    diagnostic.operation.as_deref().unwrap_or("Unavailable"),
                    diagnostic.message
                )
            ),
            if job.kind == Operation::Restore {
                "Restore targets the database. Progress percentage is unavailable. Cancellation cannot undo a committed restore."
            } else {
                "Backup completion requires successful publication; an unfinished file is not a completed backup."
            },
        ))
    }
    pub fn limits(&self) -> &'static str {
        "This session: up to four admitted jobs, one per connection, and 32 finished jobs retained for up to one hour. Missing or expired observations do not prove a queued start did not happen."
    }
}
impl Drop for Capture {
    fn drop(&mut self) {
        self.budget
            .set(self.budget.get().saturating_sub(CAPTURE_BYTES));
    }
}
pub fn releasable(job: &PgToolObservation) -> bool {
    job.phase.terminal() && job.cleanup == PgToolCleanup::Complete
}
pub fn cancellable(job: &PgToolObservation) -> bool {
    !job.phase.terminal() && job.phase != PgToolPhase::Cancelling
}
pub fn operation_label(operation: Operation) -> &'static str {
    match operation {
        Operation::Backup => "Backup",
        Operation::Restore => "Restore",
    }
}
pub fn phase_label(phase: PgToolPhase) -> &'static str {
    match phase {
        PgToolPhase::Preparing => "Preparing source",
        PgToolPhase::ReadyReview => "Ready for review",
        PgToolPhase::AwaitingConfirmation => "Awaiting confirmation",
        PgToolPhase::Queued => "Queued",
        PgToolPhase::Preflight => "Checking client tools",
        PgToolPhase::Running => "Running",
        PgToolPhase::Finalizing => "Finalizing; publication may already be committed",
        PgToolPhase::Cancelling => "Cancelling; waiting for owned work",
        PgToolPhase::Completed => "Completed",
        PgToolPhase::Cancelled => "Cancelled",
        PgToolPhase::Failed => "Failed",
    }
}
fn bytes_label(value: Option<u64>) -> String {
    value.map_or_else(|| "Unavailable".into(), |bytes| format!("{bytes} B"))
}

/// Backend-lifetime restore highwater. Advancing it requires the caller to apply
/// source invalidation first; an observation failure or dropped plan cannot ack.
#[derive(Default)]
pub struct RestoreChanges {
    acknowledged: u64,
}

pub struct RestoreInvalidation<'a> {
    from: u64,
    to: u64,
    all: bool,
    connections: [Option<&'a str>; dbunk_lib::backend::pg_tools::MAX_PG_TOOL_JOBS],
}
impl RestoreInvalidation<'_> {
    /// None requires all PostgreSQL sources to be invalidated. This is the
    /// conservative fallback when released/expired records leave a revision gap.
    pub fn connections(&self) -> Option<impl Iterator<Item = &str>> {
        (!self.all).then(|| self.connections.iter().flatten().copied())
    }
}
impl RestoreChanges {
    #[cfg(test)]
    pub fn acknowledged(&self) -> u64 {
        self.acknowledged
    }
    /// `revisions` comes from the same full observation as `highwater`. The
    /// returned connection references borrow its capture and retain its lease.
    pub fn plan<'a>(
        &self,
        highwater: u64,
        revisions: &[(u64, &'a str)],
    ) -> Option<RestoreInvalidation<'a>> {
        if highwater <= self.acknowledged {
            return None;
        }
        let mut plan = RestoreInvalidation {
            from: self.acknowledged,
            to: highwater,
            all: false,
            connections: [None; dbunk_lib::backend::pg_tools::MAX_PG_TOOL_JOBS],
        };
        let delta = highwater - self.acknowledged;
        if delta > plan.connections.len() as u64 || revisions.len() > plan.connections.len() {
            plan.all = true;
            return Some(plan);
        }
        let mut count = 0;
        for offset in 1..=delta {
            let expected = self.acknowledged + offset;
            let mut matching = revisions
                .iter()
                .filter(|(revision, _)| *revision == expected);
            let Some((_, connection)) = matching.next() else {
                plan.all = true;
                break;
            };
            if matching.next().is_some() || connection.is_empty() {
                plan.all = true;
                break;
            }
            if !plan.connections[..count].contains(&Some(*connection)) {
                plan.connections[count] = Some(connection);
                count += 1;
            }
        }
        Some(plan)
    }
    /// Call only after every targeted source has been invalidated. Reusing a
    /// plan after another acknowledgement is refused instead of losing changes.
    pub fn acknowledge(&mut self, plan: RestoreInvalidation<'_>) -> Result<(), &'static str> {
        if plan.from != self.acknowledged {
            return Err("Restore invalidation plan is stale");
        }
        self.acknowledged = plan.to;
        Ok(())
    }
}

#[cfg(test)]
mod tests;
