use serde::{Deserialize, Serialize};

pub const NORMALIZATION_VERSION: u32 = 1;
pub const SCOPE: &str = "postgres16OrdinaryTableProjectionV1";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Endpoint {
    pub connection_id: String,
    pub schema: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResultIdentity {
    pub job_id: String,
    pub result_id: String,
}

impl ResultIdentity {
    pub fn validate(&self) -> Result<(), CompareError> {
        if [&self.job_id, &self.result_id]
            .iter()
            .any(|s| s.is_empty() || s.len() > 128)
        {
            return Err(CompareError::InvalidRequest);
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Side {
    Source,
    Target,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum RelationKind {
    Table,
    PartitionedTable,
    ForeignTable,
    View,
    MaterializedView,
    Sequence,
    Composite,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Exclusion {
    Partitioned,
    Inherited,
    Foreign,
    ExtensionOwned,
    OtherKind,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum Eligibility {
    Eligible,
    Excluded { reason: Exclusion },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RelationIdentity {
    pub kind: RelationKind,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InventoryEntry {
    pub identity: RelationIdentity,
    pub eligibility: Eligibility,
}

/// Selected schemas map structurally to one namespace. External references
/// keep their exact schema. Raw SQL text is never passed through this mapping.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum Namespace {
    Selected,
    External { schema: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QualifiedName {
    pub namespace: Namespace,
    pub name: String,
}

impl QualifiedName {
    pub fn in_endpoint(endpoint: &Endpoint, schema: &str, name: &str) -> Self {
        Self {
            namespace: if schema == endpoint.schema {
                Namespace::Selected
            } else {
                Namespace::External {
                    schema: schema.into(),
                }
            },
            name: name.into(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum TableField {
    Persistence,
    Comment,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ColumnField {
    Position,
    Type,
    TypeModifier,
    ArrayDimensions,
    Nullable,
    Default,
    GeneratedKind,
    GeneratedExpression,
    Identity,
    Collation,
    Comment,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ConstraintField {
    Kind,
    Keys,
    ReferencedTable,
    ReferencedKeys,
    UpdateAction,
    DeleteAction,
    DeleteColumns,
    MatchMode,
    Deferrable,
    InitiallyDeferred,
    Validated,
    NoInherit,
    Expression,
    EqualityOperators,
    ExclusionOperators,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum IndexField {
    AccessMethod,
    Unique,
    NullsNotDistinct,
    Immediate,
    KeyCount,
    IncludedColumns,
    Predicate,
    RelationOptions,
    Valid,
    Ready,
    Live,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum IndexKeyField {
    Kind,
    Column,
    Expression,
    SortOptions,
    Opclass,
    OpclassOptions,
    Collation,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum FieldPath {
    Table {
        field: TableField,
    },
    Column {
        name: String,
        field: ColumnField,
    },
    Constraint {
        name: String,
        field: ConstraintField,
    },
    Index {
        name: String,
        owner: Option<String>,
        field: IndexField,
    },
    IndexKey {
        name: String,
        owner: Option<String>,
        position: u16,
        field: IndexKeyField,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum IncomparableReason {
    ExpressionOutsideSubset,
    RenderingVersionDifference,
    ExternalDependency,
    UnknownAccessMethod,
    ExcludedCounterpart,
    ExcludedObject,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OperatorSignature {
    pub operator: QualifiedName,
    pub left_type: Option<QualifiedName>,
    pub right_type: Option<QualifiedName>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum CompareError {
    Busy,
    LimitExceeded { limit: Limit },
    UnsupportedVersion { side: Side, version: String },
    Unavailable,
    InvalidRequest,
    CaptureChanged,
    Cancelled,
    DeadlineExceeded,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Limit {
    Inventory,
    Tables,
    ChildFacts,
    FieldBytes,
    EndpointBytes,
    ResultBytes,
    PageBytes,
    PageItems,
    Allocation,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "phase", rename_all = "camelCase")]
pub enum StatusState {
    Resolving,
    ReadingSource,
    ReadingTarget,
    ReadingBoth,
    Comparing,
    Completed {
        #[serde(rename = "resultId")]
        result_id: String,
    },
    Cancelling,
    Cancelled,
    Failed {
        failure: CompareError,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CaptureMetadata {
    pub endpoint: Endpoint,
    pub server_version: String,
    pub server_version_num: u32,
    pub captured_at: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SnapshotConsistency {
    SharedTransaction,
    IndependentTransactions,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ComparisonMetadata {
    pub identity: ResultIdentity,
    pub source: CaptureMetadata,
    pub target: CaptureMetadata,
    pub consistency: SnapshotConsistency,
    pub coverage: Coverage,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Coverage {
    pub scope: String,
    pub normalization_version: u32,
    pub excluded_relations: u32,
    pub incomparable_fields: u32,
    pub excluded_categories: Vec<ExcludedCategory>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ExcludedCategory {
    OtherRelations,
    Routines,
    Sequences,
    TypesAndDomains,
    Policies,
    Grants,
    Triggers,
    Rules,
    Extensions,
    DatabaseObjects,
    IdentitySequenceConfiguration,
    StorageSecurityOwnershipReplication,
    IndexPlacementClusteringReplicaIdentity,
}

/// List/status payloads contain counts and endpoint metadata, never values.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Status {
    pub job_id: String,
    pub request_id: String,
    pub source: Endpoint,
    pub target: Endpoint,
    pub source_objects: u32,
    pub target_objects: u32,
    #[serde(flatten)]
    pub state: StatusState,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn status(state: StatusState) -> Status {
        Status {
            job_id: "job".into(),
            request_id: "request".into(),
            source: Endpoint {
                connection_id: "source".into(),
                schema: "public".into(),
            },
            target: Endpoint {
                connection_id: "target".into(),
                schema: "public".into(),
            },
            source_objects: 1,
            target_objects: 1,
            state,
        }
    }

    #[test]
    fn status_serialization_binds_terminal_payloads_to_their_phase() {
        let completed = serde_json::to_value(status(StatusState::Completed {
            result_id: "result".into(),
        }))
        .unwrap();
        assert_eq!(completed["phase"], "completed");
        assert_eq!(completed["resultId"], "result");
        assert!(completed.get("failure").is_none());

        let failed = serde_json::to_value(status(StatusState::Failed {
            failure: CompareError::CaptureChanged,
        }))
        .unwrap();
        assert_eq!(failed["phase"], "failed");
        assert_eq!(failed["failure"]["kind"], "captureChanged");
        assert!(failed.get("resultId").is_none());
    }
}
