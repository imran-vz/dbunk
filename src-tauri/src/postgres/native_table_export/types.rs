use super::*;
use serde::Serialize;
pub const MAX_TABLE_EXPORT_COLUMNS: usize = 1024;
pub const MAX_TABLE_EXPORT_ROWS: usize = 100_000;
pub const MAX_TABLE_EXPORT_CELLS: usize = 100_000;
pub const MAX_TABLE_EXPORT_TEXT_BYTES: usize = 8 * 1024 * 1024;
pub const MAX_TABLE_EXPORT_HEAP_BYTES: usize = 16 * 1024 * 1024;
pub const MAX_TABLE_EXPORT_ENCODED_BYTES: usize = 16 * 1024 * 1024;
pub const MAX_TABLE_EXPORT_FIELD_BYTES: usize = 1024 * 1024;
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub struct TableExportIdentity {
    pub database_oid: u32,
    pub relation_oid: u32,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TableExportRequest {
    pub schema: String,
    pub table: String,
    pub expected: Option<TableExportIdentity>,
}
impl TableExportRequest {
    pub fn validate(&self) -> Result<(), CatalogError> {
        if !bounds::name(&self.schema)
            || !bounds::name(&self.table)
            || self.schema.capacity() > 256
            || self.table.capacity() > 256
            || self
                .expected
                .is_some_and(|id| id.database_oid == 0 || id.relation_oid == 0)
        {
            Err(CatalogError::InvalidReference)
        } else {
            Ok(())
        }
    }
    pub fn checked_heap_bytes(&self) -> Option<usize> {
        self.validate().ok()?;
        std::mem::size_of::<Self>()
            .checked_add(self.schema.capacity())?
            .checked_add(self.table.capacity())
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub enum TableExportKind {
    Table,
    PartitionedTable,
    View,
    MaterializedView,
    ForeignTable,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct TableExportColumn {
    pub name: String,
    pub attnum: i16,
    pub type_oid: u32,
    pub type_modifier: i32,
    pub collation_oid: u32,
}
#[derive(Serialize)]
pub struct TableExportData {
    pub connection_id: String,
    pub database: String,
    pub schema: String,
    pub table: String,
    pub identity: TableExportIdentity,
    pub schema_oid: u32,
    pub kind: TableExportKind,
    /// Catalog RLS enabled. Server SELECT policies govern which rows are visible;
    /// this flag does not claim that the caller is subject to, or bypasses, RLS.
    pub row_security: bool,
    pub captured_start: String,
    pub captured_end: String,
    pub columns: Vec<TableExportColumn>,
    pub rows: Vec<Vec<Option<String>>>,
}
impl TableExportData {
    pub fn checked_heap_bytes(&self) -> Option<usize> {
        bounds::checked(self)
    }
    pub fn encoded_bytes(&self) -> Option<usize> {
        bounds::encoded(self, MAX_TABLE_EXPORT_ENCODED_BYTES)
    }
}
/// Immutable host-neutral source. Clone shares the same admitted rows; Debug
/// omits cell values. A capture is never executable database authority.
#[derive(Clone)]
pub struct TableExportCapture(pub(super) Arc<TableExportData>);
impl TableExportCapture {
    pub fn data(&self) -> &TableExportData {
        &self.0
    }
    pub fn checked_heap_bytes(&self) -> Option<usize> {
        self.0.checked_heap_bytes()
    }
    pub fn encoded_bytes(&self) -> Option<usize> {
        self.0.encoded_bytes()
    }
}
impl std::fmt::Debug for TableExportCapture {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TableExportCapture")
            .field("identity", &self.0.identity)
            .field("rows", &self.0.rows.len())
            .finish_non_exhaustive()
    }
}
