pub use crate::backend::schema_ddl::CreateSchemaAttemptId as SchemaAlterAttemptId;
use crate::backend::table_ddl::valid_name;
/// Outcome and failure vocabulary is shared with table DDL: both are one
/// transactional statement on an owned socket with the same settlement rules.
pub use crate::backend::table_ddl::{
    TableDdlFailure as SchemaAlterFailure, TableDdlOutcome as SchemaAlterOutcome,
};
use serde::{Deserialize, Serialize};
use std::{fmt, mem::size_of};

pub const MAX_SCHEMA_ALTER_COMMENT_BYTES: usize = 4096;
pub const MAX_SCHEMA_ALTER_DESCRIPTION_BYTES: usize = 8 * 1024;
pub const MAX_SCHEMA_ALTER_PREVIEW_BYTES: usize = 16 * 1024;
pub const MAX_SCHEMA_ALTER_RECEIPT_BYTES: usize = 32 * 1024;
pub const SCHEMA_ALTER_OPERATION_TIMEOUT_MS: u32 = 30_000;
pub const SCHEMA_ALTER_EFFECT_SCOPE: &str = "One transactional change. PostgreSQL has no schema lock statement: the observed schema OID, name, comment and catalog row version are rechecked inside the transaction before and after the statement, and any difference rolls back. Rollback does not undo sequence or external effects of server hooks. No automatic retry.";

/// Database and namespace OIDs observed together. Names are never identity.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SchemaIdentity {
    pub database_oid: u32,
    pub schema_oid: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SchemaAlterRequest {
    pub schema: String,
    pub expected: Option<SchemaIdentity>,
}
impl SchemaAlterRequest {
    pub fn validate(&self) -> Result<(), SchemaAlterError> {
        if !valid_name(&self.schema)
            || self.schema.capacity() > 256
            || self
                .expected
                .is_some_and(|i| i.database_oid == 0 || i.schema_oid == 0)
        {
            return Err(SchemaAlterError::InvalidTarget);
        }
        Ok(())
    }
}

/// Read-only recovery/display information. Deserialization never creates target authority.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SchemaAlterDescription {
    pub identity: SchemaIdentity,
    pub schema: String,
    pub namespace_xmin: String,
    pub namespace_ctid: String,
    pub comment: Option<String>,
}
impl fmt::Debug for SchemaAlterDescription {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SchemaAlterDescription")
            .field("identity", &self.identity)
            .finish_non_exhaustive()
    }
}
impl SchemaAlterDescription {
    pub fn checked_heap_bytes(&self) -> Option<usize> {
        if self.identity.database_oid == 0
            || self.identity.schema_oid == 0
            || !valid_name(&self.schema)
            || self.namespace_xmin.is_empty()
            || self.namespace_xmin.len() > 10
            || !self.namespace_xmin.bytes().all(|b| b.is_ascii_digit())
            || self.namespace_ctid.len() > 32
            || self.namespace_ctid.len() < 5
            || !self
                .namespace_ctid
                .bytes()
                .all(|b| b.is_ascii_digit() || b"(),".contains(&b))
            || self
                .comment
                .as_ref()
                .is_some_and(|s| s.len() > MAX_SCHEMA_ALTER_COMMENT_BYTES || s.contains('\0'))
        {
            return None;
        }
        let bytes = size_of::<Self>()
            .checked_add(self.schema.capacity())?
            .checked_add(self.namespace_xmin.capacity())?
            .checked_add(self.namespace_ctid.capacity())?
            .checked_add(self.comment.as_ref().map_or(0, String::capacity))?;
        (bytes <= MAX_SCHEMA_ALTER_DESCRIPTION_BYTES
            && self.encoded_bytes() <= MAX_SCHEMA_ALTER_DESCRIPTION_BYTES)
            .then_some(bytes)
    }
    pub fn encoded_bytes(&self) -> usize {
        super::super::schema_ddl::encoded_bytes(self)
    }
    pub fn request(&self) -> SchemaAlterRequest {
        SchemaAlterRequest {
            schema: self.schema.clone(),
            expected: Some(self.identity),
        }
    }
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum SchemaAlterIntent {
    SetComment { comment: Option<String> },
    Rename { new_name: String },
}
impl fmt::Debug for SchemaAlterIntent {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("SchemaAlterIntent(<redacted>)")
    }
}
impl SchemaAlterIntent {
    pub fn checked_heap_bytes(&self) -> Option<usize> {
        let capacity = match self {
            Self::SetComment { comment } => {
                if comment
                    .as_ref()
                    .is_some_and(|s| s.len() > MAX_SCHEMA_ALTER_COMMENT_BYTES || s.contains('\0'))
                {
                    return None;
                }
                comment.as_ref().map_or(0, String::capacity)
            }
            Self::Rename { new_name } => {
                if !valid_name(new_name) {
                    return None;
                }
                new_name.capacity()
            }
        };
        (capacity <= MAX_SCHEMA_ALTER_COMMENT_BYTES).then_some(size_of::<Self>() + capacity)
    }
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SchemaAlterPreview {
    pub sql: String,
    pub summary: String,
    pub statement_timeout_ms: Option<u32>,
    pub operation_timeout_ms: u32,
}
impl fmt::Debug for SchemaAlterPreview {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("SchemaAlterPreview(<redacted>)")
    }
}
impl SchemaAlterPreview {
    pub fn checked_heap_bytes(&self) -> Option<usize> {
        let bytes = size_of::<Self>()
            .checked_add(self.sql.capacity())?
            .checked_add(self.summary.capacity())?;
        (self.operation_timeout_ms == SCHEMA_ALTER_OPERATION_TIMEOUT_MS
            && bytes <= MAX_SCHEMA_ALTER_PREVIEW_BYTES
            && super::super::schema_ddl::encoded_bytes(self) <= MAX_SCHEMA_ALTER_PREVIEW_BYTES)
            .then_some(bytes)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SchemaAlterReceipt {
    pub attempt_id: SchemaAlterAttemptId,
    pub connection_id: String,
    pub target: SchemaAlterDescription,
    pub intent: SchemaAlterIntent,
    pub outcome: SchemaAlterOutcome,
}
impl SchemaAlterReceipt {
    pub fn retained_bytes(&self) -> usize {
        size_of::<Self>()
            .saturating_add(self.connection_id.capacity())
            .saturating_add(self.attempt_id.as_str().len())
            .saturating_add(self.target.checked_heap_bytes().unwrap_or(usize::MAX))
            .saturating_add(self.intent.checked_heap_bytes().unwrap_or(usize::MAX))
            .saturating_add(5) // maximum retained SQLSTATE
    }
    pub fn encoded_bytes(&self) -> usize {
        super::super::schema_ddl::encoded_bytes(self)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SchemaAlterError {
    InvalidTarget,
    Unavailable,
    Storage,
    Busy,
    PolicyBlocked,
    ForeignDocument,
    OutcomeUnavailable,
}
impl fmt::Display for SchemaAlterError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::InvalidTarget => {
                "Schema change target, intent or timeout changed or exceeds its bound; inspect again"
            }
            Self::Unavailable => "Schema observation is unavailable, unsupported or retired",
            Self::Storage => "Stored connection is unavailable",
            Self::Busy => "Another schema or maintenance operation owns this connection",
            Self::PolicyBlocked => "Stored policy blocks this schema change",
            Self::ForeignDocument => "Schema target belongs to a different backend",
            Self::OutcomeUnavailable => {
                "Schema change outcome unavailable; reconcile before another attempt"
            }
        })
    }
}
impl std::error::Error for SchemaAlterError {}
