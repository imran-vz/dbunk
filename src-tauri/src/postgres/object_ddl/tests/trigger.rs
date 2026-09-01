//! Generator unit tests: trigger operations.

use super::*;

#[test]
fn create_trigger_renders_events_columns_when_and_arguments() {
    let op = PgObjectOp::CreateTrigger(CreateTriggerOp {
        schema: "lifecycle".into(),
        table: "orders".into(),
        name: "orders_touch".into(),
        timing: PgTriggerTiming::Before,
        events: vec![
            PgTriggerEvent::Insert,
            PgTriggerEvent::Update {
                columns: vec!["status".into(), "amount".into()],
            },
        ],
        for_each: PgTriggerLevel::Row,
        when: Some("OLD.status IS DISTINCT FROM NEW.status".into()),
        function_schema: "lifecycle".into(),
        function_name: "touch_orders".into(),
        arguments: vec!["it's".into(), "two".into()],
        or_replace: true,
    });
    let preview = generate_object_ddl(&[op]).expect("trigger");
    assert_eq!(
        preview.statements[0].sql,
        "CREATE OR REPLACE TRIGGER \"orders_touch\" BEFORE INSERT OR UPDATE OF \"status\", \"amount\" \
         ON \"lifecycle\".\"orders\" FOR EACH ROW WHEN (OLD.status IS DISTINCT FROM NEW.status) \
         EXECUTE FUNCTION \"lifecycle\".\"touch_orders\"(E'it''s', E'two');"
    );
    assert_eq!(
        preview.statements[0].summary,
        "Create or replace trigger orders_touch on lifecycle.orders"
    );
    assert!(preview.statements[0].destructive);

    let statement_level = trigger_op(
        vec![
            PgTriggerEvent::Truncate,
            PgTriggerEvent::Update { columns: vec![] },
        ],
        PgTriggerTiming::After,
        PgTriggerLevel::Statement,
        None,
    );
    let preview = generate_object_ddl(&[statement_level]).expect("statement trigger");
    assert_eq!(
        preview.statements[0].sql,
        "CREATE TRIGGER \"orders_touch\" AFTER TRUNCATE OR UPDATE ON \"lifecycle\".\"orders\" \
         FOR EACH STATEMENT EXECUTE FUNCTION \"lifecycle\".\"touch_orders\"();"
    );
    assert!(!preview.statements[0].destructive);
}

#[test]
fn create_trigger_validation_rules() {
    let reason = sole_invalid;
    assert!(reason(trigger_op(
        vec![],
        PgTriggerTiming::Before,
        PgTriggerLevel::Row,
        None
    ))
    .contains("events cannot be empty"));
    assert!(reason(trigger_op(
        vec![PgTriggerEvent::Delete, PgTriggerEvent::Delete],
        PgTriggerTiming::Before,
        PgTriggerLevel::Row,
        None
    ))
    .contains("DELETE is listed more than once"));
    assert!(reason(trigger_op(
        vec![PgTriggerEvent::Truncate],
        PgTriggerTiming::Before,
        PgTriggerLevel::Row,
        None
    ))
    .contains("TRUNCATE triggers must be FOR EACH STATEMENT"));
    assert!(reason(trigger_op(
        vec![PgTriggerEvent::Insert],
        PgTriggerTiming::InsteadOf,
        PgTriggerLevel::Statement,
        None
    ))
    .contains("INSTEAD OF triggers must be FOR EACH ROW"));
    assert!(reason(trigger_op(
        vec![PgTriggerEvent::Insert],
        PgTriggerTiming::InsteadOf,
        PgTriggerLevel::Row,
        Some("true")
    ))
    .contains("cannot have a WHEN condition"));
    assert!(reason(trigger_op(
        vec![PgTriggerEvent::Insert],
        PgTriggerTiming::After,
        PgTriggerLevel::Row,
        Some("true) EXECUTE FUNCTION evil(")
    ))
    .contains("escapes its typed SQL context"));
    assert!(generate_object_ddl(&[trigger_op(
        vec![PgTriggerEvent::Insert],
        PgTriggerTiming::InsteadOf,
        PgTriggerLevel::Row,
        None
    )])
    .is_ok());
}

#[test]
fn trigger_lifecycle_ops_follow_the_destructiveness_rule() {
    let drop = PgObjectOp::DropTrigger(DropTriggerOp {
        schema: "lifecycle".into(),
        table: "orders".into(),
        name: "orders_touch".into(),
        cascade: false,
    });
    let dropped = statement(drop);
    assert_eq!(
        dropped.sql,
        "DROP TRIGGER \"orders_touch\" ON \"lifecycle\".\"orders\" RESTRICT;"
    );
    assert!(dropped.destructive);

    let mode = |mode: PgTriggerMode| {
        statement(PgObjectOp::SetTriggerEnabled(SetTriggerEnabledOp {
            schema: "lifecycle".into(),
            table: "orders".into(),
            name: "orders_touch".into(),
            mode,
        }))
    };
    let disabled = mode(PgTriggerMode::Disable);
    assert_eq!(
        disabled.sql,
        "ALTER TABLE \"lifecycle\".\"orders\" DISABLE TRIGGER \"orders_touch\";"
    );
    assert!(disabled.destructive);
    assert_eq!(
        disabled.summary,
        "Disable trigger orders_touch on lifecycle.orders"
    );
    assert!(!mode(PgTriggerMode::Enable).destructive);
    assert!(mode(PgTriggerMode::EnableAlways)
        .sql
        .contains("ENABLE ALWAYS TRIGGER"));
    assert!(mode(PgTriggerMode::EnableReplica)
        .sql
        .contains("ENABLE REPLICA TRIGGER"));
}
