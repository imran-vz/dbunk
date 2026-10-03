//! Durable existing-schema descriptions are recovery, never executable authority.
pub use crate::table_ddl_model::{Lease, Settlement, TOKEN_BYTES};
use dbunk_lib::backend::{
    WORKSPACE_SCHEMA_ALTER_MAX_BYTES, WorkspaceApplyState, WorkspaceSchemaAlter,
    schema_alter::{
        MAX_SCHEMA_ALTER_RECEIPT_BYTES, SchemaAlterDescription, SchemaAlterError,
        SchemaAlterIntent, SchemaAlterOutcome, SchemaAlterReceipt, SchemaAlterRequest,
        SchemaAlterReview,
    },
};

/// A catalog row names a schema; its OID is pinned by the first observation and
/// every later observation in this review must return that same identity.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Selection {
    request: SchemaAlterRequest,
}
impl Selection {
    pub fn new(request: SchemaAlterRequest) -> Result<Self, &'static str> {
        request
            .validate()
            .map_err(|_| "Invalid schema target; select a schema row in Objects")?;
        Ok(Self { request })
    }
    pub fn request(&self) -> SchemaAlterRequest {
        self.request.clone()
    }
    #[cfg(test)]
    pub fn schema(&self) -> &str {
        &self.request.schema
    }
    #[cfg(test)]
    pub fn pinned(&self) -> bool {
        self.request.expected.is_some()
    }
    pub fn matches(&self, target: &SchemaAlterDescription) -> bool {
        target.checked_heap_bytes().is_some()
            && self.request.schema == target.schema
            && self
                .request
                .expected
                .is_none_or(|identity| identity == target.identity)
    }
    /// Pins the observed OID. Refuses a different identity once pinned.
    pub fn pin(&mut self, target: &SchemaAlterDescription) -> bool {
        if !self.matches(target) {
            return false;
        }
        self.request.expected = Some(target.identity);
        true
    }
}

pub struct Recovery {
    connection: String,
    journal: Option<WorkspaceSchemaAlter>,
}
impl Recovery {
    pub fn new(
        connection: String,
        journal: Option<WorkspaceSchemaAlter>,
    ) -> Result<Self, &'static str> {
        if connection.is_empty()
            || connection.len() > 256
            || connection.capacity() > 1024
            || journal.as_ref().is_some_and(|j| {
                j.validate().is_err() || j.retained_bytes() > WORKSPACE_SCHEMA_ALTER_MAX_BYTES
            })
        {
            return Err("Schema-change recovery exceeds its validated bounds");
        }
        Ok(Self {
            connection,
            journal,
        })
    }
    pub fn connection(&self) -> &str {
        &self.connection
    }
    pub fn journal(&self) -> Option<&WorkspaceSchemaAlter> {
        self.journal.as_ref()
    }
    /// A restored journal yields a fresh pinned selection only; review and
    /// Apply always require a new observation of that same identity.
    pub fn selection(&self) -> Result<Option<Selection>, &'static str> {
        self.journal
            .as_ref()
            .map(|j| Selection::new(j.target.request()))
            .transpose()
    }
    pub fn unknown(&self) -> bool {
        self.journal
            .as_ref()
            .is_some_and(|j| j.apply_state == WorkspaceApplyState::OutcomeUnknown)
    }
    pub fn stage(&mut self, review: &SchemaAlterReview) -> Result<(), &'static str> {
        if self.unknown() {
            return Err("Reconcile the previous unknown outcome first");
        }
        if review.retained_bytes() > TOKEN_BYTES {
            return Err("Schema-change review exceeds its allowance");
        }
        let journal = WorkspaceSchemaAlter::from_review(review)
            .map_err(|_| "Schema-change review cannot be saved")?;
        if journal.retained_bytes() > WORKSPACE_SCHEMA_ALTER_MAX_BYTES {
            return Err("Schema-change journal exceeds its allowance");
        }
        self.journal = Some(journal);
        Ok(())
    }
    pub fn matches_review(&self, review: &SchemaAlterReview) -> bool {
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
    pub fn submission_error(&mut self, error: &SchemaAlterError) {
        if *error != SchemaAlterError::OutcomeUnavailable {
            self.not_sent();
        }
    }
    pub fn receipt(&mut self, receipt: &SchemaAlterReceipt) -> Settlement {
        let matches = receipt.retained_bytes() <= MAX_SCHEMA_ALTER_RECEIPT_BYTES
            && receipt.encoded_bytes() <= MAX_SCHEMA_ALTER_RECEIPT_BYTES
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
            SchemaAlterOutcome::Applied { .. } => {
                self.journal = None;
                Settlement::Applied
            }
            SchemaAlterOutcome::NotDispatched { .. } | SchemaAlterOutcome::RolledBack { .. } => {
                self.not_sent();
                Settlement::Staged
            }
            SchemaAlterOutcome::OutcomeUnknown { .. } => Settlement::Unknown,
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
) -> Result<SchemaAlterIntent, &'static str> {
    let intent = if rename {
        SchemaAlterIntent::Rename { new_name: value }
    } else {
        SchemaAlterIntent::SetComment {
            comment: if remove_comment { None } else { Some(value) },
        }
    };
    intent
        .checked_heap_bytes()
        .ok_or("Rename requires 1–63 UTF-8 bytes; comments allow 4096 bytes; NUL is unsupported")?;
    Ok(intent)
}
