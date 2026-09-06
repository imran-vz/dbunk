use super::super::{budget::*, expression, normalize::Fact, protocol::*};
use serde::Deserialize;
use std::{mem::size_of, sync::Arc};

/// Native snapshot data, never an IPC response. Local OIDs only associate fields
/// with this inventory; matching uses exact portable names.
pub struct CapturedEndpoint {
    pub metadata: CaptureMetadata,
    pub inventory: Vec<InventoryEntry>,
    pub coverage: Coverage,
    pub excluded_counts: Vec<ExcludedCount>,
    pub(super) oids: Vec<u32>,
    pub(super) fields: Vec<CapturedField>,
    pub(super) snapshot: Arc<()>,
    retained_bytes: usize,
    budget: Budget,
    _base: Reservation,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExcludedCount {
    pub category: ExcludedCategory,
    pub count: u32,
    pub complete: bool,
}

pub struct CapturedField {
    pub table_oid: u32,
    pub path: FieldPath,
    pub value: CapturedValue,
    _reservation: Reservation,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "kind", content = "value", rename_all = "camelCase")]
pub enum CapturedValue {
    Null,
    Boolean(bool),
    Integer(i32),
    Text(String),
    Reference(QualifiedName),
    Names(Vec<String>),
    Operators(Vec<OperatorSignature>),
    // Converted from WireValue before retention.
    #[serde(skip)]
    Expression {
        raw: String,
        reason: Option<IncomparableReason>,
    },
    #[serde(skip)]
    UnknownOptions(String),
}

impl CapturedValue {
    /// Rendering compatibility is a pair property. Step 4 must pass both exact
    /// server versions rather than treating every supported PG16 pair alike.
    pub fn fact(&self, server_version: u32, other_version: u32) -> Fact<'_> {
        match self {
            Self::Null => Fact::Null,
            Self::Boolean(v) => Fact::Boolean(*v),
            Self::Integer(v) => Fact::Integer(*v),
            Self::Text(v) => Fact::Text(v),
            Self::Reference(v) => Fact::Reference(v),
            Self::Names(v) => Fact::Names(v),
            Self::Operators(v) => Fact::Operators(v),
            Self::Expression { raw, reason } => {
                let reason = reason.or((server_version != other_version)
                    .then_some(IncomparableReason::RenderingVersionDifference));
                match reason {
                    Some(reason) => Fact::NotComparable {
                        reason,
                        raw: Some(raw),
                    },
                    None => Fact::Text(raw),
                }
            }
            Self::UnknownOptions(raw) => Fact::NotComparable {
                reason: IncomparableReason::UnknownAccessMethod,
                raw: Some(raw),
            },
        }
    }

    fn owned_bytes(&self) -> usize {
        match self {
            Self::Null | Self::Boolean(_) | Self::Integer(_) => 0,
            Self::Text(v) | Self::UnknownOptions(v) | Self::Expression { raw: v, .. } => {
                v.capacity()
            }
            Self::Reference(v) => reference_bytes(v),
            Self::Names(v) => {
                v.capacity() * size_of::<String>() + v.iter().map(String::capacity).sum::<usize>()
            }
            Self::Operators(v) => {
                v.capacity() * size_of::<OperatorSignature>()
                    + v.iter()
                        .map(|o| {
                            reference_bytes(&o.operator)
                                + o.left_type.as_ref().map_or(0, reference_bytes)
                                + o.right_type.as_ref().map_or(0, reference_bytes)
                        })
                        .sum::<usize>()
            }
        }
    }
}

fn reference_bytes(v: &QualifiedName) -> usize {
    v.name.capacity()
        + match &v.namespace {
            Namespace::Selected => 0,
            Namespace::External { schema } => schema.capacity(),
        }
}

fn path_bytes(path: &FieldPath) -> usize {
    match path {
        FieldPath::Table { .. } => 0,
        FieldPath::Column { name, .. } | FieldPath::Constraint { name, .. } => name.capacity(),
        FieldPath::Index { name, owner, .. } | FieldPath::IndexKey { name, owner, .. } => {
            name.capacity() + owner.as_ref().map_or(0, String::capacity)
        }
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct WireField {
    pub table_oid: u32,
    pub path: FieldPath,
    pub fact: WireValue,
}

#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub(super) enum WireValue {
    Expression {
        value: String,
        #[serde(rename = "externalDependency")]
        external_dependency: bool,
    },
    SortOptions {
        value: SortOptions,
    },
    #[serde(untagged)]
    Plain(CapturedValue),
}

#[derive(Deserialize)]
pub(super) struct SortOptions {
    method: String,
    builtin: bool,
    bits: i16,
}

impl CapturedEndpoint {
    pub(super) fn new(metadata: CaptureMetadata, budget: &Budget) -> Result<Self, CompareError> {
        let bytes = size_of::<Self>()
            + SCOPE.len()
            + MAX_VALUES * size_of::<CapturedField>()
            + INVENTORY_ENTRIES * (size_of::<InventoryEntry>() + size_of::<u32>() + 63)
            + 14 * (size_of::<ExcludedCategory>() + size_of::<ExcludedCount>())
            + metadata.endpoint.connection_id.capacity()
            + metadata.endpoint.schema.capacity()
            + metadata.server_version.capacity()
            + metadata.captured_at.capacity();
        if bytes > ENDPOINT_BYTES {
            return Err(CompareError::LimitExceeded {
                limit: Limit::EndpointBytes,
            });
        }
        let base = budget.reserve(bytes)?;
        Ok(Self {
            metadata,
            inventory: Vec::with_capacity(INVENTORY_ENTRIES),
            oids: Vec::with_capacity(INVENTORY_ENTRIES),
            fields: Vec::with_capacity(MAX_VALUES),
            snapshot: Arc::new(()),
            coverage: Coverage {
                scope: SCOPE.into(),
                normalization_version: NORMALIZATION_VERSION,
                excluded_relations: 0,
                incomparable_fields: 0,
                excluded_categories: vec![
                    ExcludedCategory::OtherRelations,
                    ExcludedCategory::Routines,
                    ExcludedCategory::Sequences,
                    ExcludedCategory::TypesAndDomains,
                    ExcludedCategory::Policies,
                    ExcludedCategory::Grants,
                    ExcludedCategory::Triggers,
                    ExcludedCategory::Rules,
                    ExcludedCategory::Extensions,
                    ExcludedCategory::DatabaseObjects,
                    ExcludedCategory::IdentitySequenceConfiguration,
                    ExcludedCategory::StorageSecurityOwnershipReplication,
                    ExcludedCategory::IndexPlacementClusteringReplicaIdentity,
                ],
            },
            excluded_counts: Vec::with_capacity(14),
            retained_bytes: bytes,
            budget: budget.clone(),
            _base: base,
        })
    }

    pub(crate) fn result_budget(&self) -> &Budget {
        &self.budget
    }

    pub(crate) fn shares_snapshot(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.snapshot, &other.snapshot)
    }

    pub(crate) fn local_oid(&self, identity: &RelationIdentity) -> Option<u32> {
        self.inventory
            .iter()
            .position(|entry| entry.identity == *identity)
            .and_then(|i| self.oids.get(i).copied())
    }

    pub fn fields(&self) -> &[CapturedField] {
        &self.fields
    }

    pub fn table(&self, local_oid: u32) -> Option<&InventoryEntry> {
        self.oids
            .iter()
            .position(|oid| *oid == local_oid)
            .map(|i| &self.inventory[i])
    }

    pub fn retained_bytes(&self) -> usize {
        self.retained_bytes
    }

    pub(super) fn push(&mut self, row: WireField) -> Result<(), CompareError> {
        if self.fields.len() == MAX_VALUES {
            return Err(CompareError::LimitExceeded {
                limit: Limit::ChildFacts,
            });
        }
        if !self
            .table(row.table_oid)
            .is_some_and(|t| t.eligibility == Eligibility::Eligible)
        {
            return Err(CompareError::Unavailable);
        }
        let value = match row.fact {
            WireValue::Plain(value) => value,
            WireValue::Expression {
                value,
                external_dependency,
            } => CapturedValue::Expression {
                raw: value,
                reason: external_dependency.then_some(IncomparableReason::ExternalDependency),
            },
            WireValue::SortOptions { value } => {
                if value.builtin && value.method == "btree" && (0..=3).contains(&value.bits) {
                    CapturedValue::Names(vec![
                        if value.bits & 1 == 0 {
                            "ascending"
                        } else {
                            "descending"
                        }
                        .into(),
                        if value.bits & 2 == 0 {
                            "nullsLast"
                        } else {
                            "nullsFirst"
                        }
                        .into(),
                    ])
                } else if value.builtin
                    && matches!(
                        value.method.as_str(),
                        "hash" | "gist" | "gin" | "spgist" | "brin"
                    )
                    && value.bits == 0
                {
                    CapturedValue::Null
                } else {
                    CapturedValue::UnknownOptions(value.bits.to_string())
                }
            }
        };
        let bytes = path_bytes(&row.path) + value.owned_bytes();
        let next = self
            .retained_bytes
            .checked_add(bytes)
            .filter(|next| *next <= ENDPOINT_BYTES)
            .ok_or(CompareError::LimitExceeded {
                limit: Limit::EndpointBytes,
            })?;
        let reservation = self.budget.reserve(bytes)?;
        self.fields.push(CapturedField {
            table_oid: row.table_oid,
            path: row.path,
            value,
            _reservation: reservation,
        });
        self.retained_bytes = next;
        Ok(())
    }

    /// All column facts must be present before expression certification. This
    /// bounded pass borrows the immutable names; it never rewrites SQL text.
    pub(super) fn certify(
        &mut self,
        check: impl Fn() -> Result<(), CompareError>,
    ) -> Result<(), CompareError> {
        let _scratch = self
            .budget
            .scratch(TABLE_ENTRIES * size_of::<u32>() + MAX_VALUES * size_of::<&str>())?;
        let fields = &mut self.fields;
        fields.sort_unstable_by(|a, b| a.table_oid.cmp(&b.table_oid).then(a.path.cmp(&b.path)));
        if fields
            .windows(2)
            .any(|p| p[0].table_oid == p[1].table_oid && p[0].path == p[1].path)
        {
            return Err(CompareError::Unavailable);
        }
        let mut start = 0;
        while start < fields.len() {
            check()?;
            let end =
                start + fields[start..].partition_point(|f| f.table_oid == fields[start].table_oid);
            // Names are small, but borrow splitting cannot retain references into
            // a slice while mutating its values. Reserve the bounded copies.
            let _names = self.budget.scratch(1600 * (size_of::<String>() + 63))?;
            let mut safe = Vec::with_capacity(1600);
            for name in fields[start..end]
                .iter()
                .filter_map(|f| match (&f.path, &f.value) {
                    (
                        FieldPath::Column {
                            name,
                            field: ColumnField::Type,
                        },
                        CapturedValue::Reference(QualifiedName {
                            namespace: Namespace::External { schema },
                            name: ty,
                        }),
                    ) if expression::supported_scalar(schema, ty) => Some(name.clone()),
                    _ => None,
                })
            {
                if safe.len() == 1600 {
                    return Err(CompareError::Unavailable);
                }
                safe.push(name);
            }
            let safe_refs: Vec<_> = safe.iter().map(String::as_str).collect();
            for field in &mut fields[start..end] {
                check()?;
                if let CapturedValue::Expression { raw, reason } = &mut field.value {
                    if reason.is_none() && !expression::comparable(raw, &safe_refs) {
                        *reason = Some(IncomparableReason::ExpressionOutsideSubset);
                    }
                    if reason.is_some() {
                        self.coverage.incomparable_fields += 1;
                    }
                } else if matches!(field.value, CapturedValue::UnknownOptions(_)) {
                    self.coverage.incomparable_fields += 1;
                }
            }
            start = end;
        }
        Ok(())
    }
}
