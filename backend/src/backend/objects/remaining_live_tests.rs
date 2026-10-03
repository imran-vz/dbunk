//! Opt-in catalog-only description acceptance against the root-owned corpus.
use super::*;
use crate::backend::{data::DataCloseOutcome, profile};
use futures_util::FutureExt;

const SCHEMA: &str = "native_metadata_20261003";
fn reference(kind: PgObjectKind, name: &str) -> PgObjectRef {
    PgObjectRef {
        kind,
        schema: Some(SCHEMA.into()),
        name: name.into(),
        identity_args: None,
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "verified owned stage03 fixture plus root-created native_metadata_20261003 corpus only; run serially"]
async fn remaining_descriptions_exact_metadata_and_owned_cleanup() {
    assert_eq!(
        std::env::var("DBUNK_NATIVE_FIXTURE_VERIFIED").as_deref(),
        Ok("1"),
        "explicit owned fixture opt-in required"
    );
    // Verifies process/container and SQL instance identity before facade sockets.
    let baseline = live_tests::fixture_count().await;
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
            .open_data_document(
                "remaining-descriptions-live",
                "objects",
                &backend.fixture().id,
            )
            .await
            .unwrap();
        let catalog = backend.load_object_catalog(&document).await.unwrap();
        assert!(catalog.truncated.is_empty());
        let schema = catalog
            .schemas
            .iter()
            .find(|schema| schema.name == SCHEMA)
            .expect("root-created description corpus missing; do not create or adopt one here");
        let mut descriptions = std::collections::BTreeMap::new();
        for (kind, name) in [
            (PgObjectKind::Table, "generated_table"),
            (PgObjectKind::Table, "partitioned"),
            (PgObjectKind::Table, "partition_leaf"),
            (PgObjectKind::ForeignTable, "foreign_metadata"),
            (PgObjectKind::Type, "mood"),
            (PgObjectKind::Type, "pair"),
            (PgObjectKind::Type, "custom_range"),
            (PgObjectKind::Type, "custom_multirange"),
            (PgObjectKind::Domain, "positive_amount"),
        ] {
            let entries = match kind {
                PgObjectKind::Table => &schema.tables,
                PgObjectKind::ForeignTable => &schema.foreign_tables,
                PgObjectKind::Type => &schema.types,
                PgObjectKind::Domain => &schema.domains,
                _ => unreachable!(),
            };
            assert!(entries.iter().any(|entry| entry.name == name));
            let identity = reference(kind, name);
            let description = backend
                .describe_object(&document, identity.clone())
                .await
                .unwrap();
            assert_eq!(description.reference, identity);
            assert_eq!(
                description.owner.as_deref(),
                Some(backend.fixture().user.as_str())
            );
            assert!(serde_json::to_vec(&description).unwrap().len() <= MAX_DESCRIPTION_BYTES);
            descriptions.insert(name, description);
        }
        let sql = |name: &str| descriptions[name].definition_sql.as_deref().unwrap();
        let generated = sql("generated_table");
        assert!(generated.contains("\"id\" bigint GENERATED ALWAYS AS IDENTITY (START WITH 1 INCREMENT BY 1 MINVALUE 1 MAXVALUE 9223372036854775807 CACHE 1 NO CYCLE) NOT NULL"));
        assert!(generated.contains("\"base\" integer DEFAULT 2"));
        let computed = generated
            .lines()
            .find(|line| line.contains("\"doubled\""))
            .unwrap();
        assert!(computed.contains("\"doubled\" integer GENERATED ALWAYS AS ("));
        assert!(computed.contains("base * 2"));
        assert!(computed.ends_with(") STORED,") || computed.ends_with(") STORED"));
        assert!(!generated.contains("\"doubled\" integer DEFAULT"));
        assert!(generated.contains("CONSTRAINT \"positive\" CHECK (base > 0)"));
        assert!(generated.contains("PRIMARY KEY (id)"));
        assert!(generated.contains("CREATE INDEX generated_base_idx ON native_metadata_20261003.generated_table USING btree (base)"));
        assert!(
            !generated.contains("CREATE UNIQUE INDEX"),
            "primary-key index must not be duplicated"
        );
        assert!(matches!(
            descriptions["generated_table"].facts,
            PgObjectFacts::Table
        ));
        assert!(sql("partitioned").ends_with("PARTITION BY RANGE (id);"));
        assert_eq!(sql("partition_leaf"), "CREATE TABLE \"native_metadata_20261003\".\"partition_leaf\" PARTITION OF \"native_metadata_20261003\".\"partitioned\" FOR VALUES FROM (0) TO (100);");

        // This wrapper has NO HANDLER: successful description uses only catalogs.
        let foreign = sql("foreign_metadata");
        assert!(foreign.contains("\"id\" integer OPTIONS (\"column_name\" E'remote=id')"));
        assert!(foreign
            .contains("\"label\" text COLLATE \"pg_catalog\".\"C\" DEFAULT 'x'::text NOT NULL"));
        assert!(foreign.contains("CONSTRAINT \"present\" CHECK (label <> ''::text)"));
        assert!(foreign.ends_with("SERVER \"native_metadata_20261003_server\" OPTIONS (\"schema_name\" E'remote schema', \"table_name\" E'remote=table');"));
        assert_eq!(
            descriptions["foreign_metadata"].facts,
            PgObjectFacts::ForeignTable {
                server: "native_metadata_20261003_server".into()
            }
        );
        assert_eq!(
            descriptions["mood"].facts,
            PgObjectFacts::Type {
                class: PgTypeClass::Enum,
                enum_labels: Some(vec!["one".into(), "雪".into(), "quote'value".into()]),
                attributes: None,
                subtype: None,
            }
        );
        assert_eq!(sql("mood"), "CREATE TYPE \"native_metadata_20261003\".\"mood\" AS ENUM (E'one', E'雪', E'quote''value');");
        assert_eq!(
            descriptions["pair"].facts,
            PgObjectFacts::Type {
                class: PgTypeClass::Composite,
                enum_labels: None,
                attributes: Some(vec![
                    PgTypeAttribute {
                        name: "amount".into(),
                        data_type: "bigint".into(),
                        nullable: true
                    },
                    PgTypeAttribute {
                        name: "label".into(),
                        data_type: "text".into(),
                        nullable: true
                    },
                ]),
                subtype: None,
            }
        );
        assert!(sql("pair").contains("\"amount\" bigint,\n  \"label\" text"));
        for (name, class) in [
            ("custom_range", PgTypeClass::Range),
            ("custom_multirange", PgTypeClass::Multirange),
        ] {
            assert_eq!(
                descriptions[name].facts,
                PgObjectFacts::Type {
                    class,
                    enum_labels: None,
                    attributes: None,
                    subtype: Some("integer".into())
                }
            );
            let range = sql(name);
            assert!(range
                .starts_with("CREATE TYPE \"native_metadata_20261003\".\"custom_range\" AS RANGE"));
            assert!(range.contains("SUBTYPE = \"pg_catalog\".\"int4\""));
            assert!(range.contains("SUBTYPE_OPCLASS = \"pg_catalog\".\"int4_ops\""));
            assert!(range.contains("SUBTYPE_DIFF = \"pg_catalog\".\"int4range_subdiff\""));
            assert!(range.contains(
                "MULTIRANGE_TYPE_NAME = \"native_metadata_20261003\".\"custom_multirange\""
            ));
        }
        assert_eq!(sql("custom_range"), sql("custom_multirange"));
        let PgObjectFacts::Domain {
            base_type,
            not_null,
            default_value,
            checks,
        } = &descriptions["positive_amount"].facts
        else {
            panic!("domain facts missing")
        };
        assert_eq!(base_type, "numeric(12,2)");
        assert!(*not_null);
        assert_eq!(default_value.as_deref(), Some("1.25"));
        assert_eq!(checks.len(), 1);
        assert!(checks[0].contains("VALUE >"));
        assert!(sql("positive_amount").starts_with("CREATE DOMAIN \"native_metadata_20261003\".\"positive_amount\" AS numeric(12,2) DEFAULT 1.25 NOT NULL\n  CHECK"));

        for identity in [
            reference(PgObjectKind::ForeignTable, "generated_table"),
            reference(PgObjectKind::Table, "foreign_metadata"),
            reference(PgObjectKind::Domain, "mood"),
            reference(PgObjectKind::Type, "positive_amount"),
        ] {
            assert!(matches!(
                backend.describe_object(&document, identity).await,
                Err(DataError::Catalog(CatalogError::ObjectNotFound))
            ));
        }
        backend.cancel_data(&document).await.unwrap();
        assert_eq!(
            backend
                .describe_object(&document, reference(PgObjectKind::Type, "mood"))
                .await
                .unwrap(),
            descriptions["mood"]
        );
        assert_eq!(
            backend.close_data_document(&document).await.unwrap(),
            DataCloseOutcome::Closed
        );
        for kind in [
            PgObjectKind::Table,
            PgObjectKind::ForeignTable,
            PgObjectKind::Type,
            PgObjectKind::Domain,
        ] {
            assert!(matches!(
                backend
                    .describe_object(&document, reference(kind, "retired"))
                    .await,
                Err(DataError::Document(_))
            ));
        }
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
        while live_tests::fixture_count().await != baseline {
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    })
    .await;
    shutdown.expect("owned description sockets and tasks joined");
    restored.expect("fixture activity returned to its pre-test baseline");
    result
        .unwrap_or_else(|panic| std::panic::resume_unwind(panic))
        .expect("description acceptance deadline");
}
