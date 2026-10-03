//! Opt-in read-only acceptance; never creates database objects or an admin socket.
use super::*;
use crate::backend::{data::DataCloseOutcome, profile};
use futures_util::FutureExt;
use std::path::Path;

pub(super) async fn fixture_count() -> u64 {
    tokio::task::spawn_blocking(|| {
        let output = std::process::Command::new("python3")
            .arg(Path::new(env!("CARGO_MANIFEST_DIR")).join("../tools/native/fixture.py"))
            .arg("count")
            .output()
            .expect("run owned fixture verification");
        assert!(
            output.status.success(),
            "owned fixture verification failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8(output.stdout)
            .unwrap()
            .trim()
            .parse()
            .unwrap()
    })
    .await
    .unwrap()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "verified owned stage03 fixture only; run serially without window acceptance"]
async fn native_catalog_owned_read_cancel_reopen_and_shutdown() {
    assert_eq!(
        std::env::var("DBUNK_NATIVE_FIXTURE_VERIFIED").as_deref(),
        Ok("1"),
        "explicit owned fixture opt-in required"
    );
    // count verifies the fixture's container/process and SQL instance sentinel
    // before this test opens the facade's first PostgreSQL socket.
    let baseline = fixture_count().await;
    let directory = profile::directory();
    let mut backend = None;
    let operation = async {
        backend = Some(
            Backend::open_fixture(&directory.path().canonicalize().unwrap())
                .await
                .unwrap(),
        );
        let backend = backend.as_ref().unwrap();
        let document = backend
            .open_data_document("catalog-live", "objects", &backend.fixture().id)
            .await
            .unwrap();
        let catalog = backend.load_object_catalog(&document).await.unwrap();
        assert!(
            catalog.truncated.is_empty(),
            "fixture catalog should fit baseline per-kind caps"
        );
        let owned = catalog
            .schemas
            .iter()
            .find(|schema| schema.name == "plan026")
            .expect("owned fixture schema identity");
        assert!(owned
            .tables
            .iter()
            .any(|entry| entry.name == "fixture_identity" && entry.identity_args.is_none()));
        assert!(owned
            .tables
            .iter()
            .any(|entry| entry.name == "policy_probe"));
        assert!(owned.views.iter().any(|entry| entry.name == "exact_values"));
        let measured = catalog
            .schemas
            .iter()
            .find(|schema| schema.name == "plan024")
            .unwrap();
        for name in ["fixture_wide", "fixture_large", "fixture_many"] {
            assert!(measured.views.iter().any(|entry| entry.name == name));
        }
        assert!(catalog.roles.iter().any(|entry| entry.name == "dbunk"));
        assert!(catalog
            .tablespaces
            .iter()
            .any(|entry| entry.name == "pg_default"));
        assert!(catalog
            .schemas
            .iter()
            .all(|schema| schema.name != "pg_catalog" && schema.name != "information_schema"));
        assert!(serde_json::to_vec(&catalog).unwrap().len() <= MAX_CATALOG_BYTES);
        // Idle cancellation must not poison the next read. In-flight cancel and
        // the join barrier are deterministic in the headless ownership tests.
        backend.cancel_data(&document).await.unwrap();
        assert_eq!(
            backend.load_object_catalog(&document).await.unwrap(),
            catalog
        );
        assert_eq!(
            backend.close_data_document(&document).await.unwrap(),
            DataCloseOutcome::Closed
        );
        assert!(matches!(
            backend.load_object_catalog(&document).await,
            Err(DataError::Document(_))
        ));
        assert!(matches!(
            backend.cancel_data(&document).await,
            Err(DataError::Document(_))
        ));
        let replacement = backend
            .open_data_document("catalog-live", "objects", &backend.fixture().id)
            .await
            .unwrap();
        assert_eq!(
            backend.load_object_catalog(&replacement).await.unwrap(),
            catalog
        );
        assert!(matches!(
            backend.load_object_catalog(&document).await,
            Err(DataError::Document(_))
        ));
        backend.close_data_document(&replacement).await.unwrap();
    };
    // Cleanup also runs after assertion failures/timeouts.
    let result =
        std::panic::AssertUnwindSafe(tokio::time::timeout(Duration::from_secs(60), operation))
            .catch_unwind()
            .await;
    let shutdown = match backend {
        Some(backend) => backend.shutdown().await,
        None => Ok(()),
    };
    let restored = tokio::time::timeout(Duration::from_secs(10), async {
        while fixture_count().await != baseline {
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    })
    .await;
    shutdown.expect("owned catalog tasks and sockets joined");
    restored.expect("fixture activity returned to its pre-test baseline");
    result
        .unwrap_or_else(|panic| std::panic::resume_unwind(panic))
        .expect("catalog acceptance deadline");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "verified owned stage03 fixture plus root-created native_objects_20261003 schema only; run serially"]
async fn native_descriptions_exact_identity_values_and_document_lifecycle() {
    const SCHEMA: &str = "native_objects_20261003";
    assert_eq!(
        std::env::var("DBUNK_NATIVE_FIXTURE_VERIFIED").as_deref(),
        Ok("1"),
        "explicit owned fixture opt-in required"
    );
    // fixture_count verifies the owned endpoint/process and SQL instance before
    // any facade socket starts. Root owns creation/removal of the named corpus.
    let baseline = fixture_count().await;
    let directory = profile::directory();
    let mut backend = None;
    let operation = async {
        backend = Some(
            Backend::open_fixture(&directory.path().canonicalize().unwrap())
                .await
                .unwrap(),
        );
        let backend = backend.as_ref().unwrap();
        let document = backend
            .open_data_document("description-live", "objects", &backend.fixture().id)
            .await
            .unwrap();
        let catalog = backend.load_object_catalog(&document).await.unwrap();
        assert!(catalog.truncated.is_empty());
        let objects = catalog
            .schemas
            .iter()
            .find(|schema| schema.name == SCHEMA)
            .expect("root-created description corpus is absent; do not create/adopt one here");
        let reference = |kind, name: &str, identity_args: Option<String>| PgObjectRef {
            kind,
            schema: (kind != PgObjectKind::Schema).then(|| SCHEMA.to_owned()),
            name: name.into(),
            identity_args,
        };
        let schema_ref = reference(PgObjectKind::Schema, SCHEMA, None);
        let schema = backend
            .describe_object(&document, schema_ref.clone())
            .await
            .unwrap();
        assert_eq!(schema.reference, schema_ref);
        assert_eq!(
            schema.owner.as_deref(),
            Some(backend.fixture().user.as_str())
        );
        assert!(matches!(schema.facts, PgObjectFacts::Schema));
        assert_eq!(
            schema.definition_sql.as_deref(),
            Some("CREATE SCHEMA \"native_objects_20261003\";")
        );

        let view_ref = reference(PgObjectKind::View, "exact_view", None);
        assert!(objects
            .views
            .iter()
            .any(|entry| entry.name == view_ref.name));
        let view = backend
            .describe_object(&document, view_ref.clone())
            .await
            .unwrap();
        assert_eq!(view.reference, view_ref);
        let PgObjectFacts::View { definition } = &view.facts else {
            panic!("view facts missing");
        };
        assert!(definition.contains("9223372036854775807"));
        assert!(definition.contains(" AS n"));
        assert!(!definition.ends_with(';'));
        assert!(view
            .definition_sql
            .as_ref()
            .unwrap()
            .starts_with("CREATE VIEW \"native_objects_20261003\".\"exact_view\" AS\n"));

        let mat_ref = reference(PgObjectKind::MaterializedView, "exact_mat", None);
        assert!(objects
            .materialized_views
            .iter()
            .any(|entry| entry.name == mat_ref.name));
        let materialized = backend
            .describe_object(&document, mat_ref.clone())
            .await
            .unwrap();
        assert_eq!(materialized.reference, mat_ref);
        let PgObjectFacts::MaterializedView {
            definition,
            populated,
        } = &materialized.facts
        else {
            panic!("materialized-view facts missing");
        };
        assert!(definition.contains("9223372036854775807"));
        assert!(
            !populated,
            "inspection must not refresh a WITH NO DATA view"
        );
        assert!(materialized
            .definition_sql
            .as_ref()
            .unwrap()
            .ends_with("WITH NO DATA;"));

        let sequence_ref = reference(PgObjectKind::Sequence, "exact_seq", None);
        assert!(objects
            .sequences
            .iter()
            .any(|entry| entry.name == sequence_ref.name));
        let sequence = backend
            .describe_object(&document, sequence_ref.clone())
            .await
            .unwrap();
        assert_eq!(sequence.reference, sequence_ref);
        let PgObjectFacts::Sequence {
            data_type,
            start,
            increment,
            min_value,
            max_value,
            cycle,
            cache,
            last_value,
            owned_by,
        } = &sequence.facts
        else {
            panic!("sequence facts missing");
        };
        assert_eq!(data_type, "bigint");
        assert_eq!(start, "9223372036854775800");
        assert_eq!(increment, "-1");
        assert_eq!(min_value, "-9223372036854775808");
        assert_eq!(max_value, "9223372036854775807");
        assert!(!cycle);
        assert_eq!(cache, "1");
        assert!(
            last_value.is_none(),
            "inspection must not advance an unused sequence"
        );
        assert!(owned_by.is_none());
        assert!(sequence
            .definition_sql
            .as_ref()
            .unwrap()
            .contains("START WITH 9223372036854775800"));

        let overloads = objects
            .functions
            .iter()
            .filter(|entry| entry.name == "overloaded")
            .collect::<Vec<_>>();
        assert_eq!(overloads.len(), 2);
        let mut identities = overloads
            .iter()
            .map(|entry| {
                entry
                    .identity_args
                    .clone()
                    .expect("routine identity required")
            })
            .collect::<Vec<_>>();
        identities.sort();
        assert_eq!(identities, ["integer", "text"]);
        for entry in overloads {
            let identity = entry.identity_args.as_deref().unwrap();
            let routine_ref = reference(
                PgObjectKind::Function,
                &entry.name,
                entry.identity_args.clone(),
            );
            let routine = backend
                .describe_object(&document, routine_ref.clone())
                .await
                .unwrap();
            assert_eq!(routine.reference, routine_ref);
            let PgObjectFacts::Routine {
                language,
                returns,
                arguments,
                body,
                strict,
                security_definer,
                parallel,
                ..
            } = &routine.facts
            else {
                panic!("function facts missing");
            };
            assert_eq!(language, "sql");
            assert_eq!(returns.as_deref(), Some(identity));
            assert_eq!(arguments, identity);
            assert_eq!(body.as_deref(), Some("SELECT $1"));
            assert!(!strict);
            assert!(!security_definer);
            assert_eq!(parallel.as_deref(), Some("unsafe"));
            assert!(routine
                .definition_sql
                .as_ref()
                .unwrap()
                .contains("SELECT $1"));
            assert!(serde_json::to_vec(&routine).unwrap().len() <= MAX_DESCRIPTION_BYTES);
        }

        let procedure_entry = objects
            .procedures
            .iter()
            .find(|entry| entry.name == "no_op")
            .expect("procedure fixture");
        let procedure_ref = reference(
            PgObjectKind::Procedure,
            &procedure_entry.name,
            procedure_entry.identity_args.clone(),
        );
        let procedure = backend
            .describe_object(&document, procedure_ref.clone())
            .await
            .unwrap();
        assert_eq!(procedure.reference, procedure_ref);
        let PgObjectFacts::Routine {
            language,
            returns,
            arguments,
            body,
            security_definer,
            ..
        } = &procedure.facts
        else {
            panic!("procedure facts missing");
        };
        assert_eq!(language, "plpgsql");
        assert!(returns.is_none());
        assert!(arguments.contains("value integer"));
        assert_eq!(
            body.as_deref()
                .map(|body| body.trim().trim_end_matches(';')),
            Some("BEGIN NULL; END")
        );
        assert!(!security_definer);
        assert!(procedure
            .definition_sql
            .as_ref()
            .unwrap()
            .contains("CREATE OR REPLACE PROCEDURE"));

        let aggregate_entry = objects
            .aggregates
            .iter()
            .find(|entry| entry.name == "sum_big")
            .expect("aggregate fixture");
        let aggregate_ref = reference(
            PgObjectKind::Aggregate,
            &aggregate_entry.name,
            aggregate_entry.identity_args.clone(),
        );
        let aggregate = backend
            .describe_object(&document, aggregate_ref.clone())
            .await
            .unwrap();
        assert_eq!(aggregate.reference, aggregate_ref);
        let PgObjectFacts::Routine {
            returns,
            arguments,
            body,
            parallel,
            ..
        } = &aggregate.facts
        else {
            panic!("aggregate facts missing");
        };
        assert_eq!(returns.as_deref(), Some("bigint"));
        assert_eq!(arguments, "bigint");
        assert!(body.is_none());
        assert!(parallel.is_none());
        assert!(
            aggregate.definition_sql.is_none(),
            "baseline aggregate description has no reconstructed definition"
        );

        // The guarded fixture setup independently verified this installed
        // system extension. Catalog intentionally excludes system schemas.
        let extension_ref = PgObjectRef {
            kind: PgObjectKind::Extension,
            schema: Some("pg_catalog".into()),
            name: "plpgsql".into(),
            identity_args: None,
        };
        let extension = backend
            .describe_object(&document, extension_ref.clone())
            .await
            .unwrap();
        assert_eq!(extension.reference, extension_ref);
        let PgObjectFacts::Extension {
            version,
            schema: described_schema,
        } = &extension.facts
        else {
            panic!("extension facts missing")
        };
        assert!(!version.is_empty());
        assert_eq!(described_schema, "pg_catalog");
        assert!(extension
            .definition_sql
            .as_ref()
            .unwrap()
            .starts_with("CREATE EXTENSION \"plpgsql\" WITH SCHEMA "));
        let mut wrong_schema = extension_ref;
        wrong_schema.schema = Some(SCHEMA.into());
        assert!(matches!(
            backend.describe_object(&document, wrong_schema).await,
            Err(DataError::Catalog(CatalogError::ObjectNotFound))
        ));

        let mut wrong_schema = view_ref.clone();
        wrong_schema.schema = Some("plan026".into());
        let wrong_overload =
            reference(PgObjectKind::Function, "overloaded", Some("boolean".into()));
        let wrong_kind = reference(PgObjectKind::MaterializedView, "exact_view", None);
        for mismatch in [wrong_schema, wrong_overload, wrong_kind] {
            assert!(matches!(
                backend.describe_object(&document, mismatch).await,
                Err(DataError::Catalog(CatalogError::ObjectNotFound))
            ));
        }
        backend.cancel_data(&document).await.unwrap();
        assert_eq!(
            backend
                .describe_object(&document, sequence_ref)
                .await
                .unwrap(),
            sequence
        );
        assert_eq!(
            backend.close_data_document(&document).await.unwrap(),
            DataCloseOutcome::Closed
        );
        assert!(matches!(
            backend.describe_object(&document, view_ref.clone()).await,
            Err(DataError::Document(_))
        ));
        assert!(matches!(
            backend.cancel_data(&document).await,
            Err(DataError::Document(_))
        ));
        let replacement = backend
            .open_data_document("description-live", "objects", &backend.fixture().id)
            .await
            .unwrap();
        assert_eq!(
            backend
                .describe_object(&replacement, view_ref.clone())
                .await
                .unwrap(),
            view
        );
        assert!(matches!(
            backend.describe_object(&document, view_ref).await,
            Err(DataError::Document(_))
        ));
        backend.close_data_document(&replacement).await.unwrap();
    };
    let result =
        std::panic::AssertUnwindSafe(tokio::time::timeout(Duration::from_secs(90), operation))
            .catch_unwind()
            .await;
    let shutdown = match backend {
        Some(backend) => backend.shutdown().await,
        None => Ok(()),
    };
    let restored = tokio::time::timeout(Duration::from_secs(10), async {
        while fixture_count().await != baseline {
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    })
    .await;
    shutdown.expect("description tasks and sockets joined after success/failure");
    restored.expect("fixture activity returned to pre-test baseline");
    result
        .unwrap_or_else(|panic| std::panic::resume_unwind(panic))
        .expect("description acceptance deadline");
}
