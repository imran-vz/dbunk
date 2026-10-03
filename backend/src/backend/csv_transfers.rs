//! Profile-owned CSV inspections and jobs. Synchronous registration precedes all
//! owned preparation. Backend workspace permits are independent of UI budgets.
mod artifacts;
mod cleanup;
mod prepare;
mod workbook;
pub use workbook::{
    CsvWorkbook, CsvWorkbookData, CsvWorkbookSheet, CsvWorkbookSource, CsvWorkbookVisibility,
};
mod registry;
mod types;
mod types_bounds;
use super::Backend;
pub(super) use registry::Registry;
use std::{
    path::PathBuf,
    sync::{Arc, Weak},
};
pub use types::*;
pub(super) struct InspectionReady {
    data: CsvInspectionData,
    core: Arc<crate::postgres::transfer::runner::Review>,
    requires_confirmation: bool,
    retained: usize,
    _permit: tokio::sync::OwnedSemaphorePermit,
}
#[derive(Clone)]
pub struct CsvInspection {
    owner: Weak<super::Inner>,
    id: CsvInspectionId,
    revision: uuid::Uuid,
    ready: Arc<InspectionReady>,
}
impl CsvInspection {
    pub fn data(&self) -> &CsvInspectionData {
        &self.ready.data
    }
    pub fn inspection_id(&self) -> CsvInspectionId {
        self.id
    }
    pub fn retained_bytes(&self) -> usize {
        self.ready.retained
    }
    pub fn belongs_to(&self, backend: &Backend) -> bool {
        self.owner.ptr_eq(&Arc::downgrade(&backend.0))
    }
}
impl std::fmt::Debug for CsvInspection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CsvInspection")
            .field("id", &self.id)
            .finish_non_exhaustive()
    }
}
struct ReviewData {
    inspection: CsvInspection,
    mapping: Vec<CsvMapping>,
    destination: Option<PathBuf>,
}
#[derive(Clone)]
pub struct CsvTransferReview {
    data: Arc<ReviewData>,
    attempt: Option<CsvTransferAttemptId>,
}
impl CsvTransferReview {
    pub fn inspection(&self) -> &CsvInspection {
        &self.data.inspection
    }
    pub fn inspection_id(&self) -> CsvInspectionId {
        self.inspection().id
    }
    pub fn attempt_id(&self) -> Option<CsvTransferAttemptId> {
        self.attempt
    }
    pub fn direction(&self) -> CsvDirection {
        self.inspection().data().direction
    }
    pub fn mapping(&self) -> &[CsvMapping] {
        &self.data.mapping
    }
    pub fn target(&self) -> &CsvConnectionTarget {
        &self.inspection().data().connection
    }
    pub fn file_name(&self) -> &str {
        match &self.data.destination {
            Some(path) => path
                .file_name()
                .and_then(|s| s.to_str())
                .expect("validated CSV destination"),
            None => self
                .inspection()
                .data()
                .file_name
                .as_deref()
                .expect("inspected CSV source"),
        }
    }
    pub fn retained_bytes(&self) -> usize {
        self.inspection()
            .retained_bytes()
            .saturating_add(std::mem::size_of::<ReviewData>())
            .saturating_add(
                self.data
                    .mapping
                    .capacity()
                    .saturating_mul(std::mem::size_of::<CsvMapping>()),
            )
            .saturating_add(
                self.data
                    .mapping
                    .iter()
                    .map(|m| m.target_column.capacity())
                    .sum::<usize>(),
            )
            .saturating_add(self.data.destination.as_ref().map_or(0, PathBuf::capacity))
    }
    pub fn belongs_to(&self, backend: &Backend) -> bool {
        self.inspection().belongs_to(backend)
    }
}
impl std::fmt::Debug for CsvTransferReview {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CsvTransferReview")
            .field("attempt", &self.attempt)
            .finish_non_exhaustive()
    }
}
pub struct CsvTransferConfirmation {
    review: CsvTransferReview,
}
impl CsvTransferConfirmation {
    pub fn review(&self) -> &CsvTransferReview {
        &self.review
    }
    pub fn attempt_id(&self) -> CsvTransferAttemptId {
        self.review.attempt.expect("confirmation registered")
    }
    pub fn retained_bytes(&self) -> usize {
        self.review.retained_bytes()
    }
    pub fn belongs_to(&self, backend: &Backend) -> bool {
        self.review.belongs_to(backend)
    }
}
#[allow(clippy::large_enum_variant)]
pub enum CsvTransferSubmission {
    NeedsConfirmation(Box<CsvTransferConfirmation>),
    Accepted(CsvTransferObservation),
}
impl Backend {
    pub fn begin_csv_inspection(
        &self,
        id: CsvInspectionId,
        connection_id: String,
        intent: CsvInspectionIntent,
    ) -> Result<CsvInspectionObservation, CsvError> {
        self.0
            .csv_transfers
            .begin_inspection(self, id, connection_id, intent)
    }
    pub fn get_csv_inspection(
        &self,
        id: CsvInspectionId,
    ) -> Result<CsvInspectionObservation, CsvError> {
        self.0
            .csv_transfers
            .get_inspection(&self.0.state.pg_transfers, id)
    }
    pub fn list_csv_inspections(
        &self,
        connection: Option<&str>,
    ) -> Result<CsvInspectionList, CsvError> {
        self.0
            .csv_transfers
            .list_inspections(&self.0.state.pg_transfers, connection)
    }
    pub fn csv_inspection(&self, id: CsvInspectionId) -> Result<CsvInspection, CsvError> {
        self.0.csv_transfers.inspection(self, id)
    }
    pub fn cancel_csv_inspection(
        &self,
        id: CsvInspectionId,
    ) -> Result<CsvInspectionObservation, CsvError> {
        self.0
            .csv_transfers
            .cancel_inspection(&self.0.state.pg_transfers, id)
    }
    pub fn release_csv_inspection(&self, id: CsvInspectionId) -> Result<(), CsvError> {
        self.0
            .csv_transfers
            .release_inspection(&self.0.state.pg_transfers, id)
    }
    pub fn review_csv_import(
        &self,
        inspection: CsvInspection,
        mapping: Vec<CsvMapping>,
    ) -> Result<CsvTransferReview, CsvError> {
        self.0
            .csv_transfers
            .review_import(self, inspection, mapping)
    }
    pub fn review_csv_export(
        &self,
        inspection: CsvInspection,
        destination: PathBuf,
    ) -> Result<CsvTransferReview, CsvError> {
        self.0
            .csv_transfers
            .review_export(self, inspection, destination)
    }
    /// A returned admission is never a commit/publication acknowledgement. A
    /// reacquired review can only reissue confirmation for its original attempt.
    pub fn begin_csv_transfer(
        &self,
        id: CsvTransferAttemptId,
        review: CsvTransferReview,
    ) -> Result<CsvTransferSubmission, CsvError> {
        self.0.csv_transfers.begin_transfer(self, id, review)
    }
    pub fn confirm_csv_transfer(
        &self,
        confirmation: CsvTransferConfirmation,
    ) -> Result<CsvTransferSubmission, CsvError> {
        self.0.csv_transfers.confirm(self, confirmation)
    }
    pub fn review_csv_transfer(
        &self,
        id: CsvTransferAttemptId,
    ) -> Result<CsvTransferReview, CsvError> {
        self.0.csv_transfers.review_transfer(id)
    }
    pub fn get_csv_transfer(
        &self,
        id: CsvTransferAttemptId,
    ) -> Result<CsvTransferObservation, CsvError> {
        self.0.csv_transfers.get(&self.0.state.pg_transfers, id)
    }
    pub fn list_csv_transfers(
        &self,
        connection: Option<&str>,
    ) -> Result<CsvTransferList, CsvError> {
        self.0
            .csv_transfers
            .list(&self.0.state.pg_transfers, connection)
    }
    pub fn cancel_csv_transfer(
        &self,
        id: CsvTransferAttemptId,
    ) -> Result<CsvTransferObservation, CsvError> {
        self.0.csv_transfers.cancel(&self.0.state.pg_transfers, id)
    }
    pub fn release_csv_transfer(&self, id: CsvTransferAttemptId) -> Result<(), CsvError> {
        self.0.csv_transfers.release(&self.0.state.pg_transfers, id)
    }
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod live_tests;

mod retirement;
pub(super) use retirement::retire_connection;
