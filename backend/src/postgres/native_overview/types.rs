use super::CatalogError;
use serde::Serialize;

pub const MAX_OVERVIEW_BYTES: usize = 1024 * 1024;
pub const MAX_RELATION_STATS_ROWS: usize = 256;
pub const MAX_OVERVIEW_NAME_BYTES: usize = 63;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub enum OverviewMetric {
    Value(i64),
    Unknown,
    NotApplicable,
    Restricted,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct OverviewCapture {
    pub database: String,
    pub database_oid: u32,
    pub reader_pid: i32,
    pub collected_start: String,
    pub collected_end: String,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct DatabaseOverviewSnapshot {
    pub capture: OverviewCapture,
    pub database_size_bytes: OverviewMetric,
    pub table_size_bytes: OverviewMetric,
    pub index_size_bytes: OverviewMetric,
    pub table_count: i64,
    /// Baseline counts namespaces containing catalog relations, not empty schemas.
    pub schema_count: i64,
    pub row_count_estimate: OverviewMetric,
    pub known_row_count_estimate: i64,
    pub unknown_estimate_relations: i64,
    pub index_count: i64,
    /// Includes this inspection connection and other current-database sessions.
    pub connection_count: OverviewMetric,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub struct OverviewRelationIdentity {
    pub database_oid: u32,
    pub relation_oid: u32,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub enum RelationStatsScope {
    Database,
    Schema {
        name: String,
        expected_oid: Option<u32>,
    },
    Relation {
        schema: String,
        name: String,
        expected: Option<OverviewRelationIdentity>,
    },
}
#[derive(Clone, Debug)]
pub struct RelationStatsRequest {
    pub scope: RelationStatsScope,
    pub expected_database_oid: Option<u32>,
    pub cursor: Option<RelationStatsCursor>,
}
impl Default for RelationStatsRequest {
    fn default() -> Self {
        Self {
            scope: RelationStatsScope::Database,
            expected_database_oid: None,
            cursor: None,
        }
    }
}
/// Opaque continuation is a keyset, not a retained transaction or snapshot.
/// It cannot be deserialized or changed into a different scope/connection.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct RelationStatsCursor {
    pub(super) connection: String,
    pub(super) document: String,
    pub(super) database: String,
    pub(super) database_oid: u32,
    pub(super) scope: RelationStatsScope,
    pub(super) schema_oid: Option<u32>,
    pub(super) relation_oid: Option<u32>,
    pub(super) schema: String,
    pub(super) name: String,
    pub(super) oid: u32,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub enum OverviewRelationKind {
    Table,
    PartitionedTable,
    View,
    MaterializedView,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct RelationStats {
    pub identity: OverviewRelationIdentity,
    pub schema_oid: u32,
    pub schema: String,
    pub name: String,
    pub kind: OverviewRelationKind,
    pub is_partition: bool,
    pub row_count_estimate: OverviewMetric,
    /// Physical size for this relation only, not a recursive partition total.
    pub total_size_bytes: OverviewMetric,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct RelationStatsTotals {
    pub relation_count: i64,
    pub table_count: i64,
    pub view_count: i64,
    pub materialized_view_count: i64,
    pub row_count_estimate: OverviewMetric,
    pub known_row_count_estimate: i64,
    pub unknown_estimate_relations: i64,
    pub total_size_bytes: OverviewMetric,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct RelationStatsSnapshot {
    pub capture: OverviewCapture,
    pub scope: RelationStatsScope,
    pub schema_oid: Option<u32>,
    pub relation_oid: Option<u32>,
    /// Full scope totals, independently aggregated before keyset paging.
    pub totals: RelationStatsTotals,
    pub rows: Vec<RelationStats>,
    pub next_cursor: Option<RelationStatsCursor>,
}
impl RelationStatsRequest {
    pub fn validate(&self) -> Result<(), CatalogError> {
        if self.checked_heap_bytes().is_none()
            || !super::bounds::scope_valid(&self.scope)
            || self.expected_database_oid == Some(0)
            || self.cursor.as_ref().is_some_and(|c| {
                !super::bounds::cursor_valid(c)
                    || c.scope != self.scope
                    || self
                        .expected_database_oid
                        .is_some_and(|oid| oid != c.database_oid)
            })
        {
            return Err(CatalogError::InvalidReference);
        }
        Ok(())
    }
}
/// One owned read and one catalog transaction; statistics/size functions remain
/// time-varying observations. Only the first Database-scope page includes metrics.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct OverviewSnapshot {
    pub database: Option<DatabaseOverviewSnapshot>,
    pub relations: RelationStatsSnapshot,
}

impl RelationStatsRequest {
    pub(crate) fn bind_document(&self, document: &str) -> Result<(), CatalogError> {
        if self
            .cursor
            .as_ref()
            .is_some_and(|cursor| cursor.document != document)
        {
            return Err(CatalogError::OverviewIdentityChanged);
        }
        Ok(())
    }
}
