use super::*;

/// Request fences reject an older observation delivered after a release.
/// Missing rows never prove that an in-flight admission did not happen.
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
            .ok_or("CSV observation identity exhausted")?;
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

pub struct Capture {
    inspections: CsvInspectionList,
    transfers: CsvTransferList,
    _lease: Lease,
}
impl Capture {
    pub fn new(
        inspections: CsvInspectionList,
        transfers: CsvTransferList,
        budget: Rc<Cell<usize>>,
    ) -> Result<Self, &'static str> {
        if inspections
            .checked_heap_bytes()
            .is_none_or(|bytes| bytes > MAX_CSV_LIST_BYTES)
            || transfers
                .checked_heap_bytes()
                .is_none_or(|bytes| bytes > MAX_CSV_LIST_BYTES)
            || inspections
                .encoded_bytes()
                .is_none_or(|bytes| bytes > MAX_CSV_LIST_BYTES)
            || transfers
                .encoded_bytes()
                .is_none_or(|bytes| bytes > MAX_CSV_LIST_BYTES)
        {
            return Err("CSV observation exceeds its bounds; previous observation retained");
        }
        let active = transfers.jobs.iter().filter(|job| !releasable(job)).count();
        if active > MAX_CSV_ACTIVE
            || transfers.jobs.len() - active > MAX_CSV_TERMINAL
            || transfers.jobs.iter().enumerate().any(|(index, job)| {
                !releasable(job)
                    && transfers.jobs[..index]
                        .iter()
                        .any(|other| !releasable(other) && other.connection_id == job.connection_id)
            })
        {
            return Err(
                "CSV observation violates job admission limits; previous observation retained",
            );
        }
        Ok(Self {
            inspections,
            transfers,
            _lease: Lease::new(budget, 1024 * 1024)?,
        })
    }
    pub fn matches(&self, inspections: &CsvInspectionList, transfers: &CsvTransferList) -> bool {
        self.inspections == *inspections && self.transfers == *transfers
    }
    pub fn inspections(&self) -> &[CsvInspectionObservation] {
        &self.inspections.inspections
    }
    pub fn jobs(&self) -> &[CsvTransferObservation] {
        &self.transfers.jobs
    }
    pub fn row(&self, index: usize) -> Option<&CsvTransferObservation> {
        self.jobs().get(index)
    }
    #[cfg(test)]
    pub fn key(&self, index: usize) -> Option<CsvTransferAttemptId> {
        Some(self.row(index)?.attempt_id)
    }
    pub fn index_for_key(&self, key: CsvTransferAttemptId) -> Option<usize> {
        self.jobs().iter().position(|job| job.attempt_id == key)
    }
    pub fn inspection(&self, key: CsvInspectionId) -> Option<&CsvInspectionObservation> {
        self.inspections()
            .iter()
            .find(|row| row.inspection_id == key)
    }
    pub fn active_on(&self, connection: &str) -> bool {
        self.jobs()
            .iter()
            .any(|job| job.connection_id == connection && !releasable(job))
    }
    pub fn unknown_import_on(&self, connection: &str) -> bool {
        self.jobs().iter().any(|job| {
            job.connection_id == connection
                && job.direction == CsvDirection::Import
                && job.effect == CsvEffect::Unknown
        })
    }
    pub fn has_active(&self) -> bool {
        self.jobs().iter().any(|job| !releasable(job))
            || self.inspections().iter().any(|row| {
                matches!(
                    row.phase,
                    CsvInspectionPhase::Preparing | CsvInspectionPhase::Cancelling
                ) || row.cleanup != CsvCleanup::Complete
            })
    }
    pub fn import_revision(&self) -> u64 {
        self.transfers.import_change_revision
    }
    pub fn import_invalidation<'a>(
        &'a self,
        tracker: &ImportChanges,
    ) -> Option<ImportInvalidation<'a>> {
        let mut revisions = [(0, ""); MAX_CSV_ACTIVE + MAX_CSV_TERMINAL];
        let mut count = 0;
        for job in self.jobs() {
            if let Some(revision) = job.import_change_revision {
                revisions[count] = (revision, job.connection_id.as_str());
                count += 1;
            }
        }
        tracker.plan(self.import_revision(), &revisions[..count])
    }
    pub fn row_label(&self, index: usize) -> Option<String> {
        let job = self.row(index)?;
        Some(format!(
            "{} · {} · {}",
            job.file_name,
            if job.workbook.is_some() {
                "Import XLSX"
            } else {
                direction_label(job.direction)
            },
            phase_label(job.phase)
        ))
    }
    pub fn details(&self, index: usize) -> Option<String> {
        let job = self.row(index)?;
        let outcome = match job.effect {
            CsvEffect::NotApplied => "No import commit or export publication observed",
            CsvEffect::Pending => "Commit or publication outcome is pending",
            CsvEffect::Succeeded => "Commit or publication succeeded",
            CsvEffect::Unknown => "Outcome unknown; inspect the target before another transfer",
        };
        let cleanup = match job.cleanup {
            CsvCleanup::Pending => "Cleanup pending; admission remains held",
            CsvCleanup::Complete => "Cleanup complete",
            CsvCleanup::Failed => "Cleanup could not be established; admission remains held",
        };
        let diagnostic = job.diagnostic.as_ref().map_or_else(String::new, |value| {
            format!(
                "Operation: {}\nRecord: {}\nColumn: {}\nSQLSTATE: {}\n{}{}",
                value.operation.as_deref().unwrap_or("Unavailable"),
                count_label(value.record),
                value
                    .column
                    .map_or_else(|| "Unavailable".into(), |value| value.to_string()),
                value.sqlstate.as_deref().unwrap_or("Unavailable"),
                value.reason,
                match value.export_limit {
                    Some(CsvExportLimit::Field) => "\nExport field exceeds its bound",
                    Some(CsvExportLimit::Record) => "\nExport record exceeds its bound",
                    None => "",
                }
            )
        });
        let provenance = job
            .workbook
            .as_ref()
            .map_or_else(String::new, workbook_summary);
        Some(format!(
            "{} · {}\nConnection: {}\nTarget: {}.{}\nAttempt: {}\nState: {}\nStarted: {}\nFinished: {}\nSource size: {}\nBytes processed: {}\nRows processed: {}\nRows committed: {}\n{}\n{}\n{}\n{}\n{}\n{}",
            if job.workbook.is_some() {
                "Import XLSX"
            } else {
                direction_label(job.direction)
            },
            job.file_name,
            job.connection_id,
            job.target.schema,
            job.target.table,
            job.attempt_id,
            phase_label(job.phase),
            job.started_at,
            job.finished_at.as_deref().unwrap_or("Not observed"),
            count_label(job.total_bytes),
            job.bytes_processed,
            count_label(job.rows_processed),
            count_label(job.rows_committed),
            outcome,
            cleanup,
            job.failure
                .map_or_else(String::new, |error| error.to_string()),
            diagnostic,
            provenance,
            if job.direction == CsvDirection::Import {
                "Import appends in one transaction. Cancellation cannot undo committed rows or sequence/external trigger effects."
            } else {
                "Export covers the whole committed relation, including partitions. It excludes grid filters and staged edits; row order is unspecified. Completion requires publication."
            }
        ))
    }
    pub fn limits(&self) -> &'static str {
        "This session: eight inspections for five minutes, four active transfers with one per connection, and 32 terminal jobs for one hour. Missing or expired observations do not prove a queued start did not happen."
    }
}
fn count_label(value: Option<u64>) -> String {
    value.map_or_else(|| "Unavailable".into(), |value| value.to_string())
}
pub fn releasable(job: &CsvTransferObservation) -> bool {
    job.phase.terminal() && job.cleanup == CsvCleanup::Complete
}
pub fn direction_label(direction: CsvDirection) -> &'static str {
    match direction {
        CsvDirection::Import => "Import CSV",
        CsvDirection::Export => "Export CSV",
    }
}
pub fn phase_label(phase: CsvTransferPhase) -> &'static str {
    match phase {
        CsvTransferPhase::AwaitingConfirmation => "Awaiting confirmation",
        CsvTransferPhase::Preparing => "Preparing",
        CsvTransferPhase::Running => "Running",
        CsvTransferPhase::Cancelling => "Cancelling; waiting for owned work",
        CsvTransferPhase::Finalizing => {
            "Finalizing; commit or publication may already have happened"
        }
        CsvTransferPhase::Completed => "Completed",
        CsvTransferPhase::Cancelled => "Cancelled",
        CsvTransferPhase::Failed => "Failed",
        CsvTransferPhase::OutcomeUnknown => "Outcome unknown",
    }
}

/// Backend-lifetime highwater survives job release and expiry. Missing revision
/// coverage invalidates all PostgreSQL mutation sources, without dropping SQL
/// sessions, manual transactions or staged recovery intent.
#[derive(Default)]
pub struct ImportChanges {
    acknowledged: u64,
}
pub struct ImportInvalidation<'a> {
    from: u64,
    to: u64,
    all: bool,
    connections: [Option<&'a str>; MAX_CSV_ACTIVE + MAX_CSV_TERMINAL],
}
impl ImportInvalidation<'_> {
    pub fn connections(&self) -> Option<impl Iterator<Item = &str>> {
        (!self.all).then(|| self.connections.iter().flatten().copied())
    }
}
impl ImportChanges {
    pub fn plan<'a>(
        &self,
        highwater: u64,
        revisions: &[(u64, &'a str)],
    ) -> Option<ImportInvalidation<'a>> {
        if highwater <= self.acknowledged {
            return None;
        }
        let mut plan = ImportInvalidation {
            from: self.acknowledged,
            to: highwater,
            all: false,
            connections: [None; MAX_CSV_ACTIVE + MAX_CSV_TERMINAL],
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
    /// Caller acknowledges only after applying every targeted invalidation.
    pub fn acknowledge(&mut self, plan: ImportInvalidation<'_>) -> Result<(), &'static str> {
        if plan.from != self.acknowledged {
            return Err("CSV import invalidation plan is stale");
        }
        self.acknowledged = plan.to;
        Ok(())
    }
}

/// Original workbook identity stays distinct from canonical transfer progress.
pub fn workbook_summary(source: &CsvWorkbookSource) -> String {
    format!(
        "XLSX sheet {}: {}\nWorkbook bytes: {}\nCanonical CSV bytes: {} (transfer progress uses these bytes)\nData rows: {}\nDetected header: {}\nCached formula values: {}\nNULL token: {:?}\nDates remain Excel serial text. Formulas are not recalculated. The owned snapshot, not the original pathname, supplies this import.",
        source.sheet_index + 1,
        source.sheet_name,
        source.workbook_bytes,
        source.canonical_bytes,
        source.rows,
        source.header_detected,
        source.cached_formula_cells,
        source.null_token
    )
}
