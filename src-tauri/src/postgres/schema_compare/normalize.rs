//! Pure matching primitives for complete, validated captures. Capture errors
//! cannot enter this layer as missing rows. All output borrows immutable facts;
//! portable identities never contain database OIDs.
use super::{budget::*, protocol::*};
use serde::{Deserialize, Serialize};
use std::mem::size_of;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum DifferenceKind {
    SourceOnly,
    TargetOnly,
    Changed,
    Equal,
    NotComparable,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", content = "value", rename_all = "camelCase")]
pub enum Fact<'a> {
    Null,
    Boolean(bool),
    Integer(i32),
    Text(&'a str),
    Reference(&'a QualifiedName),
    Names(&'a [String]),
    References(&'a [QualifiedName]),
    Operators(&'a [OperatorSignature]),
    NotComparable {
        reason: IncomparableReason,
        raw: Option<&'a str>,
    },
}

pub struct FieldFact<'a> {
    pub path: &'a FieldPath,
    pub fact: Fact<'a>,
}

pub struct FieldDifference<'a> {
    pub path: &'a FieldPath,
    pub kind: DifferenceKind,
    pub source: Option<Fact<'a>>,
    pub target: Option<Fact<'a>>,
}

pub struct FieldDifferences<'a> {
    fields: Vec<FieldDifference<'a>>,
    _reservation: Reservation,
}

impl<'a> FieldDifferences<'a> {
    pub fn fields(&self) -> &[FieldDifference<'a>] {
        &self.fields
    }

    /// A known difference and an incomparable field coexist in fields().
    /// Incomparability never disappears behind an object-level changed label.
    pub fn kind(&self) -> DifferenceKind {
        if self.fields.iter().any(|f| {
            matches!(
                f.kind,
                DifferenceKind::Changed | DifferenceKind::SourceOnly | DifferenceKind::TargetOnly
            )
        }) {
            DifferenceKind::Changed
        } else if self
            .fields
            .iter()
            .any(|f| f.kind == DifferenceKind::NotComparable)
        {
            DifferenceKind::NotComparable
        } else {
            DifferenceKind::Equal
        }
    }
}

pub fn compare_fields<'a>(
    source: &'a [FieldFact<'a>],
    target: &'a [FieldFact<'a>],
    budget: &Budget,
) -> Result<FieldDifferences<'a>, CompareError> {
    compare_fields_checked(source, target, budget, || Ok(()))
}

pub(crate) fn compare_fields_checked<'a>(
    source: &'a [FieldFact<'a>],
    target: &'a [FieldFact<'a>],
    budget: &Budget,
    mut check: impl FnMut() -> Result<(), CompareError>,
) -> Result<FieldDifferences<'a>, CompareError> {
    check()?;
    if source.len() > MAX_VALUES || target.len() > MAX_VALUES {
        return Err(CompareError::LimitExceeded {
            limit: Limit::ChildFacts,
        });
    }
    let count = source.len() + target.len();
    let bytes = count * (size_of::<&FieldFact<'_>>() + size_of::<FieldDifference<'_>>());
    if bytes > RESULT_BYTES {
        return Err(CompareError::LimitExceeded {
            limit: Limit::ResultBytes,
        });
    }
    let reservation = budget.reserve(bytes)?;
    let mut source: Vec<_> = source.iter().collect();
    let mut target: Vec<_> = target.iter().collect();
    source.sort_unstable_by_key(|f| f.path);
    target.sort_unstable_by_key(|f| f.path);
    check()?;
    for fields in [&source, &target] {
        if fields.windows(2).any(|pair| pair[0].path == pair[1].path) {
            return Err(CompareError::InvalidRequest);
        }
    }
    let mut fields = Vec::with_capacity(count);
    let (mut left, mut right) = (0, 0);
    while left < source.len() || right < target.len() {
        check()?;
        let s = source.get(left).copied();
        let t = target.get(right).copied();
        let ordering = match (s, t) {
            (Some(s), Some(t)) => s.path.cmp(t.path),
            (Some(_), None) => std::cmp::Ordering::Less,
            (None, Some(_)) => std::cmp::Ordering::Greater,
            (None, None) => break,
        };
        let (s, t) = match ordering {
            std::cmp::Ordering::Less => {
                left += 1;
                (s, None)
            }
            std::cmp::Ordering::Greater => {
                right += 1;
                (None, t)
            }
            std::cmp::Ordering::Equal => {
                left += 1;
                right += 1;
                (s, t)
            }
        };
        let path = s.or(t).expect("at least one field").path;
        let (source, target) = (s.map(|f| f.fact), t.map(|f| f.fact));
        let kind = difference_kind(source, target);
        fields.push(FieldDifference {
            path,
            source,
            target,
            kind,
        });
    }
    Ok(FieldDifferences {
        fields,
        _reservation: reservation,
    })
}

pub(crate) fn difference_kind(
    source: Option<Fact<'_>>,
    target: Option<Fact<'_>>,
) -> DifferenceKind {
    match (source, target) {
        (Some(Fact::NotComparable { .. }), _) | (_, Some(Fact::NotComparable { .. })) => {
            DifferenceKind::NotComparable
        }
        (Some(_), None) => DifferenceKind::SourceOnly,
        (None, Some(_)) => DifferenceKind::TargetOnly,
        (s, t) if s == t => DifferenceKind::Equal,
        _ => DifferenceKind::Changed,
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ObjectMatch<'a> {
    pub source: Option<&'a RelationIdentity>,
    pub target: Option<&'a RelationIdentity>,
    pub kind: MatchKind,
    pub reason: Option<IncomparableReason>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum MatchKind {
    SourceOnly,
    TargetOnly,
    Matched,
    NotComparable,
}

pub struct ObjectMatches<'a> {
    objects: Vec<ObjectMatch<'a>>,
    _reservation: Reservation,
}

impl<'a> ObjectMatches<'a> {
    pub fn objects(&self) -> &[ObjectMatch<'a>] {
        &self.objects
    }
}

/// Both inventories must have passed schema-existence and completeness checks.
/// Eligible pairs are marked Matched, never Equal, until definitions are compared.
pub fn match_inventory<'a>(
    source: &'a [InventoryEntry],
    target: &'a [InventoryEntry],
    budget: &Budget,
) -> Result<ObjectMatches<'a>, CompareError> {
    match_inventory_checked(source, target, budget, || Ok(()))
}

pub(crate) fn match_inventory_checked<'a>(
    source: &'a [InventoryEntry],
    target: &'a [InventoryEntry],
    budget: &Budget,
    mut check: impl FnMut() -> Result<(), CompareError>,
) -> Result<ObjectMatches<'a>, CompareError> {
    check()?;
    if source.len() > INVENTORY_ENTRIES || target.len() > INVENTORY_ENTRIES {
        return Err(CompareError::LimitExceeded {
            limit: Limit::Inventory,
        });
    }
    let count = source.len() + target.len();
    let reservation =
        budget.reserve(count * (size_of::<&InventoryEntry>() + size_of::<ObjectMatch<'_>>()))?;
    let mut source: Vec<_> = source.iter().collect();
    let mut target: Vec<_> = target.iter().collect();
    for rows in [&mut source, &mut target] {
        check()?;
        rows.sort_unstable_by_key(|r| &r.identity.name);
        if rows.iter().any(|row| {
            row.identity.name.is_empty()
                || row.identity.name.len() > 63
                || (row.eligibility == Eligibility::Eligible
                    && row.identity.kind != RelationKind::Table)
        }) || rows
            .windows(2)
            .any(|pair| pair[0].identity.name == pair[1].identity.name)
        {
            return Err(CompareError::InvalidRequest);
        }
        if rows
            .iter()
            .filter(|row| row.eligibility == Eligibility::Eligible)
            .count()
            > TABLE_ENTRIES
        {
            return Err(CompareError::LimitExceeded {
                limit: Limit::Tables,
            });
        }
    }
    let mut objects = Vec::with_capacity(count);
    let (mut left, mut right) = (0, 0);
    while left < source.len() || right < target.len() {
        check()?;
        let s = source.get(left).copied();
        let t = target.get(right).copied();
        let (s, t) = match (s, t) {
            (Some(s), Some(t)) if s.identity.name == t.identity.name => {
                left += 1;
                right += 1;
                (Some(s), Some(t))
            }
            (Some(s), Some(t)) if s.identity.name < t.identity.name => {
                left += 1;
                (Some(s), None)
            }
            (Some(_), Some(t)) => {
                right += 1;
                (None, Some(t))
            }
            (Some(s), None) => {
                left += 1;
                (Some(s), None)
            }
            (None, Some(t)) => {
                right += 1;
                (None, Some(t))
            }
            (None, None) => break,
        };
        let excluded = s
            .into_iter()
            .chain(t)
            .any(|row| row.eligibility != Eligibility::Eligible);
        let (kind, reason) = if excluded {
            (
                MatchKind::NotComparable,
                Some(if s.is_some() && t.is_some() {
                    IncomparableReason::ExcludedCounterpart
                } else {
                    IncomparableReason::ExcludedObject
                }),
            )
        } else {
            (
                match (s, t) {
                    (Some(_), None) => MatchKind::SourceOnly,
                    (None, Some(_)) => MatchKind::TargetOnly,
                    _ => MatchKind::Matched,
                },
                None,
            )
        };
        objects.push(ObjectMatch {
            source: s.map(|s| &s.identity),
            target: t.map(|t| &t.identity),
            kind,
            reason,
        });
    }
    Ok(ObjectMatches {
        objects,
        _reservation: reservation,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_only_structured_selected_schema_references() {
        let source = Endpoint {
            connection_id: "a".into(),
            schema: "Source".into(),
        };
        let target = Endpoint {
            connection_id: "b".into(),
            schema: "Target".into(),
        };
        assert_eq!(
            QualifiedName::in_endpoint(&source, "Source", "Mixed"),
            QualifiedName::in_endpoint(&target, "Target", "Mixed")
        );
        assert_ne!(
            QualifiedName::in_endpoint(&source, "external", "Mixed"),
            QualifiedName::in_endpoint(&target, "Target", "Mixed")
        );
        assert_ne!(
            Fact::Text("'Source.literal'::text"),
            Fact::Text("'Target.literal'::text")
        );
    }

    #[test]
    fn field_order_is_deterministic_and_changed_does_not_hide_incomparability() {
        let a = FieldPath::Column {
            name: "id".into(),
            field: ColumnField::Position,
        };
        let b = FieldPath::Column {
            name: "id".into(),
            field: ColumnField::Default,
        };
        let source = [
            FieldFact {
                path: &b,
                fact: Fact::NotComparable {
                    reason: IncomparableReason::ExpressionOutsideSubset,
                    raw: Some("external.f(7)"),
                },
            },
            FieldFact {
                path: &a,
                fact: Fact::Integer(1),
            },
        ];
        let target = [
            FieldFact {
                path: &a,
                fact: Fact::Integer(2),
            },
            FieldFact {
                path: &b,
                fact: Fact::Text("7"),
            },
        ];
        let diff = compare_fields(&source, &target, &Budget::default()).unwrap();
        assert_eq!(diff.kind(), DifferenceKind::Changed);
        assert_eq!(diff.fields()[0].path, &a);
        assert_eq!(diff.fields()[1].kind, DifferenceKind::NotComparable);
    }

    #[test]
    fn excluded_counterparts_never_become_directional_absence() {
        let source = [InventoryEntry {
            identity: RelationIdentity {
                kind: RelationKind::Table,
                name: "orders".into(),
            },
            eligibility: Eligibility::Eligible,
        }];
        let target = [InventoryEntry {
            identity: RelationIdentity {
                kind: RelationKind::View,
                name: "orders".into(),
            },
            eligibility: Eligibility::Excluded {
                reason: Exclusion::OtherKind,
            },
        }];
        for (s, t) in [(&source[..], &target[..]), (&target[..], &source[..])] {
            let diff = match_inventory(s, t, &Budget::default()).unwrap();
            assert_eq!(diff.objects()[0].kind, MatchKind::NotComparable);
            assert_eq!(
                diff.objects()[0].reason,
                Some(IncomparableReason::ExcludedCounterpart)
            );
        }
        let only = match_inventory(&source, &[], &Budget::default()).unwrap();
        assert_eq!(only.objects()[0].kind, MatchKind::SourceOnly);
        assert!(match_inventory(&[], &[], &Budget::default())
            .unwrap()
            .objects()
            .is_empty());
    }

    #[test]
    fn inventory_and_table_caps_are_distinct() {
        let budget = Budget::default();
        let mut entries: Vec<_> = (0..INVENTORY_ENTRIES)
            .map(|i| InventoryEntry {
                identity: RelationIdentity {
                    kind: RelationKind::View,
                    name: format!("v{i}"),
                },
                eligibility: Eligibility::Excluded {
                    reason: Exclusion::OtherKind,
                },
            })
            .collect();
        assert!(match_inventory(&entries, &[], &budget).is_ok());
        entries.push(InventoryEntry {
            identity: RelationIdentity {
                kind: RelationKind::View,
                name: "extra".into(),
            },
            eligibility: Eligibility::Excluded {
                reason: Exclusion::OtherKind,
            },
        });
        assert!(matches!(
            match_inventory(&entries, &[], &budget),
            Err(CompareError::LimitExceeded {
                limit: Limit::Inventory
            })
        ));
        entries.truncate(TABLE_ENTRIES);
        for entry in &mut entries {
            entry.identity.kind = RelationKind::Table;
            entry.eligibility = Eligibility::Eligible;
        }
        assert!(match_inventory(&entries, &[], &budget).is_ok());
        entries.push(InventoryEntry {
            identity: RelationIdentity {
                kind: RelationKind::Table,
                name: "extra".into(),
            },
            eligibility: Eligibility::Eligible,
        });
        assert!(matches!(
            match_inventory(&entries, &[], &budget),
            Err(CompareError::LimitExceeded {
                limit: Limit::Tables
            })
        ));
    }

    #[test]
    fn fk_delete_subsets_and_constraint_owned_index_state_survive_matching() {
        let keys = FieldPath::Constraint {
            name: "fk".into(),
            field: ConstraintField::DeleteColumns,
        };
        let valid = FieldPath::Index {
            name: "unique_idx".into(),
            owner: Some("unique".into()),
            field: IndexField::Valid,
        };
        let nulls = FieldPath::Index {
            name: "unique_idx".into(),
            owner: Some("unique".into()),
            field: IndexField::NullsNotDistinct,
        };
        let left_keys = ["tenant".into(), "id".into()];
        let right_keys = ["id".into(), "tenant".into()];
        let source = [
            FieldFact {
                path: &keys,
                fact: Fact::Names(&left_keys),
            },
            FieldFact {
                path: &valid,
                fact: Fact::Boolean(false),
            },
            FieldFact {
                path: &nulls,
                fact: Fact::Boolean(true),
            },
        ];
        let target = [
            FieldFact {
                path: &nulls,
                fact: Fact::Boolean(false),
            },
            FieldFact {
                path: &keys,
                fact: Fact::Names(&right_keys),
            },
            FieldFact {
                path: &valid,
                fact: Fact::Boolean(true),
            },
        ];
        let diff = compare_fields(&source, &target, &Budget::default()).unwrap();
        assert_eq!(diff.fields().len(), 3);
        assert!(diff
            .fields()
            .iter()
            .all(|field| field.kind == DifferenceKind::Changed));
    }

    #[test]
    fn mixed_index_keys_compare_by_ordinal_with_explicit_optional_facts() {
        let key = |position, field| FieldPath::IndexKey {
            name: "mixed_idx".into(),
            owner: None,
            position,
            field,
        };
        let key_0_kind = key(0, IndexKeyField::Kind);
        let key_0_column = key(0, IndexKeyField::Column);
        let key_0_collation = key(0, IndexKeyField::Collation);
        let key_1_kind = key(1, IndexKeyField::Kind);
        let key_1_expression = key(1, IndexKeyField::Expression);
        let key_1_options = key(1, IndexKeyField::OpclassOptions);
        let key_2_kind = key(2, IndexKeyField::Kind);
        let key_2_column = key(2, IndexKeyField::Column);
        let collation = QualifiedName {
            namespace: Namespace::External {
                schema: "pg_catalog".into(),
            },
            name: "default".into(),
        };
        let source_options = ["compression=pglz".into()];

        // The source keys are a, (b + external.f(1)), c. The target swaps the
        // column keys, adds a per-key collation, and removes expression options.
        let source = [
            FieldFact {
                path: &key_1_expression,
                fact: Fact::NotComparable {
                    reason: IncomparableReason::ExpressionOutsideSubset,
                    raw: Some("b + external.f(1)"),
                },
            },
            FieldFact {
                path: &key_0_column,
                fact: Fact::Text("a"),
            },
            FieldFact {
                path: &key_2_column,
                fact: Fact::Text("c"),
            },
            FieldFact {
                path: &key_0_kind,
                fact: Fact::Text("column"),
            },
            FieldFact {
                path: &key_1_kind,
                fact: Fact::Text("expression"),
            },
            FieldFact {
                path: &key_2_kind,
                fact: Fact::Text("column"),
            },
            FieldFact {
                path: &key_0_collation,
                fact: Fact::Null,
            },
            FieldFact {
                path: &key_1_options,
                fact: Fact::Names(&source_options),
            },
        ];
        let target = [
            FieldFact {
                path: &key_2_column,
                fact: Fact::Text("a"),
            },
            FieldFact {
                path: &key_1_options,
                fact: Fact::Null,
            },
            FieldFact {
                path: &key_1_expression,
                fact: Fact::Text("b + 1"),
            },
            FieldFact {
                path: &key_0_column,
                fact: Fact::Text("c"),
            },
            FieldFact {
                path: &key_0_kind,
                fact: Fact::Text("column"),
            },
            FieldFact {
                path: &key_1_kind,
                fact: Fact::Text("expression"),
            },
            FieldFact {
                path: &key_2_kind,
                fact: Fact::Text("column"),
            },
            FieldFact {
                path: &key_0_collation,
                fact: Fact::Reference(&collation),
            },
        ];

        let diff = compare_fields(&source, &target, &Budget::default()).unwrap();
        assert_eq!(diff.kind(), DifferenceKind::Changed);
        assert_eq!(
            diff.fields()
                .iter()
                .filter(|field| field.kind == DifferenceKind::Changed)
                .count(),
            4
        );
        assert_eq!(
            diff.fields()
                .iter()
                .filter(|field| field.kind == DifferenceKind::NotComparable)
                .count(),
            1
        );
        assert!(diff.fields().iter().any(|field| {
            field.path == &key_0_collation
                && field.source == Some(Fact::Null)
                && field.target == Some(Fact::Reference(&collation))
        }));
        assert!(diff.fields().iter().any(|field| {
            field.path == &key_1_options
                && field.source == Some(Fact::Names(&source_options))
                && field.target == Some(Fact::Null)
        }));
        assert!(diff.fields().iter().any(|field| {
            field.path == &key_1_expression && field.kind == DifferenceKind::NotComparable
        }));
        assert!(diff
            .fields()
            .iter()
            .any(|field| { field.path == &key_0_column && field.kind == DifferenceKind::Changed }));
        assert!(diff
            .fields()
            .iter()
            .any(|field| { field.path == &key_2_column && field.kind == DifferenceKind::Changed }));
    }
}
