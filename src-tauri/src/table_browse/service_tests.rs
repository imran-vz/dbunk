use super::*;
use crate::storage;

#[tokio::test]
#[serial_test::serial]
async fn unsupported_and_missing_connections_fail_without_activity() {
    let (_directory, state) = crate::test_app_state().await;
    let connection = crate::app::test_postgres_connection(
        "unsupported-browse",
        crate::SafeMode::Disabled,
        false,
    );
    storage::upsert_connection(&state.pool, &connection)
        .await
        .unwrap();
    sqlx::query("UPDATE connections SET engine = 'MySQL' WHERE id = ?")
        .bind(connection.id())
        .execute(&state.pool)
        .await
        .unwrap();

    for (id, expected) in [
        (connection.id(), TableBrowseError::UnsupportedEngine),
        ("missing", TableBrowseError::ConnectionLost),
    ] {
        assert_eq!(
            browse(
                &state,
                BrowseTableDataPayload {
                    connection_id: id.into(),
                    tab_id: "tab".into(),
                    request_id: 1,
                    schema: "public".into(),
                    table: "rows".into(),
                    filters: vec![],
                    sort: vec![],
                    page_request: BrowsePageRequest::Offset { page: 0 },
                    page_size: 100,
                    count_policy: BrowseCountPolicy::None,
                    refresh_structure: false,
                },
            )
            .await,
            Err(expected.clone())
        );
        assert_eq!(
            count(
                &state,
                CountTableBrowseRowsPayload {
                    connection_id: id.into(),
                    tab_id: "tab".into(),
                    request_id: 2,
                    schema: "public".into(),
                    table: "rows".into(),
                    filters: vec![],
                },
            )
            .await,
            Err(expected)
        );
    }
    assert!(storage::read_connection_by_id(&state.pool, connection.id())
        .await
        .unwrap()
        .unwrap()
        .last_activity_at()
        .is_none());
}

#[tokio::test]
#[serial_test::serial]
async fn invalid_preferences_preserve_the_last_valid_table_settings() {
    let (_directory, state) = crate::test_app_state().await;
    let connection =
        crate::app::test_postgres_connection("grid-prefs", crate::SafeMode::Disabled, false);
    storage::upsert_connection(&state.pool, &connection)
        .await
        .unwrap();
    let saved = SaveTableGridPrefsPayload {
        connection_id: connection.id().into(),
        schema: "public".into(),
        table: "rows".into(),
        prefs: TableGridPrefs(serde_json::json!({
            "version": 1,
            "columnOrder": ["tenant_id", "名"],
            "filterHistory": (0..25).collect::<Vec<_>>(),
            "sortHistory": (0..25).rev().collect::<Vec<_>>()
        })),
    };
    save_grid_prefs(&state, saved.clone()).await.unwrap();
    assert!(save_grid_prefs(
        &state,
        SaveTableGridPrefsPayload {
            prefs: TableGridPrefs(serde_json::json!({"version": 0})),
            ..saved
        },
    )
    .await
    .is_err());
    let load = LoadTableGridPrefsPayload {
        connection_id: connection.id().into(),
        schema: "public".into(),
        table: "rows".into(),
    };
    assert_eq!(
        load_grid_prefs(&state, load.clone()).await.unwrap(),
        Some(TableGridPrefs(serde_json::json!({
            "version": 1,
            "columnOrder": ["tenant_id", "名"],
            "filterHistory": (0..20).collect::<Vec<_>>(),
            "sortHistory": (5..25).rev().collect::<Vec<_>>()
        })))
    );
    assert_eq!(
        load_grid_prefs(
            &state,
            LoadTableGridPrefsPayload {
                schema: "another_schema".into(),
                ..load
            },
        )
        .await
        .unwrap(),
        None
    );
}
