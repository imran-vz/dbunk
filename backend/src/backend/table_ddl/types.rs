pub use crate::backend::schema_ddl::CreateSchemaAttemptId as TableDdlAttemptId;
pub use crate::backend::table_structure::TableIdentity;
use serde::{Deserialize, Serialize};
use std::{fmt, mem::size_of};

pub const MAX_TABLE_DDL_COMMENT_BYTES: usize = 4096;
pub const MAX_TABLE_DDL_DESCRIPTION_BYTES: usize = 8 * 1024;
pub const MAX_TABLE_DDL_PREVIEW_BYTES: usize = 16 * 1024;
pub const MAX_TABLE_DDL_RECEIPT_BYTES: usize = 32 * 1024;
pub const TABLE_DDL_OPERATION_TIMEOUT_MS: u32 = 30_000;
pub const TABLE_DDL_EFFECT_SCOPE: &str = "One transactional change. Identity changes cause rollback. Rollback does not undo sequence or external effects of server hooks. No automatic retry.";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TableDdlRequest {
    pub schema: String,
    pub table: String,
    pub column: Option<String>,
    pub expected: Option<TableIdentity>,
}
pub(crate) fn valid_name(value: &str) -> bool {
    !value.is_empty() && value.len() <= 63 && !value.contains('\0')
}
impl TableDdlRequest {
    pub fn validate(&self) -> Result<(), TableDdlError> {
        if !valid_name(&self.schema)
            || !valid_name(&self.table)
            || self.column.as_ref().is_some_and(|s| !valid_name(s))
            || self
                .expected
                .is_some_and(|i| i.database_oid == 0 || i.relation_oid == 0)
            || self.schema.capacity() > 256
            || self.table.capacity() > 256
            || self.column.as_ref().is_some_and(|s| s.capacity() > 256)
        {
            return Err(TableDdlError::InvalidTarget);
        }
        Ok(())
    }
}
/// Read-only recovery/display information. Deserialization never creates target authority.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TableDdlDescription {
    pub identity: TableIdentity,
    pub schema_oid: u32,
    pub schema: String,
    pub namespace_xmin: String,
    pub namespace_ctid: String,
    pub table: String,
    pub column: Option<TableDdlColumn>,
    pub comment: Option<String>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TableDdlColumn {
    pub attnum: i16,
    pub name: String,
}
impl fmt::Debug for TableDdlDescription {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TableDdlDescription")
            .field("identity", &self.identity)
            .finish_non_exhaustive()
    }
}
impl TableDdlDescription {
    pub fn checked_heap_bytes(&self) -> Option<usize> {
        if self.identity.database_oid == 0
            || self.identity.relation_oid == 0
            || self.schema_oid == 0
            || !valid_name(&self.schema)
            || !valid_name(&self.table)
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
                .column
                .as_ref()
                .is_some_and(|c| c.attnum <= 0 || !valid_name(&c.name))
            || self
                .comment
                .as_ref()
                .is_some_and(|s| s.len() > MAX_TABLE_DDL_COMMENT_BYTES || s.contains('\0'))
        {
            return None;
        }
        let bytes = size_of::<Self>()
            .checked_add(self.schema.capacity())?
            .checked_add(self.table.capacity())?
            .checked_add(self.namespace_xmin.capacity())?
            .checked_add(self.namespace_ctid.capacity())?
            .checked_add(self.column.as_ref().map_or(0, |c| c.name.capacity()))?
            .checked_add(self.comment.as_ref().map_or(0, String::capacity))?;
        (bytes <= MAX_TABLE_DDL_DESCRIPTION_BYTES
            && self.encoded_bytes() <= MAX_TABLE_DDL_DESCRIPTION_BYTES)
            .then_some(bytes)
    }
    pub fn encoded_bytes(&self) -> usize {
        super::super::schema_ddl::encoded_bytes(self)
    }
    pub fn request(&self) -> TableDdlRequest {
        TableDdlRequest {
            schema: self.schema.clone(),
            table: self.table.clone(),
            column: self.column.as_ref().map(|c| c.name.clone()),
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
pub enum TableDdlIntent {
    SetComment { comment: Option<String> },
    Rename { new_name: String },
}
impl fmt::Debug for TableDdlIntent {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("TableDdlIntent(<redacted>)")
    }
}
impl TableDdlIntent {
    pub fn checked_heap_bytes(&self) -> Option<usize> {
        let capacity = match self {
            Self::SetComment { comment } => {
                if comment
                    .as_ref()
                    .is_some_and(|s| s.len() > MAX_TABLE_DDL_COMMENT_BYTES || s.contains('\0'))
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
        (capacity <= MAX_TABLE_DDL_COMMENT_BYTES).then_some(size_of::<Self>() + capacity)
    }
}
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TableDdlPreview {
    pub sql: String,
    pub summary: String,
    pub statement_timeout_ms: Option<u32>,
    pub operation_timeout_ms: u32,
}
impl fmt::Debug for TableDdlPreview {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("TableDdlPreview(<redacted>)")
    }
}
impl TableDdlPreview {
    pub fn checked_heap_bytes(&self) -> Option<usize> {
        let bytes = size_of::<Self>()
            .checked_add(self.sql.capacity())?
            .checked_add(self.summary.capacity())?;
        (self.operation_timeout_ms == TABLE_DDL_OPERATION_TIMEOUT_MS
            && bytes <= MAX_TABLE_DDL_PREVIEW_BYTES
            && super::super::schema_ddl::encoded_bytes(self) <= MAX_TABLE_DDL_PREVIEW_BYTES)
            .then_some(bytes)
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "state",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum TableDdlOutcome {
    NotDispatched {
        reason: TableDdlFailure,
    },
    /// PostgreSQL acknowledged ROLLBACK; only transactional changes are covered.
    RolledBack {
        reason: TableDdlFailure,
    },
    Applied {
        runtime_ms: u64,
    },
    OutcomeUnknown {
        reason: TableDdlFailure,
    },
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase", deny_unknown_fields)]
pub enum TableDdlFailure {
    Cancelled,
    Timeout,
    Connection,
    TargetChanged,
    UnsupportedTarget,
    Limit,
    RollbackUnconfirmed,
    Database { code: Option<String> },
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TableDdlReceipt {
    pub attempt_id: TableDdlAttemptId,
    pub connection_id: String,
    pub target: TableDdlDescription,
    pub intent: TableDdlIntent,
    pub outcome: TableDdlOutcome,
}
impl TableDdlReceipt {
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
pub enum TableDdlError {
    InvalidTarget,
    Unavailable,
    Storage,
    Busy,
    PolicyBlocked,
    ForeignDocument,
    OutcomeUnavailable,
}
impl fmt::Display for TableDdlError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::InvalidTarget => {
                "Table change target, intent or timeout changed or exceeds its bound; inspect again"
            }
            Self::Unavailable => "Table observation is unavailable, unsupported or retired",
            Self::Storage => "Stored connection is unavailable",
            Self::Busy => "Another schema or maintenance operation owns this connection",
            Self::PolicyBlocked => "Stored policy blocks this table change",
            Self::ForeignDocument => "Table target belongs to a different backend",
            Self::OutcomeUnavailable => {
                "Table change outcome unavailable; reconcile before another attempt"
            }
        })
    }
}
impl std::error::Error for TableDdlError {}
