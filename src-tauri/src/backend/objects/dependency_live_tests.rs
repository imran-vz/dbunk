//! Read-only acceptance against a root-created dependency corpus. Never runs DDL.
use super::live_tests::fixture_count;
use super::*;
use crate::backend::{data::DataCloseOutcome, profile};
use futures_util::FutureExt;

const SCHEMA: &str = "native_impact_20261003";
const EXTERNAL: &str = "native_impact_external_20261003";
fn reference(kind: PgObjectKind, name: &str) -> PgObjectRef {
    PgObjectRef {
        kind,
        schema: (kind != PgObjectKind::Schema).then(|| SCHEMA.into()),
        name: name.into(),
        identity_args: None,
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "verified owned stage03 fixture plus root-created native_impact_20261003 corpus only; run serially"]
async fn native_drop_impact_exact_dependencies_limits_and_cleanup() {
    assert_eq!(
        std::env::var("DBUNK_NATIVE_FIXTURE_VERIFIED").as_deref(),
        Ok("1"),
        "explicit owned fixture opt-in required"
    );
    // Verifies the process/container and SQL instance before any facade socket.
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
            .open_data_document("dependency-live", "impact", &backend.fixture().id)
            .await
            .unwrap();
        let catalog = backend.load_object_catalog(&document).await.unwrap();
        assert!(catalog.truncated.is_empty());
        let schema = catalog
            .schemas
            .iter()
            .find(|schema| schema.name == SCHEMA)
            .expect("root-created dependency corpus missing; do not create/adopt it here");
        assert!(catalog.schemas.iter().any(|schema| schema.name == EXTERNAL));
        assert!(schema.tables.iter().any(|entry| entry.name == "base"));
        let base_ref = reference(PgObjectKind::Table, "base");
        let base = backend
            .load_object_drop_impact(&document, base_ref.clone())
            .await
            .unwrap();
        for identity in [
            format!("{SCHEMA}.base_view"),
            format!("{EXTERNAL}.external_view"),
        ] {
            assert!(
                base.dependents
                    .iter()
                    .any(|entry| entry.object_type == "view"
                        && entry.identity == identity
                        && entry.depth == 1),
                "direct view missing: {identity}; {base:?}"
            );
        }
        assert!(
            base.dependents
                .iter()
                .any(|entry| entry.object_type == "sequence"
                    && entry.identity == format!("{SCHEMA}.base_id_seq")),
            "identity sequence missing: {base:?}"
        );
        assert!(
            base.dependents
                .iter()
                .any(|entry| entry.object_type == "rule"
                    && entry.identity.contains("base_noop")
                    && entry.identity.contains(SCHEMA)),
            "custom rule must remain a rule, not become its owning table: {base:?}"
        );
        assert!(
            !base
                .dependents
                .iter()
                .any(|entry| entry.identity.contains("_RETURN")),
            "view return rules must be normalized into view identities"
        );
        assert!(base.truncated, "ten-view chain extends beyond depth eight");
        assert!(base.dependents.len() <= MAX_DROP_IMPACT_RESULTS);
        assert!(serde_json::to_vec(&base).unwrap().len() <= MAX_DROP_IMPACT_BYTES);

        let schema_impact = backend
            .load_object_drop_impact(&document, reference(PgObjectKind::Schema, SCHEMA))
            .await
            .unwrap();
        assert!(
            schema_impact
                .dependents
                .iter()
                .any(|entry| entry.object_type == "view"
                    && entry.identity == format!("{EXTERNAL}.external_view")),
            "schema impact must include cross-schema dependent views: {schema_impact:?}"
        );
        let domain = backend
            .load_object_drop_impact(&document, reference(PgObjectKind::Domain, "amount"))
            .await
            .unwrap();
        assert!(!domain.truncated);
        assert!(
            domain
                .dependents
                .iter()
                .any(|entry| entry.object_type == "table column"
                    && entry.identity == format!("{SCHEMA}.domain_rows.value")),
            "domain dependency must retain its column address: {domain:?}"
        );
        assert!(
            !domain
                .dependents
                .iter()
                .any(|entry| entry.object_type == "table"
                    && entry.identity == format!("{SCHEMA}.domain_rows")),
            "a dependent column must not be inflated to the whole table"
        );
        assert!(!domain
            .dependents
            .iter()
            .any(|entry| entry.identity.ends_with(".untouched")));
        let chain = backend
            .load_object_drop_impact(&document, reference(PgObjectKind::View, "v1"))
            .await
            .unwrap();
        assert!(chain.truncated);
        assert!(chain
            .dependents
            .iter()
            .any(|entry| entry.identity == format!("{SCHEMA}.v2") && entry.depth == 1));
        assert!(chain.dependents.iter().all(|entry| entry.depth <= 8));
        assert!(!chain
            .dependents
            .iter()
            .any(|entry| entry.identity == format!("{SCHEMA}.v10")));

        let routines = schema
            .functions
            .iter()
            .filter(|entry| entry.name == "overloaded")
            .collect::<Vec<_>>();
        assert_eq!(routines.len(), 2);
        assert_ne!(routines[0].identity_args, routines[1].identity_args);
        for entry in routines {
            let mut identity = reference(PgObjectKind::Function, &entry.name);
            identity.identity_args = Some(
                entry
                    .identity_args
                    .clone()
                    .expect("catalog must retain overload identity"),
            );
            let impact = backend
                .load_object_drop_impact(&document, identity)
                .await
                .unwrap();
            assert!(
                impact.dependents.is_empty() && !impact.truncated,
                "SQL function body has no declared external dependencies: {impact:?}"
            );
        }
        for identity in [
            reference(PgObjectKind::View, "base"),
            reference(PgObjectKind::Table, "missing"),
            reference(PgObjectKind::Type, "amount"),
            PgObjectRef {
                schema: Some(EXTERNAL.into()),
                ..base_ref.clone()
            },
            PgObjectRef {
                identity_args: Some("bigint".into()),
                ..reference(PgObjectKind::Function, "overloaded")
            },
            PgObjectRef {
                identity_args: Some("integer".into()),
                ..reference(PgObjectKind::Procedure, "overloaded")
            },
        ] {
            assert!(matches!(
                backend.load_object_drop_impact(&document, identity).await,
                Err(DataError::Catalog(CatalogError::ObjectNotFound))
            ));
        }
        assert!(matches!(
            backend
                .load_object_drop_impact(&document, reference(PgObjectKind::Function, "overloaded"))
                .await,
            Err(DataError::Catalog(CatalogError::InvalidReference))
        ));
        backend.cancel_data(&document).await.unwrap();
        assert_eq!(
            backend
                .load_object_drop_impact(&document, base_ref.clone())
                .await
                .unwrap(),
            base
        );
        assert_eq!(
            backend.close_data_document(&document).await.unwrap(),
            DataCloseOutcome::Closed
        );
        assert!(matches!(
            backend.load_object_drop_impact(&document, base_ref).await,
            Err(DataError::Document(_))
        ));
        assert!(matches!(
            backend.cancel_data(&document).await,
            Err(DataError::Document(_))
        ));
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
    shutdown.expect("owned dependency readers and sockets joined");
    restored.expect("fixture activity returned to pre-test baseline");
    result
        .unwrap_or_else(|panic| std::panic::resume_unwind(panic))
        .expect("dependency acceptance deadline");
}
