use crate::backend::schema_ddl::encoded_bytes;
pub use crate::backend::schema_ddl::CreateSchemaAttemptId as ObjectDdlAttemptId;
use crate::postgres::object_ddl::{
    AddEnumValueOp, CreateIndexOp, CreateMaterializedViewOp, CreateViewOp, DropObjectOp,
    PgEnumPosition, PgIndexColumn, PgObjectOp,
};
pub use crate::postgres::objects::{PgDropDependent, PgDropImpact, PgObjectKind, PgObjectRef};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeSet, fmt, mem::size_of};

/// One request is a short ordered group, never a script.
pub const MAX_OBJECT_DDL_OPERATIONS: usize = 8;
pub const MAX_OBJECT_DDL_REQUEST_BYTES: usize = 24 * 1024;
pub const MAX_OBJECT_DDL_SQL_BODY_BYTES: usize = 16 * 1024;
pub const MAX_OBJECT_DDL_IDENTITY_ARGS_BYTES: usize = 2048;
pub const MAX_OBJECT_DDL_INDEX_COLUMNS: usize = 16;
pub const MAX_OBJECT_DDL_DESCRIPTION_BYTES: usize = 8 * 1024;
pub const MAX_OBJECT_DDL_PREVIEW_BYTES: usize = 40 * 1024;
pub const MAX_OBJECT_DDL_RECEIPT_BYTES: usize = 96 * 1024;
/// Drop impact is review evidence only; it is never journaled.
pub const MAX_OBJECT_DDL_IMPACT_BYTES: usize = 256 * 1024;
pub const OBJECT_DDL_OPERATION_TIMEOUT_MS: u32 = 60_000;
pub const OBJECT_DDL_ATOMIC_SCOPE: &str = "One transaction. Every target identity is rechecked inside it and verified after each statement; any change rolls back. Rollback does not undo sequence or external effects of server hooks. No automatic retry.";
pub const OBJECT_DDL_STANDALONE_SCOPE: &str = "Contains statements PostgreSQL forbids in a transaction block. Groups run in order and each commits on its own; a later failure leaves the earlier committed prefix in place and reports it. Standalone targets are rechecked immediately before dispatch; a concurrent replacement inside that window is not excluded. A failed concurrent index build can leave an INVALID index. No automatic retry.";
pub const OBJECT_DDL_CASCADE_DISCLOSURE: &str = "CASCADE drops every dependent that exists when the statement runs. The impact list is a bounded snapshot taken with the observed identity; objects created later, or beyond a truncated walk, are dropped too.";

pub(crate) fn valid_name(value: &str) -> bool {
    !value.is_empty() && value.len() <= 63 && !value.contains('\0')
}
fn bounded_text(value: &str, maximum: usize) -> bool {
    !value.is_empty() && value.len() <= maximum && !value.contains('\0')
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ObjectDdlIndexColumn {
    pub expression: String,
    pub descending: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum ObjectDdlEnumPosition {
    Before { neighbor: String },
    After { neighbor: String },
}

/// The admitted object-DDL vocabulary. Each variant maps to exactly one typed
/// [`PgObjectOp`] whose SQL is regenerated at every trust boundary; no variant
/// carries statement text except opaque bodies validated as one fragment.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "op",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum ObjectDdlOperation {
    DropObject {
        reference: PgObjectRef,
        cascade: bool,
    },
    CreateView {
        schema: String,
        name: String,
        or_replace: bool,
        sql_body: String,
    },
    CreateMaterializedView {
        schema: String,
        name: String,
        sql_body: String,
        with_data: bool,
    },
    /// A named index; the name is required so its absence is an exact claim.
    CreateIndex {
        schema: String,
        table: String,
        name: String,
        unique: bool,
        method: String,
        columns: Vec<ObjectDdlIndexColumn>,
        concurrently: bool,
    },
    AddEnumValue {
        schema: String,
        name: String,
        value: String,
        position: Option<ObjectDdlEnumPosition>,
    },
}
impl fmt::Debug for ObjectDdlOperation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::DropObject { .. } => "ObjectDdlOperation::DropObject(<redacted>)",
            Self::CreateView { .. } => "ObjectDdlOperation::CreateView(<redacted>)",
            Self::CreateMaterializedView { .. } => {
                "ObjectDdlOperation::CreateMaterializedView(<redacted>)"
            }
            Self::CreateIndex { .. } => "ObjectDdlOperation::CreateIndex(<redacted>)",
            Self::AddEnumValue { .. } => "ObjectDdlOperation::AddEnumValue(<redacted>)",
        })
    }
}

fn reference_bytes(reference: &PgObjectRef) -> Option<usize> {
    let schema_ok = match reference.kind {
        PgObjectKind::Schema => reference.schema.is_none(),
        PgObjectKind::Extension => false,
        _ => reference.schema.as_deref().is_some_and(valid_name),
    };
    let routine = matches!(
        reference.kind,
        PgObjectKind::Function | PgObjectKind::Procedure | PgObjectKind::Aggregate
    );
    if !schema_ok
        || !valid_name(&reference.name)
        || routine != reference.identity_args.is_some()
        || reference.identity_args.as_ref().is_some_and(|args| {
            args.len() > MAX_OBJECT_DDL_IDENTITY_ARGS_BYTES || args.contains('\0')
        })
    {
        return None;
    }
    Some(
        reference.name.capacity()
            + reference.schema.as_ref().map_or(0, String::capacity)
            + reference.identity_args.as_ref().map_or(0, String::capacity),
    )
}

impl ObjectDdlOperation {
    /// Capacity-based accounting; refuses empty/oversized/NUL text before any
    /// rendering. Whitespace-only names are refused later by the shared renderer.
    pub fn checked_heap_bytes(&self) -> Option<usize> {
        let heap = match self {
            Self::DropObject { reference, .. } => reference_bytes(reference)?,
            Self::CreateView {
                schema,
                name,
                sql_body,
                ..
            }
            | Self::CreateMaterializedView {
                schema,
                name,
                sql_body,
                ..
            } => {
                if !valid_name(schema)
                    || !valid_name(name)
                    || !bounded_text(sql_body, MAX_OBJECT_DDL_SQL_BODY_BYTES)
                {
                    return None;
                }
                schema.capacity() + name.capacity() + sql_body.capacity()
            }
            Self::CreateIndex {
                schema,
                table,
                name,
                method,
                columns,
                ..
            } => {
                if !valid_name(schema)
                    || !valid_name(table)
                    || !valid_name(name)
                    || !valid_name(method)
                    || columns.is_empty()
                    || columns.len() > MAX_OBJECT_DDL_INDEX_COLUMNS
                    || columns.capacity() > MAX_OBJECT_DDL_INDEX_COLUMNS
                    || columns
                        .iter()
                        .any(|column| !bounded_text(&column.expression, 1024))
                {
                    return None;
                }
                schema.capacity()
                    + table.capacity()
                    + name.capacity()
                    + method.capacity()
                    + columns.capacity() * size_of::<ObjectDdlIndexColumn>()
                    + columns
                        .iter()
                        .map(|column| column.expression.capacity())
                        .sum::<usize>()
            }
            Self::AddEnumValue {
                schema,
                name,
                value,
                position,
            } => {
                let neighbor = match position {
                    Some(
                        ObjectDdlEnumPosition::Before { neighbor }
                        | ObjectDdlEnumPosition::After { neighbor },
                    ) => Some(neighbor),
                    None => None,
                };
                if !valid_name(schema)
                    || !valid_name(name)
                    || !valid_name(value)
                    || neighbor.is_some_and(|n| !valid_name(n))
                {
                    return None;
                }
                schema.capacity()
                    + name.capacity()
                    + value.capacity()
                    + neighbor.map_or(0, String::capacity)
            }
        };
        size_of::<Self>().checked_add(heap)
    }

    /// The typed operation regenerated at the trust boundary.
    pub(crate) fn to_pg(&self) -> PgObjectOp {
        match self.clone() {
            Self::DropObject { reference, cascade } => {
                PgObjectOp::DropObject(DropObjectOp { reference, cascade })
            }
            Self::CreateView {
                schema,
                name,
                or_replace,
                sql_body,
            } => PgObjectOp::CreateView(CreateViewOp {
                schema,
                name,
                or_replace,
                sql_body,
            }),
            Self::CreateMaterializedView {
                schema,
                name,
                sql_body,
                with_data,
            } => PgObjectOp::CreateMaterializedView(CreateMaterializedViewOp {
                schema,
                name,
                sql_body,
                with_data,
            }),
            Self::CreateIndex {
                schema,
                table,
                name,
                unique,
                method,
                columns,
                concurrently,
            } => PgObjectOp::CreateIndex(CreateIndexOp {
                schema,
                table,
                name: Some(name),
                unique,
                method,
                columns: columns
                    .into_iter()
                    .map(|column| PgIndexColumn {
                        expression: column.expression,
                        descending: column.descending,
                    })
                    .collect(),
                include: Vec::new(),
                where_predicate: None,
                concurrently,
            }),
            Self::AddEnumValue {
                schema,
                name,
                value,
                position,
            } => PgObjectOp::AddEnumValue(AddEnumValueOp {
                schema,
                name,
                value,
                position: position.map(|position| match position {
                    ObjectDdlEnumPosition::Before { neighbor } => {
                        PgEnumPosition::Before { neighbor }
                    }
                    ObjectDdlEnumPosition::After { neighbor } => PgEnumPosition::After { neighbor },
                }),
            }),
        }
    }

    /// Claims the observation must capture, in a fixed per-operation order.
    pub(crate) fn claim_specs(&self) -> Vec<ClaimSpec> {
        match self {
            Self::DropObject { reference, .. } => vec![ClaimSpec::Existing(reference.clone())],
            Self::CreateView {
                schema,
                name,
                or_replace,
                ..
            } => vec![
                ClaimSpec::Schema(schema.clone()),
                if *or_replace {
                    ClaimSpec::ViewOrAbsent {
                        schema: schema.clone(),
                        name: name.clone(),
                    }
                } else {
                    ClaimSpec::Absent {
                        schema: schema.clone(),
                        name: name.clone(),
                    }
                },
            ],
            Self::CreateMaterializedView { schema, name, .. } => vec![
                ClaimSpec::Schema(schema.clone()),
                ClaimSpec::Absent {
                    schema: schema.clone(),
                    name: name.clone(),
                },
            ],
            Self::CreateIndex {
                schema,
                table,
                name,
                ..
            } => vec![
                ClaimSpec::Schema(schema.clone()),
                ClaimSpec::Existing(PgObjectRef {
                    kind: PgObjectKind::Table,
                    schema: Some(schema.clone()),
                    name: table.clone(),
                    identity_args: None,
                }),
                ClaimSpec::Absent {
                    schema: schema.clone(),
                    name: name.clone(),
                },
            ],
            Self::AddEnumValue { schema, name, .. } => vec![ClaimSpec::Existing(PgObjectRef {
                kind: PgObjectKind::Type,
                schema: Some(schema.clone()),
                name: name.clone(),
                identity_args: None,
            })],
        }
    }

    /// The (schema, name) slots this operation creates, drops or alters.
    /// Distinct slots keep every per-group recheck independent of earlier
    /// statements in the same request.
    fn slots(&self) -> Vec<(Option<&str>, &str)> {
        match self {
            Self::DropObject { reference, .. } => {
                vec![(reference.schema.as_deref(), reference.name.as_str())]
            }
            Self::CreateView { schema, name, .. }
            | Self::CreateMaterializedView { schema, name, .. }
            | Self::AddEnumValue { schema, name, .. } => vec![(Some(schema), name)],
            Self::CreateIndex {
                schema,
                table,
                name,
                ..
            } => vec![(Some(schema), table), (Some(schema), name)],
        }
    }
}

/// Internal claim request derived from an operation; never serialized.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum ClaimSpec {
    Existing(PgObjectRef),
    Schema(String),
    Absent { schema: String, name: String },
    ViewOrAbsent { schema: String, name: String },
}

#[derive(Clone, PartialEq, Eq)]
pub struct ObjectDdlRequest {
    pub operations: Vec<ObjectDdlOperation>,
}
impl fmt::Debug for ObjectDdlRequest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ObjectDdlRequest")
            .field("operations", &self.operations.len())
            .finish()
    }
}
impl ObjectDdlRequest {
    pub fn validate(&self) -> Result<(), ObjectDdlError> {
        operations_heap_bytes(&self.operations)
            .map(|_| ())
            .ok_or(ObjectDdlError::InvalidRequest)
    }
}

pub(crate) fn operations_heap_bytes(operations: &[ObjectDdlOperation]) -> Option<usize> {
    if operations.is_empty() || operations.len() > MAX_OBJECT_DDL_OPERATIONS {
        return None;
    }
    let mut total = operations
        .len()
        .checked_mul(size_of::<ObjectDdlOperation>())?;
    let mut slots = BTreeSet::new();
    for operation in operations {
        total = total.checked_add(operation.checked_heap_bytes()?)?;
        for slot in operation.slots() {
            if !slots.insert(slot) {
                return None;
            }
        }
    }
    (total <= MAX_OBJECT_DDL_REQUEST_BYTES * 2
        && encoded_bytes(&operations) <= MAX_OBJECT_DDL_REQUEST_BYTES)
        .then_some(total)
}

/// Exact catalog address plus the catalog row version observed with it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ObjectAddress {
    pub class_oid: u32,
    pub object_oid: u32,
    pub row_version: String,
}
impl ObjectAddress {
    fn valid(&self) -> bool {
        self.class_oid != 0
            && self.object_oid != 0
            && !self.row_version.is_empty()
            && self.row_version.len() <= 10
            && self.row_version.bytes().all(|b| b.is_ascii_digit())
    }
}

/// One observed fact that must still hold, unchanged, at apply.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "claim",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum ObjectDdlClaim {
    Existing {
        reference: PgObjectRef,
        address: ObjectAddress,
    },
    Schema {
        name: String,
        address: ObjectAddress,
    },
    /// No relation or type with this name exists in the schema.
    Absent { schema: String, name: String },
}
impl ObjectDdlClaim {
    pub(crate) fn spec(&self) -> ClaimSpec {
        match self {
            Self::Existing { reference, .. } => ClaimSpec::Existing(reference.clone()),
            Self::Schema { name, .. } => ClaimSpec::Schema(name.clone()),
            Self::Absent { schema, name } => ClaimSpec::Absent {
                schema: schema.clone(),
                name: name.clone(),
            },
        }
    }
    /// Whether this observed claim satisfies the operation's requested spec.
    pub(crate) fn satisfies(&self, spec: &ClaimSpec) -> bool {
        match (spec, self) {
            (ClaimSpec::Existing(expected), Self::Existing { reference, .. }) => {
                expected == reference
            }
            (ClaimSpec::Schema(expected), Self::Schema { name, .. }) => expected == name,
            (ClaimSpec::Absent { schema, name }, Self::Absent { schema: s, name: n })
            | (ClaimSpec::ViewOrAbsent { schema, name }, Self::Absent { schema: s, name: n }) => {
                schema == s && name == n
            }
            (ClaimSpec::ViewOrAbsent { schema, name }, Self::Existing { reference, .. }) => {
                reference.kind == PgObjectKind::View
                    && reference.schema.as_ref() == Some(schema)
                    && &reference.name == name
                    && reference.identity_args.is_none()
            }
            _ => false,
        }
    }
    fn heap_bytes(&self) -> Option<usize> {
        match self {
            Self::Existing { reference, address } => {
                if !address.valid() {
                    return None;
                }
                Some(reference_bytes(reference)? + address.row_version.capacity())
            }
            Self::Schema { name, address } => (valid_name(name) && address.valid())
                .then(|| name.capacity() + address.row_version.capacity()),
            Self::Absent { schema, name } => (valid_name(schema) && valid_name(name))
                .then(|| schema.capacity() + name.capacity()),
        }
    }
}

/// Read-only recovery/display information. Deserialization never creates
/// target authority; claims are aligned with operations by index.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ObjectDdlDescription {
    pub database_oid: u32,
    pub claims: Vec<Vec<ObjectDdlClaim>>,
}
impl ObjectDdlDescription {
    pub fn checked_heap_bytes(&self) -> Option<usize> {
        if self.database_oid == 0
            || self.claims.is_empty()
            || self.claims.len() > MAX_OBJECT_DDL_OPERATIONS
            || self.claims.capacity() > MAX_OBJECT_DDL_OPERATIONS
        {
            return None;
        }
        let mut bytes = size_of::<Self>()
            .checked_add(self.claims.capacity() * size_of::<Vec<ObjectDdlClaim>>())?;
        for claims in &self.claims {
            if claims.is_empty() || claims.len() > 3 || claims.capacity() > 4 {
                return None;
            }
            bytes = bytes.checked_add(claims.capacity() * size_of::<ObjectDdlClaim>())?;
            for claim in claims {
                bytes = bytes.checked_add(claim.heap_bytes()?)?;
            }
        }
        (bytes <= MAX_OBJECT_DDL_DESCRIPTION_BYTES * 2
            && self.encoded_bytes() <= MAX_OBJECT_DDL_DESCRIPTION_BYTES)
            .then_some(bytes)
    }
    pub fn encoded_bytes(&self) -> usize {
        encoded_bytes(self)
    }
    /// Claims exactly answer each operation's specs, in order.
    pub(crate) fn answers(&self, operations: &[ObjectDdlOperation]) -> bool {
        self.claims.len() == operations.len()
            && self.claims.iter().zip(operations).all(|(claims, op)| {
                let specs = op.claim_specs();
                claims.len() == specs.len()
                    && claims.iter().zip(&specs).all(|(c, s)| c.satisfies(s))
            })
    }
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ObjectDdlStatement {
    pub sql: String,
    pub summary: String,
    pub destructive: bool,
    pub transactional: bool,
}
impl fmt::Debug for ObjectDdlStatement {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ObjectDdlStatement")
            .field("transactional", &self.transactional)
            .finish_non_exhaustive()
    }
}

/// Atomic groups share one transaction; a standalone statement commits alone.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum ObjectDdlGroup {
    Atomic { statements: Vec<u16> },
    Standalone { statement: u16 },
}
impl ObjectDdlGroup {
    pub fn statements(&self) -> Vec<usize> {
        match self {
            Self::Atomic { statements } => statements.iter().map(|i| usize::from(*i)).collect(),
            Self::Standalone { statement } => vec![usize::from(*statement)],
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ObjectDdlPreview {
    pub statements: Vec<ObjectDdlStatement>,
    pub groups: Vec<ObjectDdlGroup>,
    /// FNV-1a 64 over the canonical operation JSON; display aid only. Apply
    /// binds by exact equality of the regenerated preview, not by digest.
    pub operation_digest: String,
    /// Stored policy at review. Apply refuses if this changed.
    pub confirmation_required: bool,
    pub statement_timeout_ms: Option<u32>,
    pub operation_timeout_ms: u32,
}
impl ObjectDdlPreview {
    pub fn checked_heap_bytes(&self) -> Option<usize> {
        let mut bytes = size_of::<Self>()
            .checked_add(self.statements.capacity() * size_of::<ObjectDdlStatement>())?
            .checked_add(self.groups.capacity() * size_of::<ObjectDdlGroup>())?
            .checked_add(self.operation_digest.capacity())?;
        for statement in &self.statements {
            bytes = bytes
                .checked_add(statement.sql.capacity())?
                .checked_add(statement.summary.capacity())?;
        }
        for group in &self.groups {
            if let ObjectDdlGroup::Atomic { statements } = group {
                bytes = bytes.checked_add(statements.capacity() * 2)?;
            }
        }
        (self.operation_timeout_ms == OBJECT_DDL_OPERATION_TIMEOUT_MS
            && !self.statements.is_empty()
            && self.statements.len() <= MAX_OBJECT_DDL_OPERATIONS
            && self.groups.len() <= MAX_OBJECT_DDL_OPERATIONS
            && bytes <= MAX_OBJECT_DDL_PREVIEW_BYTES * 2
            && encoded_bytes(self) <= MAX_OBJECT_DDL_PREVIEW_BYTES)
            .then_some(bytes)
    }
    pub fn destructive(&self) -> bool {
        self.statements.iter().any(|s| s.destructive)
    }
    pub fn standalone(&self) -> bool {
        self.groups
            .iter()
            .any(|g| matches!(g, ObjectDdlGroup::Standalone { .. }))
    }
    pub fn effect_scope(&self) -> &'static str {
        if self.standalone() {
            OBJECT_DDL_STANDALONE_SCOPE
        } else {
            OBJECT_DDL_ATOMIC_SCOPE
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase", deny_unknown_fields)]
pub enum ObjectDdlFailure {
    Cancelled,
    Timeout,
    Connection,
    TargetChanged,
    UnsupportedTarget,
    Limit,
    RollbackUnconfirmed,
    Database { code: Option<String> },
}

/// How the group at `stopped_at` ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ObjectDdlStop {
    /// PostgreSQL acknowledged ROLLBACK of this atomic group.
    RolledBack,
    /// PostgreSQL rejected this standalone statement; it did not apply.
    Rejected,
    /// Refused before this group was sent (identity changed, cancelled).
    NotDispatched,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum ObjectDdlResidue {
    InvalidIndex {
        schema: String,
        name: String,
    },
    /// The residue check itself failed; inspect the target.
    Unverified,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "state",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum ObjectDdlOutcome {
    /// No statement reached the server.
    NotDispatched { reason: ObjectDdlFailure },
    /// Every statement committed.
    Applied { runtime_ms: u64 },
    /// Statements `0..committed` committed. The group starting at `stopped_at`
    /// ended as `stop`; no later statement was sent.
    Stopped {
        committed: u16,
        stopped_at: u16,
        stop: ObjectDdlStop,
        reason: ObjectDdlFailure,
        residue: Option<ObjectDdlResidue>,
    },
    /// Statements `0..committed` committed; `committed..uncertain_end` may or
    /// may not have committed; no later statement was sent. Never retried.
    OutcomeUnknown {
        committed: u16,
        uncertain_end: u16,
        reason: ObjectDdlFailure,
    },
}
impl ObjectDdlOutcome {
    /// Any statement that committed, or might have.
    pub fn may_have_changed(&self) -> bool {
        match self {
            Self::NotDispatched { .. } => false,
            Self::Applied { .. } | Self::OutcomeUnknown { .. } => true,
            Self::Stopped {
                committed, residue, ..
            } => *committed > 0 || residue.is_some(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ObjectDdlReceipt {
    pub attempt_id: ObjectDdlAttemptId,
    pub connection_id: String,
    pub target: ObjectDdlDescription,
    pub operations: Vec<ObjectDdlOperation>,
    pub outcome: ObjectDdlOutcome,
}
impl ObjectDdlReceipt {
    pub fn retained_bytes(&self) -> usize {
        size_of::<Self>()
            .saturating_add(self.connection_id.capacity())
            .saturating_add(self.attempt_id.as_str().len())
            .saturating_add(self.target.checked_heap_bytes().unwrap_or(usize::MAX))
            .saturating_add(operations_heap_bytes(&self.operations).unwrap_or(usize::MAX))
            .saturating_add(256) // SQLSTATE and residue names
    }
    pub fn encoded_bytes(&self) -> usize {
        encoded_bytes(self)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ObjectDdlError {
    InvalidRequest,
    TargetMismatch,
    Unavailable,
    Limit,
    Storage,
    Busy,
    PolicyBlocked,
    PolicyChanged,
    ForeignDocument,
    OutcomeUnavailable,
}
impl fmt::Display for ObjectDdlError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::InvalidRequest => {
                "Object change is unsupported, invalid, repeats a target or exceeds its bound"
            }
            Self::TargetMismatch => {
                "Target is missing, already exists, or changed since it was observed; inspect again"
            }
            Self::Unavailable => "Object observation is unavailable or retired",
            Self::Limit => "Object observation or review exceeds its bound",
            Self::Storage => "Stored connection is unavailable",
            Self::Busy => "Another schema or maintenance operation owns this connection",
            Self::PolicyBlocked => "Stored policy blocks this object change",
            Self::PolicyChanged => "Stored policy changed since review; review again",
            Self::ForeignDocument => "Object target belongs to a different backend",
            Self::OutcomeUnavailable => {
                "Object change outcome unavailable; reconcile before another attempt"
            }
        })
    }
}
impl std::error::Error for ObjectDdlError {}
