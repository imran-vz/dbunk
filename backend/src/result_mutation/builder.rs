use std::collections::HashSet;

use crate::quote_double;

use super::protocol::{
    ColumnWritability, DmlParam, InvalidPlanReason, MutationIdentity, MutationIdentityKind,
    MutationOp, MutationPlan, MutationTable, MutationValue, PreviewStatement, ResultMutationError,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct MutationColumnDescriptor {
    pub name: String,
    pub cast_type: String,
    pub nullable: bool,
    pub writability: ColumnWritability,
    pub has_default: bool,
    pub has_identity: bool,
    pub projected: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct MutationTableDescriptor {
    pub schema: String,
    pub table: String,
    pub columns: Vec<MutationColumnDescriptor>,
    pub identity: MutationIdentity,
}

impl MutationTableDescriptor {
    pub(crate) fn column(&self, name: &str) -> Option<&MutationColumnDescriptor> {
        self.columns.iter().find(|column| column.name == name)
    }

    fn projected_columns(&self) -> impl Iterator<Item = &str> {
        self.columns
            .iter()
            .filter(|column| {
                column.projected
                    && !(column.writability == ColumnWritability::SystemColumn
                        && self.identity.columns.contains(&column.name))
            })
            .map(|column| column.name.as_str())
    }
}

/// Build the exact statements shared by preview and apply. The function is
/// deliberately pure: validation depends only on the cached analysis snapshot.
pub(crate) fn build_mutation_plan(
    descriptors: &[MutationTableDescriptor],
    plan: &MutationPlan,
) -> Result<Vec<PreviewStatement>, ResultMutationError> {
    plan.operations
        .iter()
        .enumerate()
        .map(|(op_index, operation)| {
            build_operation(descriptors, operation).map(|(sql, params)| PreviewStatement {
                op_index,
                sql,
                params,
            })
        })
        .collect()
}

fn build_operation(
    descriptors: &[MutationTableDescriptor],
    operation: &MutationOp,
) -> Result<(String, Vec<DmlParam>), ResultMutationError> {
    match operation {
        MutationOp::Update {
            table,
            identity,
            guards,
            set,
        } => build_update(require_table(descriptors, table)?, identity, guards, set),
        MutationOp::Delete {
            table,
            identity,
            guards,
        } => {
            require_single_origin_table(descriptors)?;
            build_delete(require_table(descriptors, table)?, identity, guards)
        }
        MutationOp::Insert { table, values } => {
            require_single_origin_table(descriptors)?;
            build_insert(require_table(descriptors, table)?, values)
        }
    }
}

fn require_single_origin_table(
    descriptors: &[MutationTableDescriptor],
) -> Result<(), ResultMutationError> {
    if descriptors.len() != 1 {
        return Err(invalid(InvalidPlanReason::MultipleOriginTables));
    }
    Ok(())
}

fn require_table<'a>(
    descriptors: &'a [MutationTableDescriptor],
    table: &MutationTable,
) -> Result<&'a MutationTableDescriptor, ResultMutationError> {
    descriptors
        .iter()
        .find(|descriptor| descriptor.schema == table.schema && descriptor.table == table.table)
        .ok_or(ResultMutationError::InvalidPlan {
            reason: InvalidPlanReason::TableMismatch,
        })
}

fn build_update(
    descriptor: &MutationTableDescriptor,
    identity: &[MutationValue],
    guards: &[MutationValue],
    set: &[MutationValue],
) -> Result<(String, Vec<DmlParam>), ResultMutationError> {
    if set.is_empty() {
        return Err(invalid(InvalidPlanReason::EmptySet));
    }
    validate_unique_columns(set)?;
    validate_writes(descriptor, set)?;
    validate_identity(descriptor, identity)?;
    validate_guards(descriptor, guards, GuardPolicy::Update(set))?;

    let mut params = Vec::with_capacity(set.len() + identity.len() + guards.len());
    let set_sql = set
        .iter()
        .map(|value| {
            let column = require_column(descriptor, &value.column)?;
            Ok(format!(
                "{} = {}",
                quote_double(&value.column),
                bind_value(value, &column.cast_type, &mut params)
            ))
        })
        .collect::<Result<Vec<_>, ResultMutationError>>()?;
    let predicates = render_predicates(descriptor, identity, guards, &mut params)?;

    Ok((
        format!(
            "UPDATE {} SET {} WHERE {}",
            qualified(descriptor),
            set_sql.join(", "),
            predicates.join(" AND ")
        ),
        params,
    ))
}

fn build_delete(
    descriptor: &MutationTableDescriptor,
    identity: &[MutationValue],
    guards: &[MutationValue],
) -> Result<(String, Vec<DmlParam>), ResultMutationError> {
    validate_identity(descriptor, identity)?;
    validate_guards(descriptor, guards, GuardPolicy::FullRow)?;

    let mut params = Vec::with_capacity(identity.len() + guards.len());
    let predicates = render_predicates(descriptor, identity, guards, &mut params)?;
    Ok((
        format!(
            "DELETE FROM {} WHERE {}",
            qualified(descriptor),
            predicates.join(" AND ")
        ),
        params,
    ))
}

fn build_insert(
    descriptor: &MutationTableDescriptor,
    values: &[MutationValue],
) -> Result<(String, Vec<DmlParam>), ResultMutationError> {
    validate_unique_columns(values)?;
    validate_writes(descriptor, values)?;
    if values.is_empty() {
        return Ok((
            format!("INSERT INTO {} DEFAULT VALUES", qualified(descriptor)),
            Vec::new(),
        ));
    }

    let mut params = Vec::with_capacity(values.len());
    let columns = values
        .iter()
        .map(|value| quote_double(&value.column))
        .collect::<Vec<_>>();
    let placeholders = values
        .iter()
        .map(|value| {
            let column = require_column(descriptor, &value.column)?;
            Ok(bind_value(value, &column.cast_type, &mut params))
        })
        .collect::<Result<Vec<_>, ResultMutationError>>()?;
    Ok((
        format!(
            "INSERT INTO {} ({}) VALUES ({})",
            qualified(descriptor),
            columns.join(", "),
            placeholders.join(", ")
        ),
        params,
    ))
}

fn validate_writes(
    descriptor: &MutationTableDescriptor,
    values: &[MutationValue],
) -> Result<(), ResultMutationError> {
    for value in values {
        let column = require_column(descriptor, &value.column)?;
        match column.writability {
            ColumnWritability::Writable => {}
            ColumnWritability::Generated => {
                return Err(invalid(InvalidPlanReason::GeneratedColumn));
            }
            ColumnWritability::IdentityAlways => {
                return Err(invalid(InvalidPlanReason::IdentityAlwaysColumn));
            }
            ColumnWritability::SystemColumn => {
                return Err(invalid(InvalidPlanReason::SystemColumn));
            }
        }
    }
    Ok(())
}

fn validate_identity(
    descriptor: &MutationTableDescriptor,
    identity: &[MutationValue],
) -> Result<(), ResultMutationError> {
    if identity.is_empty() {
        return Err(invalid(InvalidPlanReason::EmptyIdentity));
    }
    if descriptor.identity.kind == MutationIdentityKind::None
        || descriptor.identity.columns.is_empty()
    {
        return Err(invalid(InvalidPlanReason::NoIdentity));
    }
    validate_unique_columns(identity)?;
    for value in identity {
        require_projected_column(descriptor, &value.column)?;
    }
    if identity
        .iter()
        .map(|value| value.column.as_str())
        .ne(descriptor.identity.columns.iter().map(String::as_str))
    {
        return Err(invalid(InvalidPlanReason::IdentityMismatch));
    }
    if descriptor.identity.kind.is_keyed() && identity.iter().any(|value| value.value.is_none()) {
        return Err(invalid(InvalidPlanReason::NullKeyedIdentity));
    }
    Ok(())
}

enum GuardPolicy<'a> {
    Update(&'a [MutationValue]),
    FullRow,
}

fn validate_guards(
    descriptor: &MutationTableDescriptor,
    guards: &[MutationValue],
    policy: GuardPolicy<'_>,
) -> Result<(), ResultMutationError> {
    validate_unique_columns(guards)?;
    for guard in guards {
        require_projected_column(descriptor, &guard.column)?;
    }

    let provided = guards
        .iter()
        .map(|guard| guard.column.as_str())
        .collect::<HashSet<_>>();
    let required = if descriptor.identity.kind.requires_full_row_guards()
        || matches!(policy, GuardPolicy::FullRow)
    {
        descriptor.projected_columns().collect::<Vec<_>>()
    } else {
        match policy {
            GuardPolicy::Update(set) => set
                .iter()
                .map(|value| value.column.as_str())
                .collect::<Vec<_>>(),
            GuardPolicy::FullRow => unreachable!(),
        }
    };
    if required.iter().any(|column| !provided.contains(column)) {
        return Err(invalid(InvalidPlanReason::MissingGuard));
    }
    Ok(())
}

fn validate_unique_columns(values: &[MutationValue]) -> Result<(), ResultMutationError> {
    let mut columns = HashSet::with_capacity(values.len());
    if values
        .iter()
        .any(|value| !columns.insert(value.column.as_str()))
    {
        return Err(invalid(InvalidPlanReason::DuplicateColumn));
    }
    Ok(())
}

fn require_column<'a>(
    descriptor: &'a MutationTableDescriptor,
    name: &str,
) -> Result<&'a MutationColumnDescriptor, ResultMutationError> {
    descriptor
        .column(name)
        .ok_or_else(|| ResultMutationError::UnknownColumn {
            column: name.to_string(),
        })
}

fn require_projected_column<'a>(
    descriptor: &'a MutationTableDescriptor,
    name: &str,
) -> Result<&'a MutationColumnDescriptor, ResultMutationError> {
    let column = require_column(descriptor, name)?;
    if !column.projected {
        return Err(ResultMutationError::UnknownColumn {
            column: name.to_string(),
        });
    }
    Ok(column)
}

fn render_predicates(
    descriptor: &MutationTableDescriptor,
    identity: &[MutationValue],
    guards: &[MutationValue],
    params: &mut Vec<DmlParam>,
) -> Result<Vec<String>, ResultMutationError> {
    identity
        .iter()
        .chain(guards)
        .map(|value| render_predicate(descriptor, value, params))
        .collect()
}

fn render_predicate(
    descriptor: &MutationTableDescriptor,
    value: &MutationValue,
    params: &mut Vec<DmlParam>,
) -> Result<String, ResultMutationError> {
    let column = require_projected_column(descriptor, &value.column)?;
    Ok(match value.value {
        Some(_) => format!(
            "{} = {}",
            quote_double(&value.column),
            bind_value(value, &column.cast_type, params)
        ),
        None => format!("{} IS NULL", quote_double(&value.column)),
    })
}

fn bind_value(value: &MutationValue, cast_type: &str, params: &mut Vec<DmlParam>) -> String {
    params.push(DmlParam::Text {
        value: value.value.clone(),
    });
    format!("(${}::text)::{}", params.len(), cast_type)
}

fn qualified(descriptor: &MutationTableDescriptor) -> String {
    format!(
        "{}.{}",
        quote_double(&descriptor.schema),
        quote_double(&descriptor.table)
    )
}

fn invalid(reason: InvalidPlanReason) -> ResultMutationError {
    ResultMutationError::InvalidPlan { reason }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn column(
        name: &str,
        cast_type: &str,
        writability: ColumnWritability,
    ) -> MutationColumnDescriptor {
        MutationColumnDescriptor {
            name: name.into(),
            cast_type: cast_type.into(),
            nullable: true,
            writability,
            has_default: false,
            has_identity: false,
            projected: true,
        }
    }

    fn descriptor(kind: MutationIdentityKind, identity: &[&str]) -> MutationTableDescriptor {
        MutationTableDescriptor {
            schema: "public".into(),
            table: "users".into(),
            columns: vec![
                column("id", "integer", ColumnWritability::Writable),
                column("tenant_id", "uuid", ColumnWritability::Writable),
                column("name", "character varying", ColumnWritability::Writable),
                column("note", "text", ColumnWritability::Writable),
            ],
            identity: MutationIdentity {
                kind,
                columns: identity.iter().map(|value| (*value).into()).collect(),
            },
        }
    }

    fn table() -> MutationTable {
        MutationTable {
            schema: "public".into(),
            table: "users".into(),
        }
    }

    fn value(column: &str, value: Option<&str>) -> MutationValue {
        MutationValue {
            column: column.into(),
            value: value.map(str::to_string),
        }
    }

    fn update(
        identity: Vec<MutationValue>,
        guards: Vec<MutationValue>,
        set: Vec<MutationValue>,
    ) -> MutationPlan {
        MutationPlan {
            operations: vec![MutationOp::Update {
                table: table(),
                identity,
                guards,
                set,
            }],
        }
    }

    fn only_statement(
        descriptor: &MutationTableDescriptor,
        plan: &MutationPlan,
    ) -> PreviewStatement {
        build_mutation_plan(std::slice::from_ref(descriptor), plan)
            .unwrap()
            .remove(0)
    }

    #[test]
    fn update_with_single_identity_has_exact_sql_and_params() {
        let descriptor = descriptor(MutationIdentityKind::PrimaryKey, &["id"]);
        let built = only_statement(
            &descriptor,
            &update(
                vec![value("id", Some("7"))],
                vec![value("name", Some("old"))],
                vec![value("name", Some("new"))],
            ),
        );
        assert_eq!(
            built.sql,
            "UPDATE \"public\".\"users\" SET \"name\" = ($1::text)::character varying WHERE \"id\" = ($2::text)::integer AND \"name\" = ($3::text)::character varying"
        );
        assert_eq!(
            built.params,
            vec![
                DmlParam::Text {
                    value: Some("new".into())
                },
                DmlParam::Text {
                    value: Some("7".into())
                },
                DmlParam::Text {
                    value: Some("old".into())
                },
            ]
        );
    }

    #[test]
    fn update_with_multi_column_identity_preserves_order() {
        let descriptor = descriptor(MutationIdentityKind::UniqueIndex, &["tenant_id", "id"]);
        let built = only_statement(
            &descriptor,
            &update(
                vec![value("tenant_id", Some("tenant")), value("id", Some("7"))],
                vec![value("note", Some("before"))],
                vec![value("note", Some("after"))],
            ),
        );
        assert_eq!(
            built.sql,
            "UPDATE \"public\".\"users\" SET \"note\" = ($1::text)::text WHERE \"tenant_id\" = ($2::text)::uuid AND \"id\" = ($3::text)::integer AND \"note\" = ($4::text)::text"
        );
        assert_eq!(built.params.len(), 4);
    }

    #[test]
    fn null_guards_use_is_null_without_consuming_a_parameter() {
        let descriptor = descriptor(MutationIdentityKind::PrimaryKey, &["id"]);
        let built = only_statement(
            &descriptor,
            &update(
                vec![value("id", Some("7"))],
                vec![value("name", None), value("note", Some("old"))],
                vec![value("name", Some("new")), value("note", Some("new note"))],
            ),
        );
        assert_eq!(
            built.sql,
            "UPDATE \"public\".\"users\" SET \"name\" = ($1::text)::character varying, \"note\" = ($2::text)::text WHERE \"id\" = ($3::text)::integer AND \"name\" IS NULL AND \"note\" = ($4::text)::text"
        );
        assert_eq!(built.params.len(), 4);
        assert_eq!(
            built.params[3],
            DmlParam::Text {
                value: Some("old".into())
            }
        );
    }

    #[test]
    fn virtual_and_ctid_updates_require_and_render_full_row_guards() {
        for (kind, identity_column, identity_cast) in [
            (
                MutationIdentityKind::VirtualKey,
                "name",
                "character varying",
            ),
            (MutationIdentityKind::CtidFallback, "ctid", "tid"),
        ] {
            let mut descriptor = descriptor(kind, &[identity_column]);
            if kind == MutationIdentityKind::CtidFallback {
                descriptor
                    .columns
                    .push(column("ctid", "tid", ColumnWritability::SystemColumn));
            }
            let identity = value(identity_column, Some("key"));
            let guards = descriptor
                .projected_columns()
                .map(|name| value(name, Some("old")))
                .collect();
            let built = only_statement(
                &descriptor,
                &update(vec![identity], guards, vec![value("note", Some("new"))]),
            );
            assert!(built.sql.contains(&format!(
                "\"{identity_column}\" = ($2::text)::{identity_cast}"
            )));
            for column in descriptor.projected_columns() {
                assert!(built.sql.matches(&format!("\"{column}\" =")).count() >= 1);
            }
        }
    }

    #[test]
    fn keyed_delete_uses_full_projected_row_guards() {
        let descriptor = descriptor(MutationIdentityKind::PrimaryKey, &["id"]);
        let plan = MutationPlan {
            operations: vec![MutationOp::Delete {
                table: table(),
                identity: vec![value("id", Some("7"))],
                guards: vec![
                    value("id", Some("7")),
                    value("tenant_id", Some("tenant")),
                    value("name", None),
                    value("note", Some("old")),
                ],
            }],
        };
        let built = only_statement(&descriptor, &plan);
        assert_eq!(
            built.sql,
            "DELETE FROM \"public\".\"users\" WHERE \"id\" = ($1::text)::integer AND \"id\" = ($2::text)::integer AND \"tenant_id\" = ($3::text)::uuid AND \"name\" IS NULL AND \"note\" = ($4::text)::text"
        );
        assert_eq!(built.params.len(), 4);
    }

    #[test]
    fn insert_omits_unset_defaultable_and_identity_columns() {
        let mut descriptor = descriptor(MutationIdentityKind::PrimaryKey, &["id"]);
        descriptor.columns[0].has_default = true;
        descriptor.columns[0].has_identity = true;
        let plan = MutationPlan {
            operations: vec![MutationOp::Insert {
                table: table(),
                values: vec![value("name", Some("Ada"))],
            }],
        };
        let built = only_statement(&descriptor, &plan);
        assert_eq!(
            built.sql,
            "INSERT INTO \"public\".\"users\" (\"name\") VALUES (($1::text)::character varying)"
        );
        assert_eq!(
            built.params,
            vec![DmlParam::Text {
                value: Some("Ada".into())
            }]
        );
    }

    #[test]
    fn insert_explicit_null_is_a_bound_parameter() {
        let descriptor = descriptor(MutationIdentityKind::PrimaryKey, &["id"]);
        let plan = MutationPlan {
            operations: vec![MutationOp::Insert {
                table: table(),
                values: vec![value("note", None)],
            }],
        };
        let built = only_statement(&descriptor, &plan);
        assert_eq!(
            built.sql,
            "INSERT INTO \"public\".\"users\" (\"note\") VALUES (($1::text)::text)"
        );
        assert_eq!(built.params, vec![DmlParam::Text { value: None }]);
    }

    #[test]
    fn duplicate_shaped_insert_excludes_generated_columns() {
        let mut descriptor = descriptor(MutationIdentityKind::PrimaryKey, &["id"]);
        descriptor
            .columns
            .push(column("display_name", "text", ColumnWritability::Generated));
        let plan = MutationPlan {
            operations: vec![MutationOp::Insert {
                table: table(),
                values: vec![value("id", Some("8")), value("name", Some("Ada"))],
            }],
        };
        let built = only_statement(&descriptor, &plan);
        assert_eq!(
            built.sql,
            "INSERT INTO \"public\".\"users\" (\"id\", \"name\") VALUES (($1::text)::integer, ($2::text)::character varying)"
        );
        assert!(!built.sql.contains("display_name"));
    }

    #[test]
    fn identifiers_with_quotes_are_escaped() {
        let descriptor = MutationTableDescriptor {
            schema: "odd\"schema".into(),
            table: "odd\"table".into(),
            columns: vec![column("odd\"column", "text", ColumnWritability::Writable)],
            identity: MutationIdentity {
                kind: MutationIdentityKind::None,
                columns: vec![],
            },
        };
        let plan = MutationPlan {
            operations: vec![MutationOp::Insert {
                table: MutationTable {
                    schema: descriptor.schema.clone(),
                    table: descriptor.table.clone(),
                },
                values: vec![value("odd\"column", Some("safe"))],
            }],
        };
        let built = only_statement(&descriptor, &plan);
        assert_eq!(
            built.sql,
            "INSERT INTO \"odd\"\"schema\".\"odd\"\"table\" (\"odd\"\"column\") VALUES (($1::text)::text)"
        );
    }

    fn assert_invalid(
        descriptor: &MutationTableDescriptor,
        plan: MutationPlan,
        expected: ResultMutationError,
    ) {
        assert_eq!(
            build_mutation_plan(std::slice::from_ref(descriptor), &plan),
            Err(expected)
        );
    }

    #[test]
    fn rejects_each_non_writable_column_kind() {
        for (writability, reason) in [
            (
                ColumnWritability::Generated,
                InvalidPlanReason::GeneratedColumn,
            ),
            (
                ColumnWritability::IdentityAlways,
                InvalidPlanReason::IdentityAlwaysColumn,
            ),
            (
                ColumnWritability::SystemColumn,
                InvalidPlanReason::SystemColumn,
            ),
        ] {
            let mut descriptor = descriptor(MutationIdentityKind::PrimaryKey, &["id"]);
            descriptor.columns[2].writability = writability;
            assert_invalid(
                &descriptor,
                update(
                    vec![value("id", Some("1"))],
                    vec![value("name", Some("old"))],
                    vec![value("name", Some("new"))],
                ),
                invalid(reason),
            );
        }
    }

    #[test]
    fn rejects_unknown_columns() {
        let descriptor = descriptor(MutationIdentityKind::PrimaryKey, &["id"]);
        assert_invalid(
            &descriptor,
            update(
                vec![value("id", Some("1"))],
                vec![value("missing", Some("old"))],
                vec![value("name", Some("new"))],
            ),
            ResultMutationError::UnknownColumn {
                column: "missing".into(),
            },
        );
    }

    #[test]
    fn rejects_empty_set_and_identity() {
        let descriptor = descriptor(MutationIdentityKind::PrimaryKey, &["id"]);
        assert_invalid(
            &descriptor,
            update(vec![value("id", Some("1"))], vec![], vec![]),
            invalid(InvalidPlanReason::EmptySet),
        );
        assert_invalid(
            &descriptor,
            update(
                vec![],
                vec![value("name", Some("old"))],
                vec![value("name", Some("new"))],
            ),
            invalid(InvalidPlanReason::EmptyIdentity),
        );
    }

    #[test]
    fn rejects_null_identity_for_both_keyed_kinds() {
        for kind in [
            MutationIdentityKind::PrimaryKey,
            MutationIdentityKind::UniqueIndex,
        ] {
            let descriptor = descriptor(kind, &["id"]);
            assert_invalid(
                &descriptor,
                update(
                    vec![value("id", None)],
                    vec![value("name", Some("old"))],
                    vec![value("name", Some("new"))],
                ),
                invalid(InvalidPlanReason::NullKeyedIdentity),
            );
        }
    }

    #[test]
    fn rejects_missing_full_row_guard_for_virtual_and_ctid_operations() {
        for (kind, identity_column) in [
            (MutationIdentityKind::VirtualKey, "name"),
            (MutationIdentityKind::CtidFallback, "ctid"),
        ] {
            let mut descriptor = descriptor(kind, &[identity_column]);
            if kind == MutationIdentityKind::CtidFallback {
                descriptor
                    .columns
                    .push(column("ctid", "tid", ColumnWritability::SystemColumn));
            }
            assert_invalid(
                &descriptor,
                update(
                    vec![value(identity_column, Some("key"))],
                    vec![value("name", Some("old"))],
                    vec![value("name", Some("new"))],
                ),
                invalid(InvalidPlanReason::MissingGuard),
            );
        }
    }

    #[test]
    fn rejects_plan_table_mismatch() {
        let descriptor = descriptor(MutationIdentityKind::PrimaryKey, &["id"]);
        let plan = MutationPlan {
            operations: vec![MutationOp::Insert {
                table: MutationTable {
                    schema: "public".into(),
                    table: "other".into(),
                },
                values: vec![value("name", Some("new"))],
            }],
        };
        assert_invalid(&descriptor, plan, invalid(InvalidPlanReason::TableMismatch));
    }

    #[test]
    fn join_snapshot_rejects_insert_and_delete_but_keeps_per_table_updates() {
        let users = descriptor(MutationIdentityKind::PrimaryKey, &["id"]);
        let mut accounts = descriptor(MutationIdentityKind::PrimaryKey, &["id"]);
        accounts.table = "accounts".into();
        let descriptors = [users, accounts];

        let insert = MutationPlan {
            operations: vec![MutationOp::Insert {
                table: table(),
                values: vec![value("name", Some("new"))],
            }],
        };
        assert_eq!(
            build_mutation_plan(&descriptors, &insert),
            Err(invalid(InvalidPlanReason::MultipleOriginTables))
        );

        let delete = MutationPlan {
            operations: vec![MutationOp::Delete {
                table: table(),
                identity: vec![value("id", Some("1"))],
                guards: vec![
                    value("id", Some("1")),
                    value("tenant_id", Some("tenant")),
                    value("name", Some("name")),
                    value("note", Some("note")),
                ],
            }],
        };
        assert_eq!(
            build_mutation_plan(&descriptors, &delete),
            Err(invalid(InvalidPlanReason::MultipleOriginTables))
        );

        let update = update(
            vec![value("id", Some("1"))],
            vec![value("name", Some("old"))],
            vec![value("name", Some("new"))],
        );
        assert!(build_mutation_plan(&descriptors, &update).is_ok());
    }

    #[test]
    fn never_interpolates_user_values_into_sql() {
        let descriptor = descriptor(MutationIdentityKind::PrimaryKey, &["id"]);
        let dangerous = "x'); DROP TABLE users; --";
        let built = only_statement(
            &descriptor,
            &update(
                vec![value("id", Some("1"))],
                vec![value("name", Some("old"))],
                vec![value("name", Some(dangerous))],
            ),
        );
        assert!(!built.sql.contains(dangerous));
        assert_eq!(
            built.params[0],
            DmlParam::Text {
                value: Some(dangerous.into())
            }
        );
    }
}
