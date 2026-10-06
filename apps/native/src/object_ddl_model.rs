//! Object-DDL review state. Durable descriptions are recovery, never
//! executable authority; every attempt starts from a fresh observation.
use dbunk_lib::backend::{
    WorkspaceApplyState, WorkspaceObjectDdl,
    object_ddl::{
        MAX_OBJECT_DDL_RECEIPT_BYTES, ObjectDdlEnumPosition, ObjectDdlError, ObjectDdlOperation,
        ObjectDdlOutcome, ObjectDdlReceipt, ObjectDdlRequest, ObjectDdlReview, PgObjectKind,
        PgObjectRef,
    },
};
mod form;
pub use form::{
    EnumPlacement, IndexMethod, derived_index_name, index_columns_text, parse_index_columns,
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
    /// A plain table; the backend claims it as an existing `Table`.
    CreateIndex { schema: String, table: String },
    /// An enum type; the backend claims it as an existing `Type`.
    AddEnumValue { schema: String, name: String },
}
fn valid_name(value: &str) -> bool {
    !value.is_empty() && value.len() <= 63 && !value.contains('\0')
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
    pub fn create_index(schema: String, table: String) -> Result<Self, &'static str> {
        if !valid_name(&schema) || !valid_name(&table) {
            return Err("Select a table row to create an index on it");
        }
        Ok(Self::CreateIndex { schema, table })
    }
    pub fn add_enum_value(schema: String, name: String) -> Result<Self, &'static str> {
        if !valid_name(&schema) || !valid_name(&name) {
            return Err("Select an enum type row to add a value to it");
        }
        Ok(Self::AddEnumValue { schema, name })
    }
    /// Index and enum drafts edit their options through the form controls
    /// rather than the shared Mode/Option toggles.
    pub fn uses_form(&self) -> bool {
        matches!(self, Self::CreateIndex { .. } | Self::AddEnumValue { .. })
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
            // A method this build cannot represent stays inspect-only.
            [
                ObjectDdlOperation::CreateIndex {
                    schema,
                    table,
                    method,
                    ..
                },
            ] if IndexMethod::parse(method).is_some() => Some(Self::CreateIndex {
                schema: schema.clone(),
                table: table.clone(),
            }),
            [ObjectDdlOperation::AddEnumValue { schema, name, .. }] => Some(Self::AddEnumValue {
                schema: schema.clone(),
                name: name.clone(),
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
            Self::CreateIndex { schema, table } => {
                format!("Create index on table {schema:?}.{table:?}")
            }
            Self::AddEnumValue { schema, name } => {
                format!("Add value to enum {schema:?}.{name:?}")
            }
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
    pub unique: bool,
    pub concurrently: bool,
    pub method: IndexMethod,
    pub placement: EnumPlacement,
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
            [
                ObjectDdlOperation::CreateIndex {
                    name,
                    unique,
                    method,
                    columns,
                    concurrently,
                    ..
                },
            ] => (
                Self {
                    unique: *unique,
                    concurrently: *concurrently,
                    method: IndexMethod::parse(method).unwrap_or_default(),
                    ..Self::default()
                },
                name.clone(),
                index_columns_text(columns),
            ),
            [ObjectDdlOperation::AddEnumValue {
                value, position, ..
            }] => {
                let (placement, neighbor) = match position {
                    None => (EnumPlacement::End, String::new()),
                    Some(ObjectDdlEnumPosition::Before { neighbor }) => {
                        (EnumPlacement::Before, neighbor.clone())
                    }
                    Some(ObjectDdlEnumPosition::After { neighbor }) => {
                        (EnumPlacement::After, neighbor.clone())
                    }
                };
                (
                    Self {
                        placement,
                        ..Self::default()
                    },
                    value.clone(),
                    neighbor,
                )
            }
            _ => (Self::default(), String::new(), String::new()),
        }
    }
    /// `name` and `body` are the two draft fields: view name and SQL body,
    /// index name (empty derives one) and column list, or the new enum label
    /// and the neighbor label for BEFORE/AFTER.
    pub fn operations(
        &self,
        purpose: &Purpose,
        name: String,
        body: String,
    ) -> Result<Vec<ObjectDdlOperation>, &'static str> {
        let operation = match purpose {
            Purpose::CreateIndex { schema, table } => {
                self.create_index(schema, table, name, &body)?
            }
            Purpose::AddEnumValue {
                schema,
                name: enum_name,
            } => self.add_enum_value(schema, enum_name, name, body)?,
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
            if purpose.uses_form() {
                "Names and labels require 1-63 UTF-8 bytes; NUL is unsupported"
            } else {
                "Name requires 1-63 UTF-8 bytes; the SQL body requires 1 byte to 16 KiB; NUL is unsupported"
            }
        })?;
        Ok(operations)
    }
    fn create_index(
        &self,
        schema: &str,
        table: &str,
        name: String,
        columns: &str,
    ) -> Result<ObjectDdlOperation, &'static str> {
        let columns = parse_index_columns(columns)?;
        if !self.method.multicolumn() && columns.len() > 1 {
            return Err("hash and spgist indexes take exactly one column");
        }
        if self.unique && !self.method.ordered() {
            return Err("Only btree indexes can be UNIQUE");
        }
        if !self.method.ordered() && columns.iter().any(|column| column.descending) {
            return Err("Only btree indexes accept DESC columns");
        }
        let trimmed = name.trim();
        let name = if trimmed.is_empty() {
            derived_index_name(table, &columns)
        } else if trimmed.len() > 63 {
            return Err("Index names are limited to 63 bytes");
        } else if trimmed.contains('\0') {
            return Err("NUL is unsupported in index names");
        } else {
            trimmed.to_owned()
        };
        Ok(ObjectDdlOperation::CreateIndex {
            schema: schema.to_owned(),
            table: table.to_owned(),
            name,
            unique: self.unique,
            method: self.method.as_str().to_owned(),
            columns,
            concurrently: self.concurrently,
        })
    }
    fn add_enum_value(
        &self,
        schema: &str,
        name: &str,
        value: String,
        neighbor: String,
    ) -> Result<ObjectDdlOperation, &'static str> {
        if value.trim().is_empty() {
            return Err("Enter the new enum label");
        }
        if value.len() > 63 {
            return Err("Enum labels are limited to 63 bytes");
        }
        if value.contains('\0') || neighbor.contains('\0') {
            return Err("NUL is unsupported in enum labels");
        }
        let position = match self.placement {
            EnumPlacement::End => None,
            placement => {
                if neighbor.is_empty() {
                    return Err("Enter the existing label to place the new one BEFORE or AFTER");
                }
                if neighbor.len() > 63 {
                    return Err("Enum labels are limited to 63 bytes");
                }
                if neighbor == value {
                    return Err("The neighbor label must differ from the new label");
                }
                Some(if placement == EnumPlacement::Before {
                    ObjectDdlEnumPosition::Before { neighbor }
                } else {
                    ObjectDdlEnumPosition::After { neighbor }
                })
            }
        };
        Ok(ObjectDdlOperation::AddEnumValue {
            schema: schema.to_owned(),
            name: name.to_owned(),
            value,
            position,
        })
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
