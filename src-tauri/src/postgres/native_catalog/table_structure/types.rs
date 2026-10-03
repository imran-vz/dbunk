use super::*;
use serde::{Deserialize, Serialize};
// Structure's baseline trigger catalog uses tgparentid (PG13+). This is a
// capability floor for this complete capture, not a connection support floor.
pub const MIN_STRUCTURE_SERVER_VERSION: u32 = 130_000;
pub const MAX_STRUCTURE_BYTES: usize = 4 * 1024 * 1024;
pub const MAX_STRUCTURE_COMPONENTS: usize = 4096;
pub const MAX_STRUCTURE_COLUMNS: usize = 1600;
pub const MAX_STRUCTURE_DEFINITION_BYTES: usize = 1024 * 1024;
pub const MAX_STRUCTURE_METADATA_BYTES: usize = 8192;
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TableIdentity {
    pub database_oid: u32,
    pub relation_oid: u32,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TableStructureRequest {
    pub schema: String,
    pub table: String,
    pub expected: Option<TableIdentity>,
}
impl TableStructureRequest {
    pub(crate) fn validate(&self) -> Result<(), CatalogError> {
        if [&self.schema, &self.table]
            .iter()
            .any(|s| s.is_empty() || s.len() > 63 || s.contains('\0') || s.capacity() > 256)
            || self
                .expected
                .is_some_and(|i| i.database_oid == 0 || i.relation_oid == 0)
        {
            return Err(CatalogError::InvalidReference);
        }
        Ok(())
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StructureRelationKind {
    Table,
    PartitionedTable,
    View,
    MaterializedView,
    ForeignTable,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StructureIdentityKind {
    None,
    Always,
    ByDefault,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StructureGeneratedKind {
    None,
    Stored,
    Virtual,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StructureTriggerEnabled {
    Origin,
    Disabled,
    Replica,
    Always,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StructurePolicyCommand {
    All,
    Select,
    Insert,
    Update,
    Delete,
}
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StructureColumn {
    pub number: i32,
    pub name: String,
    pub data_type: String,
    pub nullable: bool,
    pub default_expression: Option<String>,
    pub comment: Option<String>,
    pub identity: StructureIdentityKind,
    pub generated: StructureGeneratedKind,
    pub primary_key_position: Option<u16>,
    pub collation_schema: Option<String>,
    pub collation_name: Option<String>,
}
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StructureKeyColumn {
    pub number: i32,
    pub name: String,
}
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StructurePrimaryKey {
    pub oid: u32,
    pub name: String,
    pub columns: Vec<StructureKeyColumn>,
    pub deferrable: bool,
    pub initially_deferred: bool,
}
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StructureKeyPair {
    pub source_number: i32,
    pub source: String,
    pub target_number: i32,
    pub target: String,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StructureReferentialAction {
    NoAction,
    Restrict,
    Cascade,
    SetNull,
    SetDefault,
}
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StructureForeignKey {
    pub oid: u32,
    pub name: String,
    pub source_oid: u32,
    pub source_schema: String,
    pub source_table: String,
    pub target_oid: u32,
    pub target_schema: String,
    pub target_table: String,
    pub columns: Vec<StructureKeyPair>,
    pub on_update: StructureReferentialAction,
    pub on_delete: StructureReferentialAction,
    pub match_type: String,
    pub deferrable: bool,
    pub initially_deferred: bool,
    pub validated: bool,
}
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StructureIndexKey {
    pub position: u16,
    pub column_number: Option<i32>,
    pub column_name: Option<String>,
    pub definition: String,
    pub included: bool,
}
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StructureIndex {
    pub oid: u32,
    pub name: String,
    pub method: String,
    pub unique: bool,
    pub primary: bool,
    pub valid: bool,
    pub ready: bool,
    pub keys: Vec<StructureIndexKey>,
    pub predicate: Option<String>,
    pub definition: String,
    pub constraint_oid: Option<u32>,
}
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StructureConstraint {
    pub oid: u32,
    pub name: String,
    pub kind: String,
    pub definition: String,
    pub deferrable: bool,
    pub initially_deferred: bool,
    pub validated: bool,
}
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StructureTrigger {
    pub oid: u32,
    pub name: String,
    pub parent_trigger_oid: Option<u32>,
    pub timing: String,
    pub events: Vec<String>,
    pub update_columns: Vec<StructureKeyColumn>,
    pub level: String,
    pub enabled: StructureTriggerEnabled,
    pub function_oid: u32,
    pub function_schema: String,
    pub function_name: String,
    pub definition: String,
}
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StructurePolicy {
    pub oid: u32,
    pub name: String,
    pub permissive: bool,
    pub command: StructurePolicyCommand,
    pub roles: Vec<String>,
    pub using_expression: Option<String>,
    pub with_check: Option<String>,
}
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StructurePrivilege {
    pub grantor: String,
    pub grantee: String,
    pub privilege: String,
    pub grantable: bool,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct StructureRowSecurity {
    pub enabled: bool,
    pub forced: bool,
}
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StructureRule {
    pub oid: u32,
    pub name: String,
    pub event: String,
    pub instead: bool,
    pub enabled: StructureTriggerEnabled,
    pub definition: String,
}
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StructureRelative {
    pub oid: u32,
    pub schema: String,
    pub name: String,
    pub sequence: i32,
    pub is_partition: bool,
    pub bound: Option<String>,
}
#[derive(Clone, Serialize)]
pub struct TableStructureSnapshot {
    pub identity: TableIdentity,
    pub schema: String,
    pub table: String,
    pub kind: StructureRelationKind,
    pub owner: String,
    pub comment: Option<String>,
    pub server_version: u32,
    pub captured_at: String,
    pub columns: Vec<StructureColumn>,
    pub primary_key: Option<StructurePrimaryKey>,
    pub outbound: Vec<StructureForeignKey>,
    pub inbound: Vec<StructureForeignKey>,
    pub indexes: Vec<StructureIndex>,
    pub constraints: Vec<StructureConstraint>,
    pub triggers: Vec<StructureTrigger>,
    pub row_security: StructureRowSecurity,
    pub policies: Vec<StructurePolicy>,
    pub privileges: Vec<StructurePrivilege>,
    pub rules: Vec<StructureRule>,
    pub partition_key: Option<String>,
    pub is_partition: bool,
    pub partition_bound: Option<String>,
    pub parents: Vec<StructureRelative>,
    pub partitions: Vec<StructureRelative>,
}
impl std::fmt::Debug for TableStructureSnapshot {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TableStructureSnapshot")
            .field("identity", &self.identity)
            .field("columns", &self.columns.len())
            .finish_non_exhaustive()
    }
}
