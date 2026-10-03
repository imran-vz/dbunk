use super::*;
use crate::backend::{profile, schema_map::SchemaMapIdentity};
fn relation(schema: &str, table: &str) -> SchemaMapPreferenceScope {
    SchemaMapPreferenceScope::Relation {
        schema: schema.into(),
        table: table.into(),
    }
}
fn value() -> SchemaMapPreferences {
    let mut value = SchemaMapPreferences::for_database(42);
    value.positions.push(SavedPosition {
        identity: SchemaMapIdentity {
            database_oid: 42,
            relation_oid: 7,
        },
        position: MapPoint { x: 12.5, y: -60.0 },
    });
    value
}
#[test]
fn bounded_exact_positions_and_defaults() {
    let initial = value();
    assert_eq!(
        initial.prefs,
        MapPrefs {
            routing: MapRouting::Curve,
            attributes: MapAttributes::All,
            show_types: true,
            show_nulls: false,
            show_comments: false
        }
    );
    assert!(initial.checked_heap_bytes().is_some());
    for coordinate in [f64::NAN, f64::INFINITY, MAX_MAP_POSITION_COORDINATE + 1.0] {
        let mut bad = initial.clone();
        bad.positions[0].position.x = coordinate;
        assert!(bad.checked_heap_bytes().is_none());
    }
    let mut duplicate = initial.clone();
    duplicate.positions.push(duplicate.positions[0]);
    assert!(duplicate.checked_heap_bytes().is_none());
    let mut foreign = initial.clone();
    foreign.positions[0].identity.database_oid = 43;
    assert!(foreign.checked_heap_bytes().is_none());
    let mut capacity = initial;
    capacity.positions.reserve(MAX_MAP_PREFERENCES_HEAP_BYTES);
    assert!(capacity.checked_heap_bytes().is_none());
    let a = storage::key("c", &relation("a.b", "c")).unwrap();
    let b = storage::key("c", &relation("a", "b.c")).unwrap();
    assert_ne!(a, b);
    assert_ne!(
        storage::key("c", &SchemaMapPreferenceScope::Database).unwrap(),
        storage::key(
            "c",
            &SchemaMapPreferenceScope::Schema {
                name: "__all__".into()
            }
        )
        .unwrap()
    );
    assert!(relation(" ", "雪").checked_heap_bytes().is_some());
}
#[test]
fn decoded_position_count_and_unknown_identity_fields_refuse() {
    let mut good = value();
    good.positions = (1..=MAX_MAP_POSITIONS as u32)
        .map(|oid| SavedPosition {
            identity: SchemaMapIdentity {
                database_oid: 42,
                relation_oid: oid,
            },
            position: MapPoint { x: 0.0, y: 0.0 },
        })
        .collect();
    let encoded = serde_json::to_string(&good).unwrap();
    let decoded: SchemaMapPreferences = serde_json::from_str(&encoded).unwrap();
    assert!(decoded.checked_heap_bytes().is_some());
    good.positions.push(good.positions[0]);
    assert!(
        serde_json::from_str::<SchemaMapPreferences>(&serde_json::to_string(&good).unwrap())
            .is_err()
    );
    let mut raw = serde_json::to_value(value()).unwrap();
    raw["positions"][0]["identity"]["future"] = serde_json::json!(true);
    assert!(serde_json::from_value::<SchemaMapPreferences>(raw).is_err());
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn committed_ack_stale_save_reset_and_absence_aba() {
    let directory = profile::directory();
    let backend = Backend::open_fixture(&directory.path().canonicalize().unwrap())
        .await
        .unwrap();
    let doc = backend
        .open_data_document("map-prefs", "tab", &backend.fixture().id)
        .await
        .unwrap();
    let scope = relation("a.b", "c");
    let absent = backend
        .load_schema_map_preferences(&doc, scope.clone())
        .await
        .unwrap();
    assert!(absent.value.is_none());
    let saved = backend
        .save_schema_map_preferences(&doc, scope.clone(), absent.revision.clone(), value())
        .await
        .unwrap();
    assert_eq!(saved.value, Some(value()));
    assert_eq!(
        backend
            .load_schema_map_preferences(&doc, scope.clone())
            .await
            .unwrap(),
        saved
    );
    assert_eq!(
        backend
            .save_schema_map_preferences(&doc, scope.clone(), absent.revision.clone(), value())
            .await
            .unwrap_err(),
        SchemaMapPreferencesError::StaleRevision
    );
    assert_eq!(
        backend
            .reset_schema_map_preferences(&doc, scope.clone(), absent.revision.clone())
            .await
            .unwrap_err(),
        SchemaMapPreferencesError::StaleRevision
    );
    let reset = backend
        .reset_schema_map_preferences(&doc, scope.clone(), saved.revision.clone())
        .await
        .unwrap();
    assert!(reset.value.is_none());
    assert_ne!(reset.revision, absent.revision);
    assert_eq!(
        backend
            .save_schema_map_preferences(&doc, scope.clone(), absent.revision, value())
            .await
            .unwrap_err(),
        SchemaMapPreferencesError::StaleRevision
    );
    assert_eq!(
        backend
            .load_schema_map_preferences(&doc, scope.clone())
            .await
            .unwrap(),
        reset
    );
    let mut changed = value();
    changed.prefs.routing = MapRouting::Step;
    let later = backend
        .save_schema_map_preferences(&doc, scope.clone(), reset.revision, changed.clone())
        .await
        .unwrap();
    assert_eq!(later.value, Some(changed));
    assert_eq!(
        backend
            .save_schema_map_preferences(&doc, scope, saved.revision, value())
            .await
            .unwrap_err(),
        SchemaMapPreferencesError::StaleRevision
    );
    backend.shutdown().await.unwrap();
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn scopes_documents_and_profiles_cannot_retarget_revision() {
    let directory = profile::directory();
    let other_directory = profile::directory();
    let backend = Backend::open_fixture(&directory.path().canonicalize().unwrap())
        .await
        .unwrap();
    let other = Backend::open_fixture(&other_directory.path().canonicalize().unwrap())
        .await
        .unwrap();
    let doc = backend
        .open_data_document("map-prefs", "tab", &backend.fixture().id)
        .await
        .unwrap();
    let a = relation("a.b", "c");
    let b = relation("a", "b.c");
    let loaded = backend
        .load_schema_map_preferences(&doc, a.clone())
        .await
        .unwrap();
    assert_eq!(
        backend
            .save_schema_map_preferences(&doc, b.clone(), loaded.revision.clone(), value())
            .await
            .unwrap_err(),
        SchemaMapPreferencesError::StaleRevision
    );
    let saved = backend
        .save_schema_map_preferences(&doc, a.clone(), loaded.revision, value())
        .await
        .unwrap();
    let b_absent = backend
        .load_schema_map_preferences(&doc, b.clone())
        .await
        .unwrap();
    assert!(b_absent.value.is_none());
    let b_saved = backend
        .save_schema_map_preferences(&doc, b.clone(), b_absent.revision, value())
        .await
        .unwrap();
    backend
        .reset_schema_map_preferences(&doc, a.clone(), saved.revision.clone())
        .await
        .unwrap();
    assert_eq!(
        backend.load_schema_map_preferences(&doc, b).await.unwrap(),
        b_saved
    );
    assert_eq!(
        other
            .load_schema_map_preferences(&doc, a.clone())
            .await
            .unwrap_err(),
        SchemaMapPreferencesError::Document
    );
    backend.close_data_document(&doc).await.unwrap();
    assert_eq!(
        backend
            .load_schema_map_preferences(&doc, a.clone())
            .await
            .unwrap_err(),
        SchemaMapPreferencesError::Document
    );
    assert_eq!(
        backend
            .reset_schema_map_preferences(&doc, a, saved.revision)
            .await
            .unwrap_err(),
        SchemaMapPreferencesError::Document
    );
    other.shutdown().await.unwrap();
    backend.shutdown().await.unwrap();
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn future_corrupt_and_oversized_bytes_survive_load_save_and_reset() {
    let directory = profile::directory();
    let backend = Backend::open_fixture(&directory.path().canonicalize().unwrap())
        .await
        .unwrap();
    let connection = backend.fixture().id;
    let doc = backend
        .open_data_document("map-prefs", "tab", &connection)
        .await
        .unwrap();
    let scope = SchemaMapPreferenceScope::Schema {
        name: "public".into(),
    };
    let observed = backend
        .load_schema_map_preferences(&doc, scope.clone())
        .await
        .unwrap();
    let key = storage::key(&connection, &scope).unwrap();
    for (encoded, error) in [
        (
            "{\"version\":2,\"future\":true}".to_owned(),
            SchemaMapPreferencesError::UnsupportedVersion,
        ),
        ("{broken".into(), SchemaMapPreferencesError::Corrupt),
        (
            format!(
                "{{\"version\":1,\"payload\":\"{}\"}}",
                "x".repeat(MAX_MAP_PREFERENCES_BYTES)
            ),
            SchemaMapPreferencesError::TooLarge,
        ),
    ] {
        sqlx::query("INSERT INTO ui_state(key,value,updated_at) VALUES(?,?,'untouched') ON CONFLICT(key) DO UPDATE SET value=excluded.value")
            .bind(&key).bind(&encoded).execute(&backend.0.state.pool).await.unwrap();
        assert_eq!(
            backend
                .load_schema_map_preferences(&doc, scope.clone())
                .await
                .unwrap_err(),
            error
        );
        assert_eq!(
            backend
                .save_schema_map_preferences(
                    &doc,
                    scope.clone(),
                    observed.revision.clone(),
                    value()
                )
                .await
                .unwrap_err(),
            error
        );
        assert_eq!(
            backend
                .reset_schema_map_preferences(&doc, scope.clone(), observed.revision.clone())
                .await
                .unwrap_err(),
            error
        );
        let stored: (String, String) =
            sqlx::query_as("SELECT value,updated_at FROM ui_state WHERE key=?")
                .bind(&key)
                .fetch_one(&backend.0.state.pool)
                .await
                .unwrap();
        assert_eq!(stored, (encoded, "untouched".into()));
    }
    backend.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn concurrent_writers_publish_only_one_exact_observed_revision() {
    let directory = profile::directory();
    let backend = Backend::open_fixture(&directory.path().canonicalize().unwrap())
        .await
        .unwrap();
    let connection = backend.fixture().id;
    let first = backend
        .open_data_document("map-prefs", "first", &connection)
        .await
        .unwrap();
    let second = backend
        .open_data_document("map-prefs", "second", &connection)
        .await
        .unwrap();
    let scope = SchemaMapPreferenceScope::Database;
    let observed = backend
        .load_schema_map_preferences(&first, scope.clone())
        .await
        .unwrap();
    let mut other = value();
    other.prefs.show_comments = true;
    let (a, b) = tokio::join!(
        backend.save_schema_map_preferences(
            &first,
            scope.clone(),
            observed.revision.clone(),
            value()
        ),
        backend.save_schema_map_preferences(&second, scope.clone(), observed.revision, other)
    );
    let committed = match (a, b) {
        (Ok(saved), Err(SchemaMapPreferencesError::StaleRevision))
        | (Err(SchemaMapPreferencesError::StaleRevision), Ok(saved)) => saved,
        result => panic!("Expected one exact CAS winner, got {result:?}"),
    };
    assert_eq!(
        backend
            .load_schema_map_preferences(&first, scope)
            .await
            .unwrap(),
        committed
    );
    backend.shutdown().await.unwrap();
}
