//! Unit tests for the DDL generator. Live tests live in
//! `commands::pg_objects_live_tests`.

mod column;
mod enum_type;
mod fragment;
mod general;
mod index;
mod policy;
mod privilege;
mod routine;
mod table;
mod trigger;

use super::super::objects::{PgObjectKind, PgObjectRef};
use super::routine::dollar_tag;
use super::*;

pub(super) fn add_column(data_type: &str, default: Option<PgDefaultValue>) -> PgObjectOp {
    PgObjectOp::AddColumn(AddColumnOp {
        schema: "s".into(),
        table: "t".into(),
        column: NewColumnSpec {
            name: "c".into(),
            data_type: data_type.into(),
            nullable: true,
            default,
            identity: None,
        },
    })
}

pub(super) fn add_check(expression: &str) -> PgObjectOp {
    PgObjectOp::AddCheck(AddCheckOp {
        schema: "s".into(),
        table: "t".into(),
        name: None,
        expression: expression.into(),
        not_valid: false,
    })
}

pub(super) fn index_with(expressions: &[&str]) -> PgObjectOp {
    PgObjectOp::CreateIndex(CreateIndexOp {
        schema: "s".into(),
        table: "orders".into(),
        name: Some("orders_idx".into()),
        unique: false,
        method: "btree".into(),
        columns: expressions
            .iter()
            .map(|expression| PgIndexColumn {
                expression: (*expression).into(),
                descending: true,
            })
            .collect(),
        include: Vec::new(),
        where_predicate: None,
        concurrently: false,
    })
}

pub(super) fn table_ref(kind: PgObjectKind, name: &str) -> PgObjectRef {
    PgObjectRef {
        kind,
        schema: Some("lifecycle".into()),
        name: name.into(),
        identity_args: None,
    }
}

pub(super) fn statement(op: PgObjectOp) -> PlannedStatement {
    generate_object_ddl(&[op])
        .expect("valid DDL")
        .statements
        .into_iter()
        .next()
        .expect("one statement")
}

pub(super) fn column(name: &str, data_type: &str) -> NewColumnSpec {
    NewColumnSpec {
        name: name.into(),
        data_type: data_type.into(),
        nullable: true,
        default: None,
        identity: None,
    }
}

pub(super) fn designer_table(columns: Vec<NewColumnSpec>) -> PgObjectOp {
    designer_table_with(columns, None, Vec::new(), Vec::new())
}

pub(super) fn designer_table_with(
    columns: Vec<NewColumnSpec>,
    primary_key: Option<PgKeySpec>,
    checks: Vec<PgCheckSpec>,
    foreign_keys: Vec<PgForeignKeySpec>,
) -> PgObjectOp {
    PgObjectOp::CreateTable(CreateTableOp {
        schema: "lifecycle".into(),
        name: "designer_demo".into(),
        columns,
        primary_key,
        uniques: Vec::new(),
        checks,
        foreign_keys,
        unlogged: false,
        if_not_exists: false,
    })
}

pub(super) fn function_with(arguments: &str, returns: &str, body: &str) -> PgObjectOp {
    PgObjectOp::CreateFunction(CreateFunctionOp {
        schema: "lifecycle".into(),
        name: "order_total".into(),
        or_replace: false,
        arguments: arguments.into(),
        returns: returns.into(),
        language: "plpgsql".into(),
        body: body.into(),
        volatility: PgVolatility::Stable,
        strict: false,
        security_definer: false,
        parallel: None,
    })
}

/// The reason for a single invalid operation, pinning that the refusal names
/// operation index zero.
pub(super) fn sole_invalid(op: PgObjectOp) -> String {
    let (op_index, reason) = invalid_reason(&[op]);
    assert_eq!(op_index, 0, "{reason}");
    reason
}

pub(super) fn invalid_reason(ops: &[PgObjectOp]) -> (usize, String) {
    match generate_object_ddl(ops).expect_err("invalid op") {
        PgObjectError::InvalidOp { op_index, reason } => (op_index, reason),
        other => panic!("expected invalidOp, got {other:?}"),
    }
}

pub(super) fn trigger_op(
    events: Vec<PgTriggerEvent>,
    timing: PgTriggerTiming,
    for_each: PgTriggerLevel,
    when: Option<&str>,
) -> PgObjectOp {
    PgObjectOp::CreateTrigger(CreateTriggerOp {
        schema: "lifecycle".into(),
        table: "orders".into(),
        name: "orders_touch".into(),
        timing,
        events,
        for_each,
        when: when.map(String::from),
        function_schema: "lifecycle".into(),
        function_name: "touch_orders".into(),
        arguments: Vec::new(),
        or_replace: false,
    })
}

pub(super) fn policy_op(
    command: PgPolicyCommand,
    using: Option<&str>,
    with_check: Option<&str>,
) -> PgObjectOp {
    PgObjectOp::CreatePolicy(CreatePolicyOp {
        schema: "lifecycle".into(),
        table: "tenant_rows".into(),
        name: "tenant_isolation".into(),
        permissive: true,
        command,
        roles: vec![PgGrantee::Public],
        using: using.map(String::from),
        with_check: with_check.map(String::from),
    })
}

pub(super) fn grant_op(
    target: PgObjectRef,
    privileges: Vec<PgPrivilege>,
    all_privileges: bool,
) -> PgObjectOp {
    PgObjectOp::GrantPrivileges(GrantPrivilegesOp {
        target,
        privileges,
        all_privileges,
        grantee: PgGrantee::Role {
            name: "lifecycle_reader".into(),
        },
        with_grant_option: false,
    })
}
