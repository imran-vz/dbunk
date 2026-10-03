use super::super::Inner;
use serde::{Deserialize, Serialize};
use std::{fmt, mem::size_of, sync::Weak};

pub const MAX_AUDIT_PAGE_ROWS: usize = 100;
pub const MAX_AUDIT_PAGE_BYTES: usize = 256 * 1024;
pub const MAX_AUDIT_COMMAND_BYTES: usize = 256;
pub const MAX_AUDIT_TIMESTAMP_BYTES: usize = 64;
pub const MAX_AUDIT_CLASSES: usize = 256;
pub const MAX_AUDIT_CURSOR_BYTES: usize = 1024;
pub const SAFETY_AUDIT_GLOBAL_RETENTION: u32 = crate::storage::SAFETY_OVERRIDE_CAP;
pub(super) const MAX_CLASSES_JSON_BYTES: usize = 4096;
pub(super) const MAX_CONNECTION_BYTES: usize = 256;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SafetyAuditClass {
    Read,
    Dml,
    Ddl,
    Transaction,
    Session,
    Unknown,
}
impl SafetyAuditClass {
    pub fn label(self) -> &'static str {
        match self {
            Self::Read => "read",
            Self::Dml => "dml",
            Self::Ddl => "ddl",
            Self::Transaction => "transaction",
            Self::Session => "session",
            Self::Unknown => "unknown",
        }
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum SafetyAuditLimit {
    RowLimit,
    ByteLimit,
}

/// Opaque in-memory continuation. It cannot be restored or forged from JSON.
#[derive(Clone)]
pub struct SafetyAuditCursor {
    pub(super) owner: Weak<Inner>,
    pub(super) connection_id: String,
    pub(super) occurred_at: String,
    pub(super) id: i64,
    pub(super) watermark: i64,
}
impl fmt::Debug for SafetyAuditCursor {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("SafetyAuditCursor(<opaque>)")
    }
}
impl SafetyAuditCursor {
    pub fn checked_heap_bytes(&self) -> Option<usize> {
        if !connection_valid(&self.connection_id)
            || self.connection_id.capacity() > MAX_CONNECTION_BYTES
            || !timestamp_valid(&self.occurred_at)
            || self.occurred_at.capacity() > MAX_AUDIT_TIMESTAMP_BYTES
            || self.id <= 0
            || self.watermark < self.id
        {
            return None;
        }
        let bytes = size_of::<Self>()
            .checked_add(size_of::<Inner>())?
            .checked_add(self.connection_id.capacity())?
            .checked_add(self.occurred_at.capacity())?;
        (bytes <= MAX_AUDIT_CURSOR_BYTES).then_some(bytes)
    }
}

#[derive(Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SafetyAuditRow {
    pub id: i64,
    pub command: String,
    pub classes: Vec<SafetyAuditClass>,
    pub occurred_at: String,
}
impl fmt::Debug for SafetyAuditRow {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SafetyAuditRow")
            .field("id", &self.id)
            .field("class_count", &self.classes.len())
            .finish_non_exhaustive()
    }
}
#[derive(Clone)]
pub struct SafetyAuditPage {
    pub connection_id: String,
    pub rows: Vec<SafetyAuditRow>,
    pub next_cursor: Option<SafetyAuditCursor>,
    pub limit: Option<SafetyAuditLimit>,
}
impl fmt::Debug for SafetyAuditPage {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SafetyAuditPage")
            .field("rows", &self.rows.len())
            .field("limit", &self.limit)
            .finish_non_exhaustive()
    }
}
impl SafetyAuditPage {
    /// Counts a canonical bounded representation without allocating JSON. The
    /// cursor's logical fields count; its profile ownership token is never output.
    pub fn encoded_bytes(&self) -> Option<usize> {
        if self.rows.len() > MAX_AUDIT_PAGE_ROWS {
            return None;
        }
        #[derive(Serialize)]
        #[serde(rename_all = "camelCase")]
        struct Cursor<'a> {
            connection_id: &'a str,
            occurred_at: &'a str,
            id: i64,
            watermark: i64,
        }
        #[derive(Serialize)]
        #[serde(rename_all = "camelCase")]
        struct Page<'a> {
            connection_id: &'a str,
            rows: &'a [SafetyAuditRow],
            next_cursor: Option<Cursor<'a>>,
            limit: Option<SafetyAuditLimit>,
        }
        count_encoded(
            &Page {
                connection_id: &self.connection_id,
                rows: &self.rows,
                next_cursor: self.next_cursor.as_ref().map(|cursor| Cursor {
                    connection_id: &cursor.connection_id,
                    occurred_at: &cursor.occurred_at,
                    id: cursor.id,
                    watermark: cursor.watermark,
                }),
                limit: self.limit,
            },
            MAX_AUDIT_PAGE_BYTES,
        )
    }
    /// Validates actual retained capacities, stable descending identities, fields,
    /// cursor/limit consistency and encoded size. No scratch allocation required.
    pub fn checked_heap_bytes(&self) -> Option<usize> {
        if !connection_valid(&self.connection_id)
            || self.connection_id.capacity() > MAX_CONNECTION_BYTES
            || self.rows.len() > MAX_AUDIT_PAGE_ROWS
            || self.rows.capacity() > MAX_AUDIT_PAGE_ROWS
            || self.next_cursor.is_some() != self.limit.is_some()
        {
            return None;
        }
        if self.limit == Some(SafetyAuditLimit::RowLimit) && self.rows.len() != MAX_AUDIT_PAGE_ROWS
        {
            return None;
        }
        let mut bytes = size_of::<Self>()
            .checked_add(self.connection_id.capacity())?
            .checked_add(
                self.rows
                    .capacity()
                    .checked_mul(size_of::<SafetyAuditRow>())?,
            )?;
        for (index, row) in self.rows.iter().enumerate() {
            if row.id <= 0
                || !command_valid(&row.command)
                || !timestamp_valid(&row.occurred_at)
                || row.command.capacity() > MAX_AUDIT_COMMAND_BYTES
                || row.occurred_at.capacity() > MAX_AUDIT_TIMESTAMP_BYTES
                || row.classes.len() > MAX_AUDIT_CLASSES
                || row.classes.capacity() > MAX_AUDIT_CLASSES
            {
                return None;
            }
            if index > 0 {
                let previous = &self.rows[index - 1];
                if (previous.occurred_at.as_str(), previous.id)
                    <= (row.occurred_at.as_str(), row.id)
                {
                    return None;
                }
            }
            if self.rows[..index].iter().any(|other| other.id == row.id) {
                return None;
            }
            bytes = bytes
                .checked_add(row.command.capacity())?
                .checked_add(row.occurred_at.capacity())?
                .checked_add(
                    row.classes
                        .capacity()
                        .checked_mul(size_of::<SafetyAuditClass>())?,
                )?;
        }
        if let Some(cursor) = &self.next_cursor {
            let last = self.rows.last()?;
            if cursor.connection_id != self.connection_id
                || cursor.id != last.id
                || cursor.occurred_at != last.occurred_at
                || self.rows.iter().any(|row| row.id > cursor.watermark)
            {
                return None;
            }
            bytes = bytes.checked_add(cursor.checked_heap_bytes()?)?;
        }
        if bytes > MAX_AUDIT_PAGE_BYTES {
            return None;
        }
        self.encoded_bytes()?;
        Some(bytes)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SafetyAuditError {
    InvalidRequest,
    ForeignCursor,
    Storage,
    Corrupt,
    TooLarge,
    Unavailable,
}
impl fmt::Display for SafetyAuditError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::InvalidRequest => "Audit request exceeds its identity bounds",
            Self::ForeignCursor => "Audit cursor belongs to another profile or connection",
            Self::Storage => "Profile-local audit storage is unavailable",
            Self::Corrupt => {
                "Retained audit contains invalid metadata or class labels; previous page preserved"
            }
            Self::TooLarge => "Retained audit field exceeds native limits; previous page preserved",
            Self::Unavailable => {
                "Native profile is closing or its local audit request could not be admitted"
            }
        })
    }
}
impl std::error::Error for SafetyAuditError {}

pub(super) fn connection_valid(value: &str) -> bool {
    !value.is_empty() && value.len() <= MAX_CONNECTION_BYTES && !value.chars().any(char::is_control)
}
pub(super) fn command_valid(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= MAX_AUDIT_COMMAND_BYTES
        && !value.chars().any(char::is_control)
}
pub(super) fn timestamp_valid(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= MAX_AUDIT_TIMESTAMP_BYTES
        && chrono::DateTime::parse_from_rfc3339(value).is_ok()
}

pub(super) fn count_encoded(value: &impl Serialize, limit: usize) -> Option<usize> {
    struct Count {
        bytes: usize,
        limit: usize,
    }
    impl std::io::Write for Count {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            let count = self
                .bytes
                .checked_add(bytes.len())
                .filter(|bytes| *bytes <= self.limit)
                .ok_or_else(|| std::io::Error::other("audit encoding bound"))?;
            self.bytes = count;
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let mut count = Count { bytes: 0, limit };
    serde_json::to_writer(&mut count, value).ok()?;
    Some(count.bytes)
}
