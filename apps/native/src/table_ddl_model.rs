//! Durable descriptions are recovery, never executable authority.
use dbunk_lib::backend::{
    WorkspaceApplyState, WorkspaceTableDdl,
    table_ddl::{
        MAX_TABLE_DDL_RECEIPT_BYTES, TableDdlDescription, TableDdlError, TableDdlIntent,
        TableDdlOutcome, TableDdlReceipt, TableDdlRequest, TableDdlReview,
    },
};
use std::{cell::Cell, rc::Rc};
pub const ALLOWANCE: usize = 1024 * 1024;
pub const TOKEN_BYTES: usize = 64 * 1024;
const SHARED_BYTES: usize = 128 * 1024 * 1024;
/// Presentation allowance, not a bound on process RSS or transient editor input.
pub struct Lease(Rc<Cell<usize>>);
impl Lease {
    pub fn admit(budget: Rc<Cell<usize>>) -> Option<Self> {
        let total = budget.get().checked_add(ALLOWANCE)?;
        if total > SHARED_BYTES {
            return None;
        }
        budget.set(total);
        Some(Self(budget))
    }
}
impl Drop for Lease {
    fn drop(&mut self) {
        self.0.set(self.0.get().saturating_sub(ALLOWANCE));
    }
}
#[derive(Clone)]
pub struct Selection {
    request: TableDdlRequest,
    attnum: Option<i16>,
}
impl Selection {
    pub fn new(request: TableDdlRequest, attnum: Option<i16>) -> Result<Self, &'static str> {
        request
            .validate()
            .map_err(|_| "Invalid observed table target")?;
        if request.expected.is_none()
            || request.column.is_some() != attnum.is_some()
            || attnum.is_some_and(|n| n <= 0)
        {
            return Err("Select an exact table or column identity from Structure");
        }
        Ok(Self { request, attnum })
    }
    pub fn request(&self) -> TableDdlRequest {
        self.request.clone()
    }
    pub fn attnum(&self) -> Option<i16> {
        self.attnum
    }
    pub fn matches(&self, target: &TableDdlDescription) -> bool {
        target.checked_heap_bytes().is_some()
            && self.request.expected == Some(target.identity)
            && self.request.schema == target.schema
            && self.request.table == target.table
            && self.request.column.as_deref() == target.column.as_ref().map(|c| c.name.as_str())
            && self.attnum == target.column.as_ref().map(|c| c.attnum)
    }
}
pub enum Settlement {
    Applied,
    Staged,
    Unknown,
}
pub struct Recovery {
    connection: String,
    journal: Option<WorkspaceTableDdl>,
}
impl Recovery {
    pub fn new(
        connection: String,
        journal: Option<WorkspaceTableDdl>,
    ) -> Result<Self, &'static str> {
        if connection.is_empty()
            || connection.len() > 256
            || connection.capacity() > 1024
            || journal
                .as_ref()
                .is_some_and(|j| j.validate().is_err() || j.retained_bytes() > 48 * 1024)
        {
            return Err("Table-change recovery exceeds its validated bounds");
        }
        Ok(Self {
            connection,
            journal,
        })
    }
    pub fn connection(&self) -> &str {
        &self.connection
    }
    pub fn journal(&self) -> Option<&WorkspaceTableDdl> {
        self.journal.as_ref()
    }
    pub fn selection(&self) -> Result<Option<Selection>, &'static str> {
        self.journal
            .as_ref()
            .map(|j| {
                Selection::new(
                    j.target.request(),
                    j.target.column.as_ref().map(|c| c.attnum),
                )
            })
            .transpose()
    }
    pub fn unknown(&self) -> bool {
        self.journal
            .as_ref()
            .is_some_and(|j| j.apply_state == WorkspaceApplyState::OutcomeUnknown)
    }
    pub fn stage(&mut self, review: &TableDdlReview) -> Result<(), &'static str> {
        if self.unknown() {
            return Err("Reconcile the previous unknown outcome first");
        }
        if review.retained_bytes() > TOKEN_BYTES {
            return Err("Table-change review exceeds its allowance");
        }
        let journal = WorkspaceTableDdl::from_review(review)
            .map_err(|_| "Table-change review cannot be saved")?;
        if journal.retained_bytes() > 48 * 1024 {
            return Err("Table-change journal exceeds its allowance");
        }
        self.journal = Some(journal);
        Ok(())
    }
    pub fn matches_review(&self, review: &TableDdlReview) -> bool {
        review.retained_bytes() <= TOKEN_BYTES
            && self
                .journal
                .as_ref()
                .is_some_and(|j| j.matches_review(review))
    }
    pub fn mark_unknown(&mut self) -> bool {
        let Some(journal) = &mut self.journal else {
            return false;
        };
        journal.apply_state = WorkspaceApplyState::OutcomeUnknown;
        true
    }
    pub fn not_sent(&mut self) {
        if let Some(journal) = &mut self.journal {
            journal.apply_state = WorkspaceApplyState::Staged;
        }
    }
    pub fn submission_error(&mut self, error: &TableDdlError) {
        if *error != TableDdlError::OutcomeUnavailable {
            self.not_sent();
        }
    }
    pub fn receipt(&mut self, receipt: &TableDdlReceipt) -> Settlement {
        let matches = receipt.retained_bytes() <= MAX_TABLE_DDL_RECEIPT_BYTES
            && receipt.encoded_bytes() <= MAX_TABLE_DDL_RECEIPT_BYTES
            && receipt.connection_id == self.connection
            && self.journal.as_ref().is_some_and(|j| {
                j.validate().is_ok()
                    && j.apply_state == WorkspaceApplyState::OutcomeUnknown
                    && j.attempt_id == receipt.attempt_id
                    && j.target == receipt.target
                    && j.intent == receipt.intent
            });
        if !matches {
            self.mark_unknown();
            return Settlement::Unknown;
        }
        match receipt.outcome {
            TableDdlOutcome::Applied { .. } => {
                self.journal = None;
                Settlement::Applied
            }
            TableDdlOutcome::NotDispatched { .. } | TableDdlOutcome::RolledBack { .. } => {
                self.not_sent();
                Settlement::Staged
            }
            TableDdlOutcome::OutcomeUnknown { .. } => Settlement::Unknown,
        }
    }
    /// Caller requires an explicit reconciliation acknowledgement for Unknown.
    pub fn discard(&mut self, reconciled: bool) -> bool {
        if self.unknown() && !reconciled {
            return false;
        }
        self.journal = None;
        true
    }
}
pub fn intent(
    rename: bool,
    remove_comment: bool,
    value: String,
) -> Result<TableDdlIntent, &'static str> {
    let intent = if rename {
        TableDdlIntent::Rename { new_name: value }
    } else {
        TableDdlIntent::SetComment {
            comment: if remove_comment { None } else { Some(value) },
        }
    };
    intent
        .checked_heap_bytes()
        .ok_or("Rename requires 1–63 UTF-8 bytes; comments allow 4096 bytes; NUL is unsupported")?;
    Ok(intent)
}
#[cfg(test)]
mod tests;
