use super::CatalogError;
use serde::{Deserialize, Serialize};
pub const MAX_SCHEMA_MAP_BYTES: usize = 4 * 1024 * 1024;
pub const MAX_SCHEMA_MAP_TABLES: usize = 512;
pub const MAX_SCHEMA_MAP_COLUMNS: usize = 8192;
pub const MAX_SCHEMA_MAP_FOREIGN_KEYS: usize = 1024;
pub const MAX_SCHEMA_MAP_PAIRS: usize = 4096;
pub const MAX_SCHEMA_MAP_TRIGGERS: usize = 1024;
pub const MAX_SCHEMA_MAP_UNIQUE_KEYS: usize = 2048;
pub const MAX_SCHEMA_MAP_KEY_COLUMNS: usize = 8192;
pub const MAX_SCHEMA_MAP_COMMENT_BYTES: usize = 4096;
pub const MAX_SCHEMA_MAP_TYPE_BYTES: usize = 8192;
pub const MIN_SCHEMA_MAP_SERVER_VERSION: u32 = 130000;
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct SchemaMapIdentity {
    pub database_oid: u32,
    pub relation_oid: u32,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum SchemaMapScope {
    Database,
    Schema {
        name: String,
        expected_oid: Option<u32>,
    },
    Relation {
        schema: String,
        table: String,
        expected: Option<SchemaMapIdentity>,
    },
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SchemaMapRequest {
    pub scope: SchemaMapScope,
    pub expected_database_oid: Option<u32>,
}
impl Default for SchemaMapRequest {
    fn default() -> Self {
        Self {
            scope: SchemaMapScope::Database,
            expected_database_oid: None,
        }
    }
}
impl SchemaMapRequest {
    pub fn validate(&self) -> Result<(), CatalogError> {
        if self.checked_heap_bytes().is_none() {
            Err(CatalogError::InvalidReference)
        } else {
            Ok(())
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SchemaMapTableKind {
    Table,
    PartitionedTable,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SchemaMapColumn {
    pub attnum: i16,
    pub name: String,
    pub data_type: String,
    pub nullable: bool,
    pub primary_key: bool,
    pub comment: Option<String>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SchemaMapTriggerEnabled {
    Origin,
    Replica,
    Always,
    Disabled,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SchemaMapTrigger {
    pub oid: u32,
    pub name: String,
    pub columns: Vec<i16>,
    pub timing: String,
    pub events: Vec<String>,
    pub orientation: String,
    pub enabled: SchemaMapTriggerEnabled,
    pub function_oid: u32,
    pub function_schema: String,
    pub function_name: String,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SchemaMapTable {
    pub identity: SchemaMapIdentity,
    pub schema_oid: u32,
    pub schema: String,
    pub name: String,
    pub kind: SchemaMapTableKind,
    /// Outside the selected schema, included as the fully described target of
    /// one of that schema's outgoing relationships. Never an incomplete stub.
    pub external: bool,
    pub columns: Vec<SchemaMapColumn>,
    pub triggers: Vec<SchemaMapTrigger>,
    pub junction: bool,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SchemaMapAction {
    NoAction,
    Restrict,
    Cascade,
    SetNull,
    SetDefault,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SchemaMapCardinality {
    OneToOne,
    OneToMany,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SchemaMapColumnPair {
    pub source: i16,
    pub target: i16,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SchemaMapForeignKey {
    pub database_oid: u32,
    pub constraint_oid: u32,
    pub name: String,
    pub source: SchemaMapIdentity,
    pub target: SchemaMapIdentity,
    pub columns: Vec<SchemaMapColumnPair>,
    pub on_update: SchemaMapAction,
    pub on_delete: SchemaMapAction,
    pub match_type: String,
    pub validated: bool,
    pub deferrable: bool,
    pub columns_nullable: bool,
    pub columns_unique: bool,
    pub cardinality: SchemaMapCardinality,
    pub cardinality_reason: String,
    pub junction_participant: bool,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SchemaMapSnapshot {
    pub database: String,
    pub database_oid: u32,
    pub captured_at: String,
    pub server_version: u32,
    pub scope: SchemaMapScope,
    pub schema_oid: Option<u32>,
    pub focus: Option<SchemaMapIdentity>,
    pub tables: Vec<SchemaMapTable>,
    pub foreign_keys: Vec<SchemaMapForeignKey>,
}
impl SchemaMapSnapshot {
    /// Refresh the exact observed scope. This is a read request, never write authority.
    pub fn refresh_request(&self) -> SchemaMapRequest {
        let scope = match &self.scope {
            SchemaMapScope::Database => SchemaMapScope::Database,
            SchemaMapScope::Schema { name, .. } => SchemaMapScope::Schema {
                name: name.clone(),
                expected_oid: self.schema_oid,
            },
            SchemaMapScope::Relation { schema, table, .. } => SchemaMapScope::Relation {
                schema: schema.clone(),
                table: table.clone(),
                expected: self.focus,
            },
        };
        SchemaMapRequest {
            scope,
            expected_database_oid: Some(self.database_oid),
        }
    }
}
