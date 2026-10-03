//! Object-DDL review state. Durable descriptions are recovery, never
//! executable authority; every attempt starts from a fresh observation.
use dbunk_lib::backend::{
    WorkspaceApplyState, WorkspaceObjectDdl,
    object_ddl::{
        MAX_OBJECT_DDL_RECEIPT_BYTES, ObjectDdlError, ObjectDdlOperation, ObjectDdlOutcome,
        ObjectDdlReceipt, ObjectDdlRequest, ObjectDdlReview, PgObjectKind, PgObjectRef,
    },
};
use std::{cell::Cell, rc::Rc};
pub const ALLOWANCE: usize = 2 * 1024 * 1024;
/// Review tokens include the bounded drop-impact evidence.
pub const TOKEN_BYTES: usize = 768 * 1024;
pub const JOURNAL_BYTES: usize = 96 * 1024;
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

/// What the review was opened for. Selection is a request, never identity:
/// the backend observes exact OIDs and the review binds to those.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Purpose {
    Drop(PgObjectRef),
    CreateView { schema: String },
}
impl Purpose {
    pub fn drop(reference: PgObjectRef) -> Result<Self, &'static str> {
        if matches!(reference.kind, PgObjectKind::Extension) {
            return Err("Dropping extensions is not supported here");
        }
        let probe = ObjectDdlRequest {
            operations: vec![ObjectDdlOperation::DropObject {
                reference: reference.clone(),
                cascade: false,
            }],
        };
        probe
            .validate()
            .map_err(|_| "Select a schema, relation, sequence, routine, type or domain row")?;
        Ok(Self::Drop(reference))
    }
    pub fn create_view(schema: String) -> Result<Self, &'static str> {
        if schema.is_empty() || schema.len() > 63 || schema.contains('\0') {
            return Err("Select a schema row, or an object inside the target schema");
        }
        Ok(Self::CreateView { schema })
    }
    /// The purpose a restored journal described; used only to prefill a draft.
    pub fn from_operations(operations: &[ObjectDdlOperation]) -> Option<Self> {
        match operations {
            [ObjectDdlOperation::DropObject { reference, .. }] => {
                Some(Self::Drop(reference.clone()))
            }
            [
                ObjectDdlOperation::CreateView { schema, .. }
                | ObjectDdlOperation::CreateMaterializedView { schema, .. },
            ] => Some(Self::CreateView {
                schema: schema.clone(),
            }),
            _ => None,
        }
    }
    pub fn label(&self) -> String {
        match self {
            Self::Drop(reference) => {
                let schema = reference
                    .schema
                    .as_deref()
                    .map(|s| format!("{s:?}."))
                    .unwrap_or_default();
                let arguments = reference
                    .identity_args
                    .as_deref()
                    .map(|a| format!("({a})"))
                    .unwrap_or_default();
                format!(
                    "Drop {:?} {schema}{:?}{arguments}",
                    reference.kind, reference.name
                )
            }
            Self::CreateView { schema } => format!("Create view in schema {schema:?}"),
        }
    }
}

/// Local draft choices. Converting validates bounds; the backend still
/// regenerates and revalidates at its trust boundary.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Draft {
    pub cascade: bool,
    pub materialized: bool,
    pub or_replace: bool,
    pub with_data: bool,
}
impl Draft {
    pub fn from_operations(operations: &[ObjectDdlOperation]) -> (Self, String, String) {
        match operations {
            [ObjectDdlOperation::DropObject { cascade, .. }] => (
                Self {
                    cascade: *cascade,
                    ..Self::default()
                },
                String::new(),
                String::new(),
            ),
            [
                ObjectDdlOperation::CreateView {
                    name,
                    or_replace,
                    sql_body,
                    ..
                },
            ] => (
                Self {
                    or_replace: *or_replace,
                    ..Self::default()
                },
                name.clone(),
                sql_body.clone(),
            ),
            [
                ObjectDdlOperation::CreateMaterializedView {
                    name,
                    sql_body,
                    with_data,
                    ..
                },
            ] => (
                Self {
                    materialized: true,
                    with_data: *with_data,
                    ..Self::default()
                },
                name.clone(),
                sql_body.clone(),
            ),
            _ => (Self::default(), String::new(), String::new()),
        }
    }
    pub fn operations(
        &self,
        purpose: &Purpose,
        name: String,
        body: String,
    ) -> Result<Vec<ObjectDdlOperation>, &'static str> {
        let operation = match purpose {
            Purpose::Drop(reference) => ObjectDdlOperation::DropObject {
                reference: reference.clone(),
                cascade: self.cascade,
            },
            Purpose::CreateView { schema } if self.materialized => {
                ObjectDdlOperation::CreateMaterializedView {
                    schema: schema.clone(),
                    name,
                    sql_body: body,
                    with_data: self.with_data,
                }
            }
            Purpose::CreateView { schema } => ObjectDdlOperation::CreateView {
                schema: schema.clone(),
                name,
                or_replace: self.or_replace,
                sql_body: body,
            },
        };
        let operations = vec![operation];
        ObjectDdlRequest {
            operations: operations.clone(),
        }
        .validate()
        .map_err(|_| {
            "Name requires 1-63 UTF-8 bytes; the SQL body requires 1 byte to 16 KiB; NUL is unsupported"
        })?;
        Ok(operations)
    }
}

pub enum Settlement {
    Applied,
    /// A committed prefix (or residue) is known; nothing is in flight.
    Partial,
    Staged,
    Unknown,
}
pub struct Recovery {
    connection: String,
    journal: Option<WorkspaceObjectDdl>,
}
impl Recovery {
    pub fn new(
        connection: String,
        journal: Option<WorkspaceObjectDdl>,
    ) -> Result<Self, &'static str> {
        if connection.is_empty()
            || connection.len() > 256
            || connection.capacity() > 1024
            || journal
                .as_ref()
                .is_some_and(|j| j.validate().is_err() || j.retained_bytes() > JOURNAL_BYTES)
        {
            return Err("Object-change recovery exceeds its validated bounds");
        }
        Ok(Self {
            connection,
            journal,
        })
    }
    pub fn connection(&self) -> &str {
        &self.connection
    }
    pub fn journal(&self) -> Option<&WorkspaceObjectDdl> {
        self.journal.as_ref()
    }
    pub fn unknown(&self) -> bool {
        self.journal
            .as_ref()
            .is_some_and(|j| j.apply_state == WorkspaceApplyState::OutcomeUnknown)
    }
    pub fn stage(&mut self, review: &ObjectDdlReview) -> Result<(), &'static str> {
        if self.unknown() {
            return Err("Reconcile the previous unknown outcome first");
        }
        if review.retained_bytes() > TOKEN_BYTES {
            return Err("Object-change review exceeds its allowance");
        }
        let journal = WorkspaceObjectDdl::from_review(review)
            .map_err(|_| "Object-change review cannot be saved")?;
        if journal.retained_bytes() > JOURNAL_BYTES {
            return Err("Object-change journal exceeds its allowance");
        }
        self.journal = Some(journal);
        Ok(())
    }
    pub fn matches_review(&self, review: &ObjectDdlReview) -> bool {
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
    pub fn submission_error(&mut self, error: &ObjectDdlError) {
        if *error != ObjectDdlError::OutcomeUnavailable {
            self.not_sent();
        }
    }
    pub fn receipt(&mut self, receipt: &ObjectDdlReceipt) -> Settlement {
        let matches = receipt.retained_bytes() <= MAX_OBJECT_DDL_RECEIPT_BYTES * 2
            && receipt.encoded_bytes() <= MAX_OBJECT_DDL_RECEIPT_BYTES
            && receipt.connection_id == self.connection
            && self.journal.as_ref().is_some_and(|j| {
                j.validate().is_ok()
                    && j.apply_state == WorkspaceApplyState::OutcomeUnknown
                    && j.attempt_id == receipt.attempt_id
                    && j.target == receipt.target
                    && j.operations == receipt.operations
            });
        if !matches {
            self.mark_unknown();
            return Settlement::Unknown;
        }
        match &receipt.outcome {
            ObjectDdlOutcome::Applied { .. } => {
                self.journal = None;
                Settlement::Applied
            }
            ObjectDdlOutcome::NotDispatched { .. } => {
                self.not_sent();
                Settlement::Staged
            }
            outcome @ ObjectDdlOutcome::Stopped { .. } if !outcome.may_have_changed() => {
                self.not_sent();
                Settlement::Staged
            }
            ObjectDdlOutcome::Stopped { .. } => {
                // Known partial effect: never re-stage the same operations.
                self.journal = None;
                Settlement::Partial
            }
            ObjectDdlOutcome::OutcomeUnknown { .. } => Settlement::Unknown,
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
#[cfg(test)]
mod tests;
