//! Opaque workbook selection authority contains bounded metadata, never paths or
//! file owners. Selection consumes one revision; previous CSV mappings go stale.
use super::*;
use serde::Serialize;
use std::{
    mem::size_of,
    time::{Duration, Instant},
};
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub enum CsvWorkbookVisibility {
    Visible,
    Hidden,
    VeryHidden,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct CsvWorkbookSheet {
    pub index: u16,
    pub name: String,
    pub visibility: CsvWorkbookVisibility,
}
#[derive(Serialize)]
pub struct CsvWorkbookData {
    pub inspection_id: CsvInspectionId,
    pub file_name: String,
    pub workbook_bytes: u64,
    pub sheets: Vec<CsvWorkbookSheet>,
}
impl CsvWorkbookData {
    pub fn checked_heap_bytes(&self) -> Option<usize> {
        if self.sheets.is_empty()
            || self.sheets.len() > 256
            || self.file_name.len() > 1024
            || self.workbook_bytes > artifacts::MAX_WORKBOOK_BYTES
        {
            return None;
        }
        let mut n = size_of::<Self>()
            .checked_add(self.file_name.capacity())?
            .checked_add(
                self.sheets
                    .capacity()
                    .checked_mul(size_of::<CsvWorkbookSheet>())?,
            )?;
        for (i, s) in self.sheets.iter().enumerate() {
            if s.index as usize != i
                || s.name.is_empty()
                || s.name.len() > 1024
                || s.name.contains('\0')
            {
                return None;
            }
            n = n.checked_add(s.name.capacity())?;
        }
        types_bounds::encoded(self, 1024 * 1024)?;
        (n <= 1024 * 1024).then_some(n)
    }
}
#[derive(Clone, PartialEq, Eq, Serialize)]
pub struct CsvWorkbookSource {
    pub sheet_index: u16,
    pub sheet_name: String,
    pub workbook_bytes: u64,
    pub canonical_bytes: u64,
    pub null_token: String,
    pub header_detected: bool,
    pub rows: u64,
    pub cached_formula_cells: u64,
}
impl CsvWorkbookSource {
    pub fn checked_heap_bytes(&self) -> Option<usize> {
        if self.sheet_index >= 256
            || self.sheet_name.is_empty()
            || self.sheet_name.len() > 1024
            || self.sheet_name.contains('\0')
            || self.null_token.len() > 1024
            || self.null_token.contains(['\0', '\r', '\n'])
            || self.workbook_bytes > artifacts::MAX_WORKBOOK_BYTES
            || self.canonical_bytes > artifacts::MAX_CANONICAL_BYTES
            || self.rows > crate::xlsx_native::MAX_LOGICAL_CELLS
            || self.cached_formula_cells > crate::xlsx_native::MAX_LOGICAL_CELLS
        {
            return None;
        }
        let n = size_of::<Self>()
            .checked_add(self.sheet_name.capacity())?
            .checked_add(self.null_token.capacity())?;
        (n <= 4096).then_some(n)
    }
}
impl std::fmt::Debug for CsvWorkbookSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CsvWorkbookSource")
            .field("sheet_index", &self.sheet_index)
            .finish_non_exhaustive()
    }
}
#[derive(Clone)]
pub struct CsvWorkbook {
    owner: Weak<super::super::Inner>,
    id: CsvInspectionId,
    revision: uuid::Uuid,
    data: Arc<CsvWorkbookData>,
    _permit: Arc<tokio::sync::OwnedSemaphorePermit>,
}
impl CsvWorkbook {
    pub fn data(&self) -> &CsvWorkbookData {
        &self.data
    }
    pub fn inspection_id(&self) -> CsvInspectionId {
        self.id
    }
    pub fn retained_bytes(&self) -> usize {
        self.data
            .checked_heap_bytes()
            .unwrap_or(usize::MAX)
            .saturating_add(size_of::<Self>())
    }
    pub fn belongs_to(&self, backend: &Backend) -> bool {
        self.owner.ptr_eq(&Arc::downgrade(&backend.0))
    }
}
impl std::fmt::Debug for CsvWorkbook {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CsvWorkbook")
            .field("id", &self.id)
            .finish_non_exhaustive()
    }
}
impl Backend {
    pub fn csv_workbook(&self, id: CsvInspectionId) -> Result<CsvWorkbook, CsvError> {
        let state = self.0.csv_transfers.state.lock().unwrap();
        let e = state.inspections.get(&id).ok_or(CsvError::Missing)?;
        if !matches!(
            e.observation.phase,
            CsvInspectionPhase::WorkbookReady | CsvInspectionPhase::Ready
        ) || e.created.elapsed() >= Duration::from_secs(CSV_INSPECTION_TTL_SECONDS)
            || *e.cancel.borrow()
        {
            return Err(CsvError::InspectionExpired);
        }
        Ok(CsvWorkbook {
            owner: Arc::downgrade(&self.0),
            id,
            revision: e.revision,
            data: e.workbook.clone().ok_or(CsvError::InvalidRequest)?,
            _permit: e.workbook_permit.clone().ok_or(CsvError::StaleReview)?,
        })
    }
    pub fn select_csv_workbook_sheet(
        &self,
        workbook: CsvWorkbook,
        sheet_index: u16,
    ) -> Result<CsvInspectionObservation, CsvError> {
        if !workbook.belongs_to(self) {
            return Err(CsvError::ForeignReview);
        }
        let _submit = self.0.submission.lock().unwrap();
        if self.0.closing.load(std::sync::atomic::Ordering::SeqCst) {
            return Err(CsvError::Closing);
        }
        let mut state = self.0.csv_transfers.state.lock().unwrap();
        let connection = state
            .inspections
            .get(&workbook.id)
            .ok_or(CsvError::Missing)?
            .observation
            .connection_id
            .clone();
        if state.retiring(&connection) {
            return Err(CsvError::Closing);
        }
        let e = state
            .inspections
            .get_mut(&workbook.id)
            .ok_or(CsvError::Missing)?;
        if e.revision != workbook.revision
            || e.workbook
                .as_ref()
                .is_none_or(|d| !Arc::ptr_eq(d, &workbook.data))
            || !matches!(
                e.observation.phase,
                CsvInspectionPhase::WorkbookReady | CsvInspectionPhase::Ready
            )
            || *e.cancel.borrow()
            || e.created.elapsed() >= Duration::from_secs(CSV_INSPECTION_TTL_SECONDS)
        {
            return Err(CsvError::StaleReview);
        }
        if workbook.data.sheets.get(sheet_index as usize).is_none() {
            return Err(CsvError::InvalidRequest);
        }
        let admission = self
            .0
            .state
            .pg_transfers
            .admission(&connection)
            .map_err(prepare::error)?;
        let permit = self
            .0
            .csv_transfers
            .inspections
            .clone()
            .try_acquire_owned()
            .map_err(|_| CsvError::Busy)?;
        if let Some(token) = e.token.take() {
            self.0.state.pg_transfers.release_review(&token);
        }
        e.ready.take();
        e.permit = Some(permit);
        e.admission = Some(admission);
        e.revision = uuid::Uuid::new_v4();
        e.selected_sheet = Some(sheet_index);
        e.observation.phase = CsvInspectionPhase::Preparing;
        e.observation.cleanup = CsvCleanup::Pending;
        e.observation.failure = None;
        e.created = Instant::now();
        e.io = crate::postgres::transfer::native::IoContext::new(
            self.0.csv_transfers.owner.child(),
            self.0.tasks.child(),
        );
        let inner = self.0.clone();
        let id = workbook.id;
        if self
            .0
            .csv_transfers
            .owner
            .spawn(async move { prepare::inspect(inner, id).await })
            .is_err()
        {
            e.observation.phase = CsvInspectionPhase::Failed;
            e.observation.failure = Some(CsvError::Busy);
            self.0.csv_transfers.cleanup_inspection(e, id);
            return Err(CsvError::Busy);
        }
        Ok(e.observation.clone())
    }
}

#[cfg(test)]
#[path = "workbook_tests.rs"]
mod tests;
