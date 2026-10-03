use super::*;

fn strict_connection(connection_id: &str) -> crate::StoredConnection {
    crate::StoredConnection::PostgreSQL(crate::PgStoredConnection {
        organization: Default::default(),
        id: connection_id.into(),
        name: "Strict mutation".into(),
        database: "dbunk_demo".into(),
        host: "127.0.0.1".into(),
        port: 15432,
        user: "dbunk".into(),
        password: "dbunk".into(),
        role: "read/write".into(),
        environment: crate::Environment::Production,
        safe_mode: crate::SafeMode::Inherit,
        read_only: false,
        last_activity_at: None,
        ssl: true,
        tls_options: None,
        driver_options: None,
        ssh_tunnel: crate::SshTunnelConfig::default(),
    })
}

#[test]
fn virtual_key_storage_errors_preserve_the_command_contract() {
    assert_eq!(
        virtual_key_storage_error(storage::VirtualKeyStorageError::UnsupportedEngine),
        ResultMutationError::UnsupportedEngine
    );
    assert_eq!(
        virtual_key_storage_error(storage::VirtualKeyStorageError::ConnectionNotFound),
        ResultMutationError::ConnectionLost
    );
    assert_eq!(
        virtual_key_storage_error(storage::VirtualKeyStorageError::InvalidInput(
            storage::VirtualKeyValidationError::EmptyIdentity,
        )),
        ResultMutationError::InvalidPlan {
            reason: InvalidPlanReason::EmptyIdentity,
        }
    );
    assert_eq!(
        virtual_key_storage_error(storage::VirtualKeyStorageError::InvalidInput(
            storage::VirtualKeyValidationError::DuplicateColumn,
        )),
        ResultMutationError::InvalidPlan {
            reason: InvalidPlanReason::DuplicateColumn,
        }
    );
    assert!(matches!(
        virtual_key_storage_error(storage::VirtualKeyStorageError::CorruptDocument(
            "stale JSON".to_string(),
        )),
        ResultMutationError::Database { .. }
    ));
}

#[tokio::test]
#[serial_test::serial]
async fn virtual_key_changes_invalidate_analysis_only_after_storage_success() {
    use crate::result_mutation::{new_executor, AnalysisSnapshot};

    let (_directory, state) = crate::test_app_state().await;
    let connection = strict_connection("virtual-key-service");
    storage::upsert_connection(&state.pool, &connection)
        .await
        .unwrap();
    // A cached analysis without a connection exercises invalidation without
    // contacting PostgreSQL or depending on an external fixture.
    let executor = new_executor(ResolvedPostgresConnectSpec::from_connection(&connection).unwrap());
    state
        .result_mutations
        .inner
        .lock()
        .await
        .executors
        .insert(connection.id().into(), executor.clone());
    let snapshot = || AnalysisSnapshot {
        tab_id: "tab".into(),
        descriptors: vec![],
    };
    let initial = executor.state.lock().await.snapshots.insert(snapshot());
    let save = SaveVirtualKeyPayload {
        connection_id: connection.id().into(),
        schema: "public".into(),
        table: "rows".into(),
        columns: vec!["tenant_id".into(), "email".into()],
    };
    assert_eq!(
        save_virtual_key(
            &state,
            SaveVirtualKeyPayload {
                columns: vec!["id".into(), "id".into()],
                ..save.clone()
            },
        )
        .await,
        Err(ResultMutationError::InvalidPlan {
            reason: InvalidPlanReason::DuplicateColumn,
        })
    );
    assert!(executor
        .state
        .lock()
        .await
        .snapshots
        .values
        .contains_key(&initial));
    save_virtual_key(&state, save.clone()).await.unwrap();
    let load = LoadVirtualKeyPayload {
        connection_id: save.connection_id.clone(),
        schema: save.schema.clone(),
        table: save.table.clone(),
    };
    assert_eq!(
        load_virtual_key(&state, load.clone()).await.unwrap(),
        Some(VirtualKey {
            version: 1,
            columns: save.columns,
        })
    );
    assert!(executor.state.lock().await.snapshots.values.is_empty());

    let replacement = executor.state.lock().await.snapshots.insert(snapshot());
    sqlx::query(
        "CREATE TRIGGER deny_key_delete BEFORE DELETE ON virtual_keys \
         BEGIN SELECT RAISE(ABORT, 'private storage failure'); END",
    )
    .execute(&state.pool)
    .await
    .unwrap();
    let clear = ClearVirtualKeyPayload {
        connection_id: load.connection_id.clone(),
        schema: load.schema.clone(),
        table: load.table.clone(),
    };
    let error = clear_virtual_key(&state, clear.clone()).await.unwrap_err();
    assert!(matches!(error, ResultMutationError::Database { .. }));
    assert!(!serde_json::to_string(&error)
        .unwrap()
        .contains("private storage failure"));
    assert!(executor
        .state
        .lock()
        .await
        .snapshots
        .values
        .contains_key(&replacement));
    assert!(load_virtual_key(&state, load.clone())
        .await
        .unwrap()
        .is_some());

    sqlx::query("DROP TRIGGER deny_key_delete")
        .execute(&state.pool)
        .await
        .unwrap();
    clear_virtual_key(&state, clear).await.unwrap();
    assert_eq!(load_virtual_key(&state, load).await.unwrap(), None);
    assert!(executor.state.lock().await.snapshots.values.is_empty());
    close_connection(
        &state,
        CloseResultMutationPayload {
            connection_id: connection.id().into(),
        },
    )
    .await
    .unwrap();
}

#[tokio::test]
#[serial_test::serial]
async fn apply_service_refusal_and_failure_never_audit() {
    let (_directory, state) = crate::test_app_state().await;
    let connection_id = "strict-apply-command";
    crate::connections::save(&state, strict_connection(connection_id))
        .await
        .expect("save strict connection");

    let result = apply(
        &state,
        ApplyResultMutationsPayload {
            connection_id: connection_id.into(),
            tab_id: "tab".into(),
            request_id: 1,
            confirmed: false,
            analysis_id: 1,
            plan: MutationPlan {
                operations: vec![MutationOp::Insert {
                    table: MutationTable {
                        schema: "public".into(),
                        table: "rows".into(),
                    },
                    values: vec![MutationValue {
                        column: "body".into(),
                        value: Some("value".into()),
                    }],
                }],
            },
        },
    )
    .await;
    assert!(matches!(
        result,
        Err(ResultMutationError::PolicyNeedsConfirmation { .. })
    ));

    let failed_after_admission = apply(
        &state,
        ApplyResultMutationsPayload {
            connection_id: connection_id.into(),
            tab_id: "tab".into(),
            request_id: 2,
            confirmed: true,
            analysis_id: 1,
            plan: MutationPlan {
                operations: vec![MutationOp::Insert {
                    table: MutationTable {
                        schema: "public".into(),
                        table: "rows".into(),
                    },
                    values: vec![MutationValue {
                        column: "body".into(),
                        value: Some("value".into()),
                    }],
                }],
            },
        },
    )
    .await;
    assert_eq!(
        failed_after_admission,
        Err(ResultMutationError::AnalysisExpired)
    );
    sqlx::query("UPDATE connections SET read_only = 1 WHERE id = ?")
        .bind(connection_id)
        .execute(&state.pool)
        .await
        .unwrap();
    assert!(matches!(
        apply(
            &state,
            ApplyResultMutationsPayload {
                connection_id: connection_id.into(),
                tab_id: "tab".into(),
                request_id: 3,
                confirmed: true,
                analysis_id: 1,
                plan: MutationPlan {
                    operations: vec![MutationOp::Insert {
                        table: MutationTable {
                            schema: "public".into(),
                            table: "rows".into(),
                        },
                        values: vec![MutationValue {
                            column: "body".into(),
                            value: Some("value".into()),
                        }],
                    }],
                },
            },
        )
        .await,
        Err(ResultMutationError::PolicyBlocked { .. })
    ));
    assert!(storage::read_safety_overrides(&state.pool, connection_id)
        .await
        .expect("read audits")
        .is_empty());
    assert!(storage::read_connection_by_id(&state.pool, connection_id)
        .await
        .expect("read connection")
        .expect("stored connection")
        .last_activity_at()
        .is_none());
}
