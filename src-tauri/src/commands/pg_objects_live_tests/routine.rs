//! Live: routine create, replace, and header attributes.

use super::*;

#[tokio::test]
#[serial_test::serial]
#[ignore = "requires pnpm db:postgres"]
async fn routine_live_replaces_source_and_reports_header_attributes() {
    let (_directory, state, connection) = live_state("live-routine").await;
    let reset = "DROP FUNCTION IF EXISTS lifecycle.live_total(integer);";
    with_reset(&connection, reset, async {
        let create = |or_replace: bool, body: &str, volatility, strict| {
            PgObjectOp::CreateFunction(CreateFunctionOp {
                schema: "lifecycle".into(),
                name: "live_total".into(),
                or_replace,
                arguments: "order_id integer".into(),
                returns: "numeric".into(),
                language: "plpgsql".into(),
                body: body.into(),
                volatility,
                strict,
                security_definer: false,
                parallel: Some(PgParallelSafety::Safe),
            })
        };
        apply_live(
            &state,
            connection.id(),
            vec![create(
                false,
                "BEGIN\n  RETURN 1;\nEND;",
                PgVolatility::Stable,
                false,
            )],
        )
        .await
        .expect("create function");
        apply_live(
            &state,
            connection.id(),
            vec![create(
                true,
                "BEGIN\n  RETURN 42; -- $dbunk$ inside\nEND;",
                PgVolatility::Immutable,
                true,
            )],
        )
        .await
        .expect("replace function");

        // Identity arguments carry the parameter name, exactly as
        // pg_get_function_identity_arguments reports it.
        let routine = describe_pg_object(
            &connection,
            lifecycle_ref(
                PgObjectKind::Function,
                "live_total",
                Some("order_id integer"),
            ),
        )
        .await
        .expect("describe replaced function");
        match routine.facts {
            PgObjectFacts::Routine {
                body,
                strict,
                parallel,
                volatility,
                security_definer,
                ..
            } => {
                assert!(body
                    .as_deref()
                    .is_some_and(|body| body.contains("RETURN 42")));
                assert!(strict);
                assert!(!security_definer);
                assert_eq!(parallel.as_deref(), Some("safe"));
                assert_eq!(volatility.as_deref(), Some("immutable"));
            }
            other => panic!("expected routine facts, got {other:?}"),
        }

        // A return-type change is refused by PostgreSQL under OR REPLACE and
        // surfaces as the typed database error with its SQLSTATE.
        let mut changed = create(true, "BEGIN RETURN 1; END;", PgVolatility::Stable, false);
        if let PgObjectOp::CreateFunction(CreateFunctionOp { returns, .. }) = &mut changed {
            *returns = "integer".into();
        }
        let error = apply_live(&state, connection.id(), vec![changed])
            .await
            .expect_err("return type change");
        assert!(matches!(
            error,
            PgObjectError::Database { ref code, applied_statements: 0, .. }
                if code.as_deref() == Some("42P13")
        ));
    })
    .await;
}
