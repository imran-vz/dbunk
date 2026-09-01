//! Live: trigger create, enable states, and drop.

use super::*;

#[tokio::test]
#[serial_test::serial]
#[ignore = "requires pnpm db:postgres"]
async fn trigger_live_create_disable_and_drop() {
    let (_directory, state, connection) = live_state("live-trigger").await;
    let reset = "DROP TRIGGER IF EXISTS live_touch ON lifecycle.orders;\n\
                 DROP FUNCTION IF EXISTS lifecycle.live_touch_fn();";
    with_reset(&connection, reset, async {
        apply_live(
            &state,
            connection.id(),
            vec![
                PgObjectOp::CreateFunction(CreateFunctionOp {
                    schema: "lifecycle".into(),
                    name: "live_touch_fn".into(),
                    or_replace: false,
                    arguments: String::new(),
                    returns: "trigger".into(),
                    language: "plpgsql".into(),
                    body: "BEGIN RETURN NEW; END;".into(),
                    volatility: PgVolatility::Volatile,
                    strict: false,
                    security_definer: false,
                    parallel: None,
                }),
                PgObjectOp::CreateTrigger(CreateTriggerOp {
                    schema: "lifecycle".into(),
                    table: "orders".into(),
                    name: "live_touch".into(),
                    timing: PgTriggerTiming::Before,
                    events: vec![PgTriggerEvent::Update {
                        columns: vec!["amount".into()],
                    }],
                    for_each: PgTriggerLevel::Row,
                    when: Some("OLD.amount IS DISTINCT FROM NEW.amount".into()),
                    function_schema: "lifecycle".into(),
                    function_name: "live_touch_fn".into(),
                    arguments: vec!["one".into()],
                    or_replace: false,
                }),
            ],
        )
        .await
        .expect("create trigger function and trigger");

        let set_mode = |mode| {
            PgObjectOp::SetTriggerEnabled(SetTriggerEnabledOp {
                schema: "lifecycle".into(),
                table: "orders".into(),
                name: "live_touch".into(),
                mode,
            })
        };
        apply_live(
            &state,
            connection.id(),
            vec![set_mode(PgTriggerMode::Disable)],
        )
        .await
        .expect("disable trigger");
        let structure = fetch_table_structure(&connection, "lifecycle", "orders")
            .await
            .expect("orders structure");
        assert!(structure.capabilities.triggers);
        let trigger = structure
            .triggers
            .iter()
            .find(|trigger| trigger.name == "live_touch")
            .expect("live trigger listed");
        assert_eq!(trigger.enabled, TriggerEnabledState::Disabled);
        assert_eq!(trigger.timing, "BEFORE");
        assert_eq!(trigger.events, vec!["UPDATE".to_string()]);
        assert_eq!(trigger.update_columns, vec!["amount".to_string()]);
        assert_eq!(trigger.level, "ROW");
        assert_eq!(trigger.function_schema, "lifecycle");
        assert_eq!(trigger.function_name, "live_touch_fn");
        assert!(
            trigger.definition.contains("WHEN"),
            "{}",
            trigger.definition
        );
        // The fixture trigger stays untouched and enabled.
        assert!(structure.triggers.iter().any(|trigger| {
            trigger.name == "orders_touch" && trigger.enabled == TriggerEnabledState::Origin
        }));

        apply_live(
            &state,
            connection.id(),
            vec![set_mode(PgTriggerMode::EnableAlways)],
        )
        .await
        .expect("enable always");
        let structure = fetch_table_structure(&connection, "lifecycle", "orders")
            .await
            .expect("orders structure");
        assert!(structure.triggers.iter().any(|trigger| {
            trigger.name == "live_touch" && trigger.enabled == TriggerEnabledState::Always
        }));

        apply_live(
            &state,
            connection.id(),
            vec![PgObjectOp::DropTrigger(DropTriggerOp {
                schema: "lifecycle".into(),
                table: "orders".into(),
                name: "live_touch".into(),
                cascade: false,
            })],
        )
        .await
        .expect("drop trigger");
        let structure = fetch_table_structure(&connection, "lifecycle", "orders")
            .await
            .expect("orders structure");
        assert!(!structure
            .triggers
            .iter()
            .any(|trigger| trigger.name == "live_touch"));
    })
    .await;
}
