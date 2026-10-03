//! Workbook handles carry bounded metadata and authority, never source files.
use super::*;

impl CsvStore {
    pub fn workbook(&self) -> Option<&CsvWorkbook> {
        self.workbook.as_ref().map(|(workbook, _)| workbook)
    }
    pub fn load_workbook(&mut self, id: CsvInspectionId, cx: &mut Context<Self>) {
        if self.owned.get(&id).is_none_or(OwnedInspection::abandoned) {
            self.fail("This setup no longer owns the workbook", cx);
            return;
        }
        self.send(|request| CsvCommand::LoadWorkbook(request, id), cx);
    }
    pub fn select_sheet(
        &mut self,
        id: CsvInspectionId,
        index: u16,
        cx: &mut Context<Self>,
    ) -> bool {
        if self.pending.is_some() || self.owned.get(&id).is_none_or(OwnedInspection::abandoned) {
            self.fail(
                "Wait for current observations before selecting a worksheet",
                cx,
            );
            return false;
        }
        let Some(workbook) = self
            .workbook()
            .filter(|book| book.data().inspection_id == id)
        else {
            self.fail("Load the current workbook before selecting a worksheet", cx);
            return false;
        };
        if !workbook
            .data()
            .sheets
            .iter()
            .any(|sheet| sheet.index == index)
        {
            self.fail(
                "The selected worksheet no longer belongs to this workbook",
                cx,
            );
            return false;
        }
        let workbook = workbook.clone();
        if !self.send(
            |request| CsvCommand::SelectSheet(request, workbook, index),
            cx,
        ) {
            return false;
        }
        // The backend invalidates the exact previous revision. Do not keep its
        // mapping, review or sheet-selection authority while conversion runs.
        self.workbook = None;
        self.inspection = None;
        self.clear_tokens();
        true
    }
    pub(super) fn accept_workbook(&mut self, workbook: CsvWorkbook) {
        let id = workbook.data().inspection_id;
        if self.owned.get(&id).is_none_or(OwnedInspection::abandoned) {
            return;
        }
        if workbook.retained_bytes() > MAX_CSV_INSPECTION_BYTES
            || workbook
                .data()
                .checked_heap_bytes()
                .is_none_or(|bytes| bytes > MAX_CSV_INSPECTION_BYTES)
        {
            self.message = Some("Workbook metadata exceeds its retained allowance".into());
            return;
        }
        match Lease::inspection(self.budget.clone()) {
            Ok(lease) => {
                self.workbook = Some((workbook, lease));
                self.message = None;
            }
            Err(error) => self.message = Some(error.into()),
        }
    }
}
