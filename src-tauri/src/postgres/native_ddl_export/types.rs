use super::CatalogError;
use serde::Serialize;
pub const MAX_DDL_EXPORT_RELATIONS: usize = 1024;
pub const MAX_DDL_EXPORT_SCHEMAS: usize = 1024;
pub const MAX_DDL_EXPORT_SQL_BYTES: usize = 4 * 1024 * 1024;
pub const MAX_DDL_EXPORT_HEAP_BYTES: usize = 8 * 1024 * 1024;
pub const MAX_DDL_EXPORT_ENCODED_BYTES: usize = 16 * 1024 * 1024;
pub const MAX_DDL_EXPORT_NAME_BYTES: usize = 63;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub struct DdlExportIdentity {
    pub database_oid: u32,
    pub relation_oid: u32,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub enum DdlExportScope {
    Database,
    Schema {
        name: String,
        expected_oid: Option<u32>,
    },
    Relation {
        schema: String,
        name: String,
        expected: Option<DdlExportIdentity>,
    },
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct DdlExportRequest {
    pub scope: DdlExportScope,
    pub expected_database_oid: Option<u32>,
}
impl Default for DdlExportRequest {
    fn default() -> Self {
        Self {
            scope: DdlExportScope::Database,
            expected_database_oid: None,
        }
    }
}
impl DdlExportRequest {
    pub fn validate(&self) -> Result<(), CatalogError> {
        self.checked_heap_bytes()
            .map(|_| ())
            .ok_or(CatalogError::InvalidReference)
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub enum DdlExportRelationKind {
    Table,
    PartitionedTable,
    View,
    MaterializedView,
    ForeignTable,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct DdlExportSchema {
    pub oid: u32,
    pub name: String,
    pub declared: bool,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct DdlExportRelation {
    pub identity: DdlExportIdentity,
    pub schema_oid: u32,
    pub schema: String,
    pub name: String,
    pub kind: DdlExportRelationKind,
    /// UTF-8 boundaries into the single artifact SQL string, excluding separator.
    pub sql_start: u32,
    pub sql_end: u32,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub enum DdlExportOmission {
    Data,
    NonRelationObjects,
    DependencyOrdering,
    OwnershipAndPrivileges,
    TriggersRulesAndRowSecurity,
    StorageAndOrdinaryInheritance,
    ForeignServerAndUserMappings,
    SequenceObjectsAndCurrentValues,
    CommentsAndMaterializedViewIndexes,
}
impl DdlExportOmission {
    pub fn explanation(self) -> &'static str {
        match self {
            Self::Data => "No table data is exported; materialized-view SQL preserves its captured populated state.",
            Self::NonRelationObjects => "Standalone routines, types, domains, extensions and other non-relation objects are not included.",
            Self::DependencyOrdering => "Relations are ordered by schema/name, not dependency order; referenced objects may need to exist first.",
            Self::OwnershipAndPrivileges => "Ownership and privileges are not reconstructed.",
            Self::TriggersRulesAndRowSecurity => "Triggers, non-view rules, row-security settings and policies are not reconstructed.",
            Self::StorageAndOrdinaryInheritance => "Physical storage settings and ordinary table inheritance are not fully reconstructed.",
            Self::ForeignServerAndUserMappings => "Foreign-table SQL is a reconstruction, not verified remote compatibility. Foreign servers and user mappings are prerequisites and are not exported; no foreign data is read.",
            Self::CommentsAndMaterializedViewIndexes => "Object comments and materialized-view indexes are not reconstructed.",
            Self::SequenceObjectsAndCurrentValues => "Standalone sequences and current sequence values are not exported; defaults may reference existing sequences.",
        }
    }
}
pub(super) const OMISSIONS: [DdlExportOmission; 9] = [
    DdlExportOmission::Data,
    DdlExportOmission::NonRelationObjects,
    DdlExportOmission::DependencyOrdering,
    DdlExportOmission::OwnershipAndPrivileges,
    DdlExportOmission::TriggersRulesAndRowSecurity,
    DdlExportOmission::StorageAndOrdinaryInheritance,
    DdlExportOmission::ForeignServerAndUserMappings,
    DdlExportOmission::SequenceObjectsAndCurrentValues,
    DdlExportOmission::CommentsAndMaterializedViewIndexes,
];
#[derive(Clone, PartialEq, Eq, Serialize)]
pub struct DdlExportArtifact {
    pub connection_id: String,
    pub database: String,
    pub database_oid: u32,
    pub reader_pid: i32,
    pub collected_start: String,
    pub collected_end: String,
    pub request: DdlExportRequest,
    pub schemas: Vec<DdlExportSchema>,
    pub relations: Vec<DdlExportRelation>,
    pub omissions: Vec<DdlExportOmission>,
    pub sql: String,
}
impl std::fmt::Debug for DdlExportArtifact {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DdlExportArtifact")
            .field("relations", &self.relations.len())
            .field("sql_bytes", &self.sql.len())
            .finish_non_exhaustive()
    }
}
impl DdlExportArtifact {
    pub fn relation_sql(&self, index: usize) -> Option<&str> {
        let relation = self.relations.get(index)?;
        self.sql
            .get(relation.sql_start as usize..relation.sql_end as usize)
    }
}
