use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use tokio::sync::Mutex;
use tokio_postgres::types::ToSql;

use crate::postgres::connect_spec::ResolvedPostgresConnectSpec;
use crate::query_session::postgres as session_postgres;
use crate::storage;
use crate::PgDriverOptions;

use super::protocol::*;
use super::{ResultMutationManager, VirtualKeyLookup};

type VirtualKeys = Arc<Mutex<HashMap<(String, String, String), VirtualKey>>>;

fn live_spec(connection_id: &str) -> ResolvedPostgresConnectSpec {
    ResolvedPostgresConnectSpec {
        connection_id: connection_id.into(),
        host: "127.0.0.1".into(),
        port: 15432,
        database: "dbunk_demo".into(),
        user: "dbunk".into(),
        password: "dbunk".into(),
        tls: crate::postgres::tls::ResolvedTls::prefer("127.0.0.1"),
        connect_timeout: Some(Duration::from_secs(5)),
        keepalive: None,
        driver_options: PgDriverOptions::default(),
        safety_policy: Default::default(),
    }
}

fn empty_virtual_keys() -> VirtualKeyLookup {
    Arc::new(|_, _, _| Box::pin(async { Ok(None) }))
}

fn virtual_key_lookup(keys: VirtualKeys) -> VirtualKeyLookup {
    Arc::new(move |connection_id, schema, table| {
        let keys = keys.clone();
        Box::pin(async move {
            Ok(keys
                .lock()
                .await
                .get(&(connection_id, schema, table))
                .cloned())
        })
    })
}

fn statement_payload(
    connection_id: &str,
    tab_id: &str,
    request_id: u64,
    sql: impl Into<String>,
) -> AnalyzeResultSetPayload {
    AnalyzeResultSetPayload {
        connection_id: connection_id.into(),
        tab_id: tab_id.into(),
        request_id,
        source: AnalyzeSource::Statement { sql: sql.into() },
        refresh_structure: false,
    }
}

fn relation_payload(
    connection_id: &str,
    tab_id: &str,
    request_id: u64,
    schema: &str,
    table: &str,
) -> AnalyzeResultSetPayload {
    AnalyzeResultSetPayload {
        connection_id: connection_id.into(),
        tab_id: tab_id.into(),
        request_id,
        source: AnalyzeSource::Relation {
            schema: schema.into(),
            table: table.into(),
        },
        refresh_structure: false,
    }
}

fn table(schema: &str, table: &str) -> MutationTable {
    MutationTable {
        schema: schema.into(),
        table: table.into(),
    }
}

fn value(column: &str, value: Option<&str>) -> MutationValue {
    MutationValue {
        column: column.into(),
        value: value.map(str::to_owned),
    }
}

async fn cleanup_schema(admin: &session_postgres::SessionConnection, schema: &str) {
    admin
        .client
        .batch_execute(&format!("DROP SCHEMA IF EXISTS {schema} CASCADE"))
        .await
        .ok();
}

fn unique_schema() -> String {
    format!("result_mutation_live_{}", uuid::Uuid::new_v4().simple())
}

#[tokio::test]
#[ignore = "requires pnpm db:postgres"]
async fn result_mutation_live_idle_close_releases_socket_worker_and_capacity() {
    let connection_id = "result-mutation-idle-close";
    let spec = live_spec(connection_id);
    let admin = session_postgres::connect(&spec).await.expect("admin");
    let manager = ResultMutationManager::new();
    manager
        .analyze(
            spec.clone(),
            relation_payload(
                connection_id,
                "idle",
                1,
                "result_mutation_fixture",
                "generated_identity",
            ),
            empty_virtual_keys(),
        )
        .await
        .expect("open mutation executor");
    let executor = manager
        .existing_executor(connection_id)
        .await
        .expect("installed executor");
    let backend_pid: i32 = executor
        .state
        .lock()
        .await
        .connection
        .as_ref()
        .expect("open socket")
        .inner
        .client
        .query_one("SELECT pg_backend_pid()", &[])
        .await
        .expect("backend pid")
        .get(0);
    executor.state.lock().await.last_used = std::time::Instant::now() - super::IDLE_TIMEOUT;

    manager.close_idle().await;

    assert!(manager.existing_executor(connection_id).await.is_none());
    assert!(executor.state.lock().await.connection.is_none());
    let closed = tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            let gone: bool = admin
                .client
                .query_one(
                    "SELECT NOT EXISTS (SELECT 1 FROM pg_stat_activity WHERE pid = $1)",
                    &[&backend_pid],
                )
                .await
                .expect("inspect socket lifecycle")
                .get(0);
            if gone {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await;
    assert!(closed.is_ok(), "idle backend socket closed");
    let worker_stopped = tokio::time::timeout(Duration::from_secs(1), async {
        loop {
            let notified = executor.notify.notified();
            tokio::pin!(notified);
            if !executor
                .worker_running
                .load(std::sync::atomic::Ordering::Acquire)
            {
                break;
            }
            notified.await;
        }
    })
    .await;
    assert!(worker_stopped.is_ok(), "idle analysis worker stopped");

    manager
        .analyze(
            spec,
            relation_payload(
                connection_id,
                "replacement",
                1,
                "result_mutation_fixture",
                "generated_identity",
            ),
            empty_virtual_keys(),
        )
        .await
        .expect("idle close released executor capacity");
}

#[tokio::test]
#[ignore = "requires pnpm db:postgres"]
async fn result_mutation_live_analysis_matrix() {
    let connection_id = "result-mutation-analysis";
    let spec = live_spec(connection_id);
    let admin = session_postgres::connect(&spec).await.expect("admin");
    let manager = ResultMutationManager::new();
    let lookup = empty_virtual_keys();

    let single = manager
        .analyze(
            spec.clone(),
            statement_payload(
                connection_id,
                "single",
                1,
                "SELECT id, body, note, body_length FROM result_mutation_fixture.generated_identity",
            ),
            lookup.clone(),
        )
        .await
        .expect("single-table analysis");
    assert_eq!(single.statement, AnalysisStatement::Analyzed);
    assert_eq!(single.tables.len(), 1);
    assert_eq!(
        single.tables[0].identity.kind,
        MutationIdentityKind::PrimaryKey
    );
    assert!(single.tables[0].identity_projected);
    assert_eq!(
        single.columns[0].writability,
        ColumnWritability::IdentityAlways
    );
    assert_eq!(single.columns[3].writability, ColumnWritability::Generated);
    assert!(single
        .columns
        .iter()
        .all(|column| matches!(column.origin, ColumnOrigin::Table { .. })));

    let join = manager
        .analyze(
            spec.clone(),
            statement_payload(
                connection_id,
                "join",
                1,
                "SELECT g.id, g.body, a.id AS account_id, a.name FROM result_mutation_fixture.generated_identity g JOIN crm.accounts a ON true",
            ),
            lookup.clone(),
        )
        .await
        .expect("join analysis");
    assert_eq!(join.tables.len(), 2);
    assert!(join.tables.iter().all(|table| table.updatable.allowed));
    assert!(join.tables.iter().all(|table| !table.deletable.allowed));
    assert!(join.tables.iter().all(|table| !table.insertable.allowed));
    let crafted_join_insert = MutationPlan {
        operations: vec![MutationOp::Insert {
            table: table("result_mutation_fixture", "generated_identity"),
            values: vec![value("body", Some("must-not-insert"))],
        }],
    };
    let join_preview = manager
        .preview(PreviewResultMutationsPayload {
            connection_id: connection_id.into(),
            tab_id: "join".into(),
            analysis_id: join.analysis_id,
            plan: crafted_join_insert.clone(),
        })
        .await;
    assert_eq!(
        join_preview,
        Err(ResultMutationError::InvalidPlan {
            reason: InvalidPlanReason::MultipleOriginTables
        })
    );
    let join_apply = manager
        .apply(
            spec.clone(),
            ApplyResultMutationsPayload {
                connection_id: connection_id.into(),
                tab_id: "join".into(),
                request_id: 2,
                confirmed: false,
                analysis_id: join.analysis_id,
                plan: crafted_join_insert,
            },
        )
        .await;
    assert_eq!(
        join_apply,
        Err(ResultMutationError::InvalidPlan {
            reason: InvalidPlanReason::MultipleOriginTables
        })
    );

    let self_join = manager
        .analyze(
            spec.clone(),
            statement_payload(
                connection_id,
                "self-join",
                1,
                "SELECT left_row.id AS left_id, right_row.body AS right_body FROM result_mutation_fixture.generated_identity left_row JOIN result_mutation_fixture.generated_identity right_row ON left_row.id = right_row.id",
            ),
            lookup.clone(),
        )
        .await
        .expect("typed self-join refusal");
    assert_eq!(
        self_join.statement,
        AnalysisStatement::NotAnalyzable {
            reason: NotAnalyzableReason::Database {
                code: None,
                message: "PostgreSQL could not unambiguously resolve relation range variables"
                    .into(),
                severity: None,
                position: None,
            }
        }
    );
    assert!(self_join.tables.is_empty());

    let duplicate_projection = manager
        .analyze(
            spec.clone(),
            statement_payload(
                connection_id,
                "duplicate-projection",
                1,
                "SELECT id, id FROM result_mutation_fixture.generated_identity",
            ),
            lookup.clone(),
        )
        .await
        .expect("ordinary duplicate projection remains analyzable");
    assert_eq!(duplicate_projection.statement, AnalysisStatement::Analyzed);
    assert_eq!(duplicate_projection.tables.len(), 1);

    let parameterized = manager
        .analyze(
            spec.clone(),
            statement_payload(
                connection_id,
                "parameterized",
                1,
                "SELECT id, body FROM result_mutation_fixture.generated_identity WHERE id = $1",
            ),
            lookup.clone(),
        )
        .await
        .expect("raw positional parameter analysis");
    assert_eq!(parameterized.statement, AnalysisStatement::Analyzed);

    let quoted_schema = unique_schema();
    admin
        .client
        .batch_execute(&format!(
            "CREATE SCHEMA {quoted_schema};
             CREATE TABLE {quoted_schema}.\"Odd Rows\" (id integer PRIMARY KEY, body text NOT NULL)"
        ))
        .await
        .expect("quoted-name fixture");
    let executor = manager
        .existing_executor(connection_id)
        .await
        .expect("analysis executor");
    let mutation_connection = executor
        .state
        .lock()
        .await
        .connection
        .clone()
        .expect("analysis connection");
    mutation_connection
        .inner
        .client
        .batch_execute("SET search_path TO result_mutation_fixture")
        .await
        .expect("set self-join search path");
    let mixed_spelling_self_join = manager
        .analyze(
            spec.clone(),
            statement_payload(
                connection_id,
                "mixed-spelling-self-join",
                1,
                "SELECT left_row.id, right_row.body \
                 FROM result_mutation_fixture.generated_identity left_row \
                 JOIN generated_identity right_row ON left_row.id = right_row.id",
            ),
            lookup.clone(),
        )
        .await
        .expect("qualified and unqualified self-join refusal");
    assert!(matches!(
        mixed_spelling_self_join.statement,
        AnalysisStatement::NotAnalyzable {
            reason: NotAnalyzableReason::Database { code: None, .. }
        }
    ));
    mutation_connection
        .inner
        .client
        .batch_execute("RESET search_path")
        .await
        .expect("restore analysis search path");
    mutation_connection
        .inner
        .client
        .batch_execute(&format!("SET search_path TO {quoted_schema}"))
        .await
        .expect("set analysis search path");
    let quoted_duplicate = manager
        .analyze(
            spec.clone(),
            statement_payload(
                connection_id,
                "quoted-duplicate",
                1,
                "SELECT id, id AS id_again FROM \"Odd Rows\"",
            ),
            lookup.clone(),
        )
        .await
        .expect("search-path and quoted-name duplicate projection");
    assert_eq!(quoted_duplicate.statement, AnalysisStatement::Analyzed);
    mutation_connection
        .inner
        .client
        .batch_execute("RESET search_path")
        .await
        .expect("restore analysis search path");
    cleanup_schema(&admin, &quoted_schema).await;

    let inheritance_schema = unique_schema();
    admin
        .client
        .batch_execute(&format!(
            "CREATE SCHEMA {inheritance_schema};
             CREATE TABLE {inheritance_schema}.ordinary_parent (
               id integer PRIMARY KEY,
               body text NOT NULL
             );
             CREATE TABLE {inheritance_schema}.ordinary_child ()
               INHERITS ({inheritance_schema}.ordinary_parent);
             CREATE TABLE {inheritance_schema}.partitioned_parent (
               id integer,
               body text NOT NULL
             ) PARTITION BY RANGE (id);
             CREATE TABLE {inheritance_schema}.partitioned_child
               PARTITION OF {inheritance_schema}.partitioned_parent
               FOR VALUES FROM (0) TO (100)"
        ))
        .await
        .expect("inheritance fixtures");
    let inherited = manager
        .analyze(
            spec.clone(),
            statement_payload(
                connection_id,
                "ordinary-inheritance",
                1,
                format!("SELECT id, body FROM {inheritance_schema}.ordinary_parent"),
            ),
            lookup.clone(),
        )
        .await
        .expect("ordinary inheritance refusal");
    assert!(matches!(
        inherited.statement,
        AnalysisStatement::NotAnalyzable {
            reason: NotAnalyzableReason::Database { code: None, .. }
        }
    ));
    let partitioned = manager
        .analyze(
            spec.clone(),
            statement_payload(
                connection_id,
                "partitioned-parent",
                1,
                format!(
                    "SELECT tableoid, ctid, id, body \
                     FROM {inheritance_schema}.partitioned_parent"
                ),
            ),
            lookup.clone(),
        )
        .await
        .expect("partitioned parent analysis");
    assert_eq!(partitioned.statement, AnalysisStatement::Analyzed);
    assert_eq!(
        partitioned.tables[0].identity,
        MutationIdentity {
            kind: MutationIdentityKind::CtidFallback,
            columns: vec!["tableoid".into(), "ctid".into()],
        }
    );
    cleanup_schema(&admin, &inheritance_schema).await;

    let expressions = manager
        .analyze(
            spec.clone(),
            statement_payload(
                connection_id,
                "expressions",
                1,
                "SELECT id, upper(body) AS upper_body, count(*) OVER () AS total FROM result_mutation_fixture.generated_identity",
            ),
            lookup.clone(),
        )
        .await
        .expect("expression analysis");
    assert!(matches!(
        expressions.columns[0].origin,
        ColumnOrigin::Table { .. }
    ));
    assert!(matches!(
        expressions.columns[1].origin,
        ColumnOrigin::Expression
    ));
    assert!(matches!(
        expressions.columns[2].origin,
        ColumnOrigin::Expression
    ));

    let system = manager
        .analyze(
            spec.clone(),
            statement_payload(
                connection_id,
                "system",
                1,
                "SELECT ctid, xmin, * FROM result_mutation_fixture.generated_identity",
            ),
            lookup.clone(),
        )
        .await
        .expect("system-column analysis");
    assert_eq!(
        system.columns[0].writability,
        ColumnWritability::SystemColumn
    );
    assert_eq!(
        system.columns[1].writability,
        ColumnWritability::SystemColumn
    );
    assert!(matches!(
        system.columns[0].origin,
        ColumnOrigin::Table { attnum: -1, .. }
    ));
    assert!(matches!(
        system.columns[1].origin,
        ColumnOrigin::Table { attnum: -2, .. }
    ));

    let multiple = manager
        .analyze(
            spec.clone(),
            statement_payload(
                connection_id,
                "multiple",
                1,
                "SELECT id FROM result_mutation_fixture.generated_identity; SELECT 1",
            ),
            lookup.clone(),
        )
        .await
        .expect("typed multi-statement result");
    assert_eq!(
        multiple.statement,
        AnalysisStatement::NotAnalyzable {
            reason: NotAnalyzableReason::MultiStatement
        }
    );

    let relation = manager
        .analyze(
            spec.clone(),
            relation_payload(
                connection_id,
                "relation",
                1,
                "result_mutation_fixture",
                "generated_identity",
            ),
            lookup.clone(),
        )
        .await
        .expect("relation analysis");
    assert_eq!(relation.columns.len(), 4);
    assert!(relation.tables[0].insertable.allowed);

    let shadow = session_postgres::connect(&spec)
        .await
        .expect("shadow connection");
    shadow
        .client
        .batch_execute(
            "CREATE TEMP TABLE generated_identity (id bigint PRIMARY KEY, body text, note text, body_length integer)",
        )
        .await
        .expect("create temp shadow");
    let shadowed = manager
        .analyze(
            spec,
            statement_payload(
                connection_id,
                "shadow",
                1,
                "SELECT id, body FROM result_mutation_fixture.generated_identity",
            ),
            lookup,
        )
        .await
        .expect("typed shadowing result");
    assert_eq!(
        shadowed.statement,
        AnalysisStatement::NotAnalyzable {
            reason: NotAnalyzableReason::PossibleTempShadowing
        }
    );
}

#[tokio::test]
#[ignore = "requires pnpm db:postgres"]
async fn result_mutation_live_virtual_key_drift_and_analysis_expiry() {
    let connection_id = "result-mutation-virtual-key";
    let spec = live_spec(connection_id);
    let admin = session_postgres::connect(&spec).await.expect("admin");
    let schema = unique_schema();
    admin
        .client
        .batch_execute(&format!(
            "CREATE SCHEMA {schema}; CREATE TABLE {schema}.rows (claimed_key text NOT NULL, body text NOT NULL); INSERT INTO {schema}.rows VALUES ('one', 'body')"
        ))
        .await
        .expect("fixture schema");

    let manager = ResultMutationManager::new();
    let key_pool = sqlx::sqlite::SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .expect("virtual-key database");
    sqlx::query(
        "CREATE TABLE virtual_keys (
           connection_id TEXT NOT NULL,
           schema TEXT NOT NULL,
           table_name TEXT NOT NULL,
           virtual_key TEXT NOT NULL,
           updated_at TEXT NOT NULL,
           PRIMARY KEY (connection_id, schema, table_name)
         )",
    )
    .execute(&key_pool)
    .await
    .expect("virtual-key table");
    let saved_key = VirtualKey {
        version: storage::VIRTUAL_KEY_VERSION,
        columns: vec!["claimed_key".into()],
    };
    storage::upsert_virtual_key(&key_pool, connection_id, &schema, "rows", &saved_key)
        .await
        .expect("save virtual key");
    assert_eq!(
        storage::read_virtual_key(&key_pool, connection_id, &schema, "rows")
            .await
            .expect("load virtual key"),
        Some(saved_key)
    );
    let lookup_pool = key_pool.clone();
    let lookup: VirtualKeyLookup = Arc::new(move |connection_id, schema, table| {
        let pool = lookup_pool.clone();
        Box::pin(async move {
            storage::read_virtual_key(&pool, &connection_id, &schema, &table)
                .await
                .map_err(|_| ResultMutationError::ConnectionLost)
        })
    });
    let first = manager
        .analyze(
            spec.clone(),
            statement_payload(
                connection_id,
                "virtual",
                1,
                format!("SELECT claimed_key, body FROM {schema}.rows"),
            ),
            lookup.clone(),
        )
        .await
        .expect("virtual-key analysis");
    assert_eq!(
        first.tables[0].identity.kind,
        MutationIdentityKind::VirtualKey
    );
    assert!(first.tables[0].updatable.allowed);

    admin
        .client
        .batch_execute(&format!(
            "ALTER TABLE {schema}.rows DROP COLUMN claimed_key"
        ))
        .await
        .expect("drop virtual-key column");
    let mut drift = statement_payload(
        connection_id,
        "virtual",
        2,
        format!("SELECT body FROM {schema}.rows"),
    );
    drift.refresh_structure = true;
    let drift = manager
        .analyze(spec.clone(), drift, lookup.clone())
        .await
        .expect("stale virtual key is classified");
    assert_eq!(
        drift.tables[0].updatable.reason,
        Some(CapabilityReason::InvalidVirtualKey)
    );
    assert!(!drift.tables[0].updatable.allowed);
    storage::clear_virtual_key(&key_pool, connection_id, &schema, "rows")
        .await
        .expect("clear virtual key");
    assert_eq!(
        storage::read_virtual_key(&key_pool, connection_id, &schema, "rows")
            .await
            .expect("load cleared virtual key"),
        None
    );

    let expiry_tab = "expiry";
    let oldest = manager
        .analyze(
            spec.clone(),
            relation_payload(
                connection_id,
                expiry_tab,
                1,
                "result_mutation_fixture",
                "generated_identity",
            ),
            empty_virtual_keys(),
        )
        .await
        .expect("oldest analysis");
    for request_id in 2..=17 {
        manager
            .analyze(
                spec.clone(),
                relation_payload(
                    connection_id,
                    expiry_tab,
                    request_id,
                    "result_mutation_fixture",
                    "generated_identity",
                ),
                empty_virtual_keys(),
            )
            .await
            .expect("fill analysis cache");
    }
    let expired = manager
        .preview(PreviewResultMutationsPayload {
            connection_id: connection_id.into(),
            tab_id: expiry_tab.into(),
            analysis_id: oldest.analysis_id,
            plan: MutationPlan {
                operations: Vec::new(),
            },
        })
        .await;
    assert_eq!(expired, Err(ResultMutationError::AnalysisExpired));

    cleanup_schema(&admin, &schema).await;
}

#[tokio::test]
#[ignore = "requires pnpm db:postgres"]
async fn result_mutation_live_catalog_drift_expires_before_dml() {
    let connection_id = "result-mutation-catalog-drift";
    let spec = live_spec(connection_id);
    let admin = session_postgres::connect(&spec).await.expect("admin");
    let schema = unique_schema();
    admin
        .client
        .batch_execute(&format!(
            "CREATE SCHEMA {schema};
             CREATE TABLE {schema}.rows (
               id integer PRIMARY KEY,
               body text NOT NULL DEFAULT 'default-body'
             );
             INSERT INTO {schema}.rows (id, body) VALUES (1, 'before')"
        ))
        .await
        .expect("fixture schema");
    let manager = ResultMutationManager::new();

    let type_analysis = manager
        .analyze(
            spec.clone(),
            relation_payload(connection_id, "type-drift", 1, &schema, "rows"),
            empty_virtual_keys(),
        )
        .await
        .expect("type analysis");
    admin
        .client
        .batch_execute(&format!(
            "ALTER TABLE {schema}.rows ALTER COLUMN body TYPE varchar(40)"
        ))
        .await
        .expect("alter type");
    let type_drift = manager
        .apply(
            spec.clone(),
            ApplyResultMutationsPayload {
                connection_id: connection_id.into(),
                tab_id: "type-drift".into(),
                request_id: 2,
                confirmed: false,
                analysis_id: type_analysis.analysis_id,
                plan: MutationPlan {
                    operations: vec![MutationOp::Update {
                        table: table(&schema, "rows"),
                        identity: vec![value("id", Some("1"))],
                        guards: vec![value("body", Some("before"))],
                        set: vec![value("body", Some("must-not-write"))],
                    }],
                },
            },
        )
        .await;
    assert_eq!(type_drift, Err(ResultMutationError::AnalysisExpired));

    let oid_analysis = manager
        .analyze(
            spec.clone(),
            relation_payload(connection_id, "oid-drift", 1, &schema, "rows"),
            empty_virtual_keys(),
        )
        .await
        .expect("oid analysis");
    admin
        .client
        .batch_execute(&format!(
            "DROP TABLE {schema}.rows;
             CREATE TABLE {schema}.rows (
               id integer PRIMARY KEY,
               body varchar(40) NOT NULL DEFAULT 'default-body'
             );
             INSERT INTO {schema}.rows (id, body) VALUES (1, 'before')"
        ))
        .await
        .expect("drop and recreate table");
    let oid_drift = manager
        .apply(
            spec.clone(),
            ApplyResultMutationsPayload {
                connection_id: connection_id.into(),
                tab_id: "oid-drift".into(),
                request_id: 2,
                confirmed: false,
                analysis_id: oid_analysis.analysis_id,
                plan: MutationPlan {
                    operations: vec![MutationOp::Update {
                        table: table(&schema, "rows"),
                        identity: vec![value("id", Some("1"))],
                        guards: vec![value("body", Some("before"))],
                        set: vec![value("body", Some("must-not-write"))],
                    }],
                },
            },
        )
        .await;
    assert_eq!(oid_drift, Err(ResultMutationError::AnalysisExpired));

    let default_analysis = manager
        .analyze(
            spec.clone(),
            relation_payload(connection_id, "default-drift", 1, &schema, "rows"),
            empty_virtual_keys(),
        )
        .await
        .expect("default analysis");
    admin
        .client
        .batch_execute(&format!(
            "ALTER TABLE {schema}.rows ALTER COLUMN body SET DEFAULT 'changed-default'"
        ))
        .await
        .expect("alter default");
    let default_drift = manager
        .apply(
            spec,
            ApplyResultMutationsPayload {
                connection_id: connection_id.into(),
                tab_id: "default-drift".into(),
                request_id: 2,
                confirmed: false,
                analysis_id: default_analysis.analysis_id,
                plan: MutationPlan {
                    operations: vec![MutationOp::Insert {
                        table: table(&schema, "rows"),
                        values: vec![value("id", Some("2"))],
                    }],
                },
            },
        )
        .await;
    assert_eq!(default_drift, Err(ResultMutationError::AnalysisExpired));
    let count: i64 = admin
        .client
        .query_one(&format!("SELECT count(*) FROM {schema}.rows"), &[])
        .await
        .expect("no drifted DML")
        .get(0);
    assert_eq!(count, 1);

    let identity_analysis = manager
        .analyze(
            live_spec(connection_id),
            relation_payload(connection_id, "identity-drift", 1, &schema, "rows"),
            empty_virtual_keys(),
        )
        .await
        .expect("identity analysis");
    admin
        .client
        .batch_execute(&format!(
            "ALTER TABLE {schema}.rows DROP CONSTRAINT rows_pkey;
             ALTER TABLE {schema}.rows ADD PRIMARY KEY (id)"
        ))
        .await
        .expect("replace identity index");
    let identity_drift = manager
        .apply(
            live_spec(connection_id),
            ApplyResultMutationsPayload {
                connection_id: connection_id.into(),
                tab_id: "identity-drift".into(),
                request_id: 2,
                confirmed: false,
                analysis_id: identity_analysis.analysis_id,
                plan: MutationPlan {
                    operations: vec![MutationOp::Update {
                        table: table(&schema, "rows"),
                        identity: vec![value("id", Some("1"))],
                        guards: vec![value("body", Some("before"))],
                        set: vec![value("body", Some("must-not-write"))],
                    }],
                },
            },
        )
        .await;
    assert_eq!(identity_drift, Err(ResultMutationError::AnalysisExpired));

    let locked_analysis = manager
        .analyze(
            live_spec(connection_id),
            relation_payload(connection_id, "locked-drift", 1, &schema, "rows"),
            empty_virtual_keys(),
        )
        .await
        .expect("locked drift analysis");
    admin
        .client
        .batch_execute(&format!(
            "BEGIN;
             LOCK TABLE {schema}.rows IN ACCESS EXCLUSIVE MODE;
             ALTER TABLE {schema}.rows ALTER COLUMN body TYPE varchar(80)"
        ))
        .await
        .expect("hold changed catalog behind a relation lock");
    let locked_manager = manager.clone();
    let locked_spec = live_spec(connection_id);
    let locked_schema = schema.clone();
    let locked_apply = tokio::spawn(async move {
        locked_manager
            .apply(
                locked_spec,
                ApplyResultMutationsPayload {
                    connection_id: connection_id.into(),
                    tab_id: "locked-drift".into(),
                    request_id: 2,
                    confirmed: false,
                    analysis_id: locked_analysis.analysis_id,
                    plan: MutationPlan {
                        operations: vec![MutationOp::Update {
                            table: table(&locked_schema, "rows"),
                            identity: vec![value("id", Some("1"))],
                            guards: vec![value("body", Some("before"))],
                            set: vec![value("body", Some("must-not-write"))],
                        }],
                    },
                },
            )
            .await
    });
    tokio::time::sleep(Duration::from_millis(200)).await;
    admin
        .client
        .batch_execute("COMMIT")
        .await
        .expect("release DDL");
    assert_eq!(
        locked_apply.await.expect("locked apply task"),
        Err(ResultMutationError::AnalysisExpired)
    );

    admin
        .client
        .batch_execute(&format!(
            "CREATE TABLE {schema}.unrelated (
               id integer PRIMARY KEY,
               note text NOT NULL
             );
             INSERT INTO {schema}.unrelated VALUES (1, 'unrelated')"
        ))
        .await
        .expect("unrelated fixture");
    let joined_analysis = manager
        .analyze(
            live_spec(connection_id),
            statement_payload(
                connection_id,
                "unrelated-drift",
                1,
                format!(
                    "SELECT a.id, a.body, b.id AS unrelated_id, b.note \
                     FROM {schema}.rows a JOIN {schema}.unrelated b ON true"
                ),
            ),
            empty_virtual_keys(),
        )
        .await
        .expect("joined analysis");
    admin
        .client
        .batch_execute(&format!(
            "BEGIN;
             ALTER TABLE {schema}.unrelated ALTER COLUMN note TYPE varchar(80)"
        ))
        .await
        .expect("hold unrelated catalog drift behind an exclusive lock");
    let unrelated_apply = tokio::time::timeout(
        Duration::from_secs(2),
        manager.apply(
            live_spec(connection_id),
            ApplyResultMutationsPayload {
                connection_id: connection_id.into(),
                tab_id: "unrelated-drift".into(),
                request_id: 2,
                confirmed: false,
                analysis_id: joined_analysis.analysis_id,
                plan: MutationPlan {
                    operations: vec![MutationOp::Update {
                        table: table(&schema, "rows"),
                        identity: vec![value("id", Some("1"))],
                        guards: vec![value("body", Some("before"))],
                        set: vec![value("body", Some("updated-with-unrelated-lock"))],
                    }],
                },
            },
        ),
    )
    .await
    .expect("unrelated lock cannot delay target apply")
    .expect("unrelated drift cannot expire target apply");
    assert_eq!(unrelated_apply.operations[0].rows_affected, 1);
    admin
        .client
        .batch_execute("COMMIT")
        .await
        .expect("release unrelated drift");
    let updated: String = admin
        .client
        .query_one(&format!("SELECT body FROM {schema}.rows WHERE id = 1"), &[])
        .await
        .expect("read targeted row")
        .get(0);
    assert_eq!(updated, "updated-with-unrelated-lock");

    cleanup_schema(&admin, &schema).await;
}

#[tokio::test]
#[ignore = "requires pnpm db:postgres"]
async fn result_mutation_live_preview_apply_conflicts_defaults_and_index_plan() {
    let connection_id = "result-mutation-apply";
    let spec = live_spec(connection_id);
    let admin = session_postgres::connect(&spec).await.expect("admin");
    let schema = unique_schema();
    admin
        .client
        .batch_execute(&format!(
            "CREATE SCHEMA {schema};
             CREATE TABLE {schema}.rows (id integer PRIMARY KEY, body text NOT NULL, note text DEFAULT 'default-note');
             INSERT INTO {schema}.rows VALUES (1, 'alpha', 'note-a'), (2, 'beta', 'note-b'), (3, 'gamma', 'note-c');
             CREATE TABLE {schema}.generated_rows (
               id bigint GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
               body text NOT NULL,
               note text DEFAULT 'default-note',
               body_length integer GENERATED ALWAYS AS (length(body)) STORED
             )"
        ))
        .await
        .expect("fixture schema");

    let manager = ResultMutationManager::new();
    let analysis = manager
        .analyze(
            spec.clone(),
            statement_payload(
                connection_id,
                "apply",
                1,
                format!("SELECT id, body, note FROM {schema}.rows"),
            ),
            empty_virtual_keys(),
        )
        .await
        .expect("analysis");
    let update = MutationOp::Update {
        table: table(&schema, "rows"),
        identity: vec![value("id", Some("1"))],
        guards: vec![value("body", Some("alpha"))],
        set: vec![value("body", Some("alpha-updated"))],
    };
    let preview = manager
        .preview(PreviewResultMutationsPayload {
            connection_id: connection_id.into(),
            tab_id: "apply".into(),
            analysis_id: analysis.analysis_id,
            plan: MutationPlan {
                operations: vec![update.clone()],
            },
        })
        .await
        .expect("preview");
    assert_eq!(
        preview.statements[0].sql,
        format!(
            "UPDATE \"{schema}\".\"rows\" SET \"body\" = ($1::text)::text WHERE \"id\" = ($2::text)::integer AND \"body\" = ($3::text)::text"
        )
    );
    assert_eq!(
        preview.statements[0].params,
        vec![
            DmlParam::Text {
                value: Some("alpha-updated".into())
            },
            DmlParam::Text {
                value: Some("1".into())
            },
            DmlParam::Text {
                value: Some("alpha".into())
            }
        ]
    );

    admin
        .client
        .batch_execute("SET enable_seqscan = off")
        .await
        .expect("prefer index plan");
    let explain_sql = format!("EXPLAIN (COSTS OFF) {}", preview.statements[0].sql);
    let explain = admin
        .client
        .query(
            &explain_sql,
            &[
                &Some("alpha-updated".to_string()) as &(dyn ToSql + Sync),
                &Some("1".to_string()),
                &Some("alpha".to_string()),
            ],
        )
        .await
        .expect("explain guarded update")
        .into_iter()
        .map(|row| row.get::<_, String>(0))
        .collect::<Vec<_>>()
        .join("\n");
    assert!(explain.contains("Index Scan"));
    assert!(!explain.contains("Seq Scan"));

    let happy_plan = MutationPlan {
        operations: vec![
            update,
            MutationOp::Insert {
                table: table(&schema, "rows"),
                values: vec![value("id", Some("4")), value("body", Some("inserted"))],
            },
            MutationOp::Delete {
                table: table(&schema, "rows"),
                identity: vec![value("id", Some("2"))],
                guards: vec![
                    value("id", Some("2")),
                    value("body", Some("beta")),
                    value("note", Some("note-b")),
                ],
            },
            MutationOp::Insert {
                table: table(&schema, "rows"),
                values: vec![value("id", Some("5")), value("body", Some("alpha"))],
            },
        ],
    };
    let applied = manager
        .apply(
            spec.clone(),
            ApplyResultMutationsPayload {
                connection_id: connection_id.into(),
                tab_id: "apply".into(),
                request_id: 2,
                confirmed: false,
                analysis_id: analysis.analysis_id,
                plan: happy_plan,
            },
        )
        .await
        .expect("happy-path apply");
    assert_eq!(
        applied
            .operations
            .iter()
            .map(|operation| operation.rows_affected)
            .collect::<Vec<_>>(),
        vec![1, 1, 1, 1]
    );

    let conflict_analysis = manager
        .analyze(
            spec.clone(),
            statement_payload(
                connection_id,
                "conflict",
                1,
                format!("SELECT id, body, note FROM {schema}.rows"),
            ),
            empty_virtual_keys(),
        )
        .await
        .expect("conflict analysis");
    admin
        .client
        .batch_execute(&format!(
            "UPDATE {schema}.rows SET body = 'concurrent' WHERE id = 3"
        ))
        .await
        .expect("concurrent update");
    let conflict = manager
        .apply(
            spec.clone(),
            ApplyResultMutationsPayload {
                connection_id: connection_id.into(),
                tab_id: "conflict".into(),
                request_id: 2,
                confirmed: false,
                analysis_id: conflict_analysis.analysis_id,
                plan: MutationPlan {
                    operations: vec![
                        MutationOp::Update {
                            table: table(&schema, "rows"),
                            identity: vec![value("id", Some("1"))],
                            guards: vec![value("body", Some("alpha-updated"))],
                            set: vec![value("body", Some("must-roll-back"))],
                        },
                        MutationOp::Update {
                            table: table(&schema, "rows"),
                            identity: vec![value("id", Some("3"))],
                            guards: vec![value("body", Some("gamma"))],
                            set: vec![value("body", Some("must-conflict"))],
                        },
                    ],
                },
            },
        )
        .await;
    assert_eq!(conflict, Err(ResultMutationError::Conflict { op_index: 1 }));
    let rolled_back: String = admin
        .client
        .query_one(&format!("SELECT body FROM {schema}.rows WHERE id = 1"), &[])
        .await
        .expect("rollback proof")
        .get(0);
    assert_eq!(rolled_back, "alpha-updated");

    let delete_analysis = manager
        .analyze(
            spec.clone(),
            statement_payload(
                connection_id,
                "delete-conflict",
                1,
                format!("SELECT id, body, note FROM {schema}.rows"),
            ),
            empty_virtual_keys(),
        )
        .await
        .expect("delete analysis");
    admin
        .client
        .batch_execute(&format!(
            "UPDATE {schema}.rows SET note = 'changed' WHERE id = 5"
        ))
        .await
        .expect("concurrent delete guard change");
    let delete_conflict = manager
        .apply(
            spec.clone(),
            ApplyResultMutationsPayload {
                connection_id: connection_id.into(),
                tab_id: "delete-conflict".into(),
                request_id: 2,
                confirmed: false,
                analysis_id: delete_analysis.analysis_id,
                plan: MutationPlan {
                    operations: vec![MutationOp::Delete {
                        table: table(&schema, "rows"),
                        identity: vec![value("id", Some("5"))],
                        guards: vec![
                            value("id", Some("5")),
                            value("body", Some("alpha")),
                            value("note", Some("default-note")),
                        ],
                    }],
                },
            },
        )
        .await;
    assert_eq!(
        delete_conflict,
        Err(ResultMutationError::Conflict { op_index: 0 })
    );

    let generated = manager
        .analyze(
            spec.clone(),
            statement_payload(
                connection_id,
                "generated",
                1,
                format!("SELECT id, body, note, body_length FROM {schema}.generated_rows"),
            ),
            empty_virtual_keys(),
        )
        .await
        .expect("generated analysis");
    let generated_rejection = manager
        .preview(PreviewResultMutationsPayload {
            connection_id: connection_id.into(),
            tab_id: "generated".into(),
            analysis_id: generated.analysis_id,
            plan: MutationPlan {
                operations: vec![MutationOp::Insert {
                    table: table(&schema, "generated_rows"),
                    values: vec![value("body_length", Some("9"))],
                }],
            },
        })
        .await;
    assert_eq!(
        generated_rejection,
        Err(ResultMutationError::InvalidPlan {
            reason: InvalidPlanReason::GeneratedColumn
        })
    );
    let defaults_plan = MutationPlan {
        operations: vec![MutationOp::Insert {
            table: table(&schema, "generated_rows"),
            values: vec![value("body", Some("defaults"))],
        }],
    };
    let defaults_preview = manager
        .preview(PreviewResultMutationsPayload {
            connection_id: connection_id.into(),
            tab_id: "generated".into(),
            analysis_id: generated.analysis_id,
            plan: defaults_plan.clone(),
        })
        .await
        .expect("default omission preview");
    assert_eq!(
        defaults_preview.statements[0].sql,
        format!("INSERT INTO \"{schema}\".\"generated_rows\" (\"body\") VALUES (($1::text)::text)")
    );
    manager
        .apply(
            spec,
            ApplyResultMutationsPayload {
                connection_id: connection_id.into(),
                tab_id: "generated".into(),
                request_id: 2,
                confirmed: false,
                analysis_id: generated.analysis_id,
                plan: defaults_plan,
            },
        )
        .await
        .expect("default omission apply");
    let generated_row = admin
        .client
        .query_one(
            &format!(
                "SELECT id > 0, note, body_length FROM {schema}.generated_rows WHERE body = 'defaults'"
            ),
            &[],
        )
        .await
        .expect("generated/default values");
    assert!(generated_row.get::<_, bool>(0));
    assert_eq!(generated_row.get::<_, String>(1), "default-note");
    assert_eq!(generated_row.get::<_, i32>(2), 8);

    cleanup_schema(&admin, &schema).await;
}

#[tokio::test]
#[ignore = "requires pnpm db:postgres"]
async fn result_mutation_live_virtual_identity_ctid_and_lock_timeout() {
    let connection_id = "result-mutation-guards";
    let spec = live_spec(connection_id);
    let admin = session_postgres::connect(&spec).await.expect("admin");
    let schema = unique_schema();
    admin
        .client
        .batch_execute(&format!(
            "CREATE SCHEMA {schema};
             CREATE TABLE {schema}.duplicates (claimed_key text NOT NULL, body text NOT NULL);
             INSERT INTO {schema}.duplicates VALUES ('duplicate', 'same'), ('duplicate', 'same');
             CREATE TABLE {schema}.heap_rows (body text NOT NULL);
             INSERT INTO {schema}.heap_rows VALUES ('before');
             CREATE TABLE {schema}.locked_rows (id integer PRIMARY KEY, body text NOT NULL);
             INSERT INTO {schema}.locked_rows VALUES (1, 'before')"
        ))
        .await
        .expect("fixture schema");
    let manager = ResultMutationManager::new();

    let keys = Arc::new(Mutex::new(HashMap::new()));
    keys.lock().await.insert(
        (connection_id.into(), schema.clone(), "duplicates".into()),
        VirtualKey {
            version: 1,
            columns: vec!["claimed_key".into()],
        },
    );
    let duplicate_analysis = manager
        .analyze(
            spec.clone(),
            statement_payload(
                connection_id,
                "duplicates",
                1,
                format!("SELECT claimed_key, body FROM {schema}.duplicates"),
            ),
            virtual_key_lookup(keys),
        )
        .await
        .expect("virtual-key analysis");
    let not_unique = manager
        .apply(
            spec.clone(),
            ApplyResultMutationsPayload {
                connection_id: connection_id.into(),
                tab_id: "duplicates".into(),
                request_id: 2,
                confirmed: false,
                analysis_id: duplicate_analysis.analysis_id,
                plan: MutationPlan {
                    operations: vec![MutationOp::Update {
                        table: table(&schema, "duplicates"),
                        identity: vec![value("claimed_key", Some("duplicate"))],
                        guards: vec![
                            value("claimed_key", Some("duplicate")),
                            value("body", Some("same")),
                        ],
                        set: vec![value("body", Some("changed"))],
                    }],
                },
            },
        )
        .await;
    assert_eq!(
        not_unique,
        Err(ResultMutationError::IdentityNotUnique { op_index: 0 })
    );
    let unchanged: i64 = admin
        .client
        .query_one(
            &format!("SELECT count(*) FROM {schema}.duplicates WHERE body = 'same'"),
            &[],
        )
        .await
        .expect("identity rollback")
        .get(0);
    assert_eq!(unchanged, 2);

    let ctid_analysis = manager
        .analyze(
            spec.clone(),
            statement_payload(
                connection_id,
                "ctid",
                1,
                format!("SELECT ctid, body FROM {schema}.heap_rows"),
            ),
            empty_virtual_keys(),
        )
        .await
        .expect("ctid analysis");
    assert_eq!(
        ctid_analysis.tables[0].identity.kind,
        MutationIdentityKind::CtidFallback
    );
    let ctid: String = admin
        .client
        .query_one(&format!("SELECT ctid::text FROM {schema}.heap_rows"), &[])
        .await
        .expect("ctid")
        .get(0);
    manager
        .apply(
            spec.clone(),
            ApplyResultMutationsPayload {
                connection_id: connection_id.into(),
                tab_id: "ctid".into(),
                request_id: 2,
                confirmed: false,
                analysis_id: ctid_analysis.analysis_id,
                plan: MutationPlan {
                    operations: vec![MutationOp::Update {
                        table: table(&schema, "heap_rows"),
                        identity: vec![value("ctid", Some(&ctid))],
                        guards: vec![value("ctid", Some(&ctid)), value("body", Some("before"))],
                        set: vec![value("body", Some("after"))],
                    }],
                },
            },
        )
        .await
        .expect("ctid update");
    let current_ctid: String = admin
        .client
        .query_one(&format!("SELECT ctid::text FROM {schema}.heap_rows"), &[])
        .await
        .expect("current ctid")
        .get(0);
    admin
        .client
        .batch_execute(&format!(
            "UPDATE {schema}.heap_rows SET body = 'concurrent'"
        ))
        .await
        .expect("concurrent heap update");
    let stale_ctid = manager
        .apply(
            spec.clone(),
            ApplyResultMutationsPayload {
                connection_id: connection_id.into(),
                tab_id: "ctid".into(),
                request_id: 3,
                confirmed: false,
                analysis_id: ctid_analysis.analysis_id,
                plan: MutationPlan {
                    operations: vec![MutationOp::Update {
                        table: table(&schema, "heap_rows"),
                        identity: vec![value("ctid", Some(&current_ctid))],
                        guards: vec![
                            value("ctid", Some(&current_ctid)),
                            value("body", Some("after")),
                        ],
                        set: vec![value("body", Some("must-not-write"))],
                    }],
                },
            },
        )
        .await;
    assert_eq!(
        stale_ctid,
        Err(ResultMutationError::Conflict { op_index: 0 })
    );

    let lock_analysis = manager
        .analyze(
            spec.clone(),
            statement_payload(
                connection_id,
                "lock",
                1,
                format!("SELECT id, body FROM {schema}.locked_rows"),
            ),
            empty_virtual_keys(),
        )
        .await
        .expect("lock analysis");
    admin
        .client
        .batch_execute(&format!(
            "BEGIN; UPDATE {schema}.locked_rows SET body = 'held' WHERE id = 1"
        ))
        .await
        .expect("hold row lock");
    let lock_timeout = manager
        .apply(
            spec,
            ApplyResultMutationsPayload {
                connection_id: connection_id.into(),
                tab_id: "lock".into(),
                request_id: 2,
                confirmed: false,
                analysis_id: lock_analysis.analysis_id,
                plan: MutationPlan {
                    operations: vec![MutationOp::Update {
                        table: table(&schema, "locked_rows"),
                        identity: vec![value("id", Some("1"))],
                        guards: vec![value("body", Some("before"))],
                        set: vec![value("body", Some("after"))],
                    }],
                },
            },
        )
        .await;
    admin.client.batch_execute("ROLLBACK").await.ok();
    assert_eq!(
        lock_timeout,
        Err(ResultMutationError::LockTimeout { op_index: 0 })
    );

    cleanup_schema(&admin, &schema).await;
}

#[tokio::test]
#[ignore = "requires pnpm db:postgres"]
async fn result_mutation_live_cancel_teardown_rollback_and_recovery() {
    let connection_id = "result-mutation-cancel";
    let spec = live_spec(connection_id);
    let admin = session_postgres::connect(&spec).await.expect("admin");
    let schema = unique_schema();
    admin
        .client
        .batch_execute(&format!(
            "CREATE SCHEMA {schema};
             CREATE TABLE {schema}.rows (id integer PRIMARY KEY, body text NOT NULL);
             INSERT INTO {schema}.rows VALUES (1, 'before');
             CREATE FUNCTION {schema}.delay_mutation() RETURNS trigger LANGUAGE plpgsql AS $$
             BEGIN PERFORM pg_sleep(30); RETURN NEW; END $$;
             CREATE TRIGGER delay_mutation BEFORE UPDATE ON {schema}.rows
             FOR EACH ROW EXECUTE FUNCTION {schema}.delay_mutation()"
        ))
        .await
        .expect("fixture schema");
    let manager = ResultMutationManager::new();
    let analysis = manager
        .analyze(
            spec.clone(),
            statement_payload(
                connection_id,
                "cancel",
                1,
                format!("SELECT id, body FROM {schema}.rows"),
            ),
            empty_virtual_keys(),
        )
        .await
        .expect("cancel analysis");
    let payload = ApplyResultMutationsPayload {
        connection_id: connection_id.into(),
        tab_id: "cancel".into(),
        request_id: 2,
        confirmed: false,
        analysis_id: analysis.analysis_id,
        plan: MutationPlan {
            operations: vec![MutationOp::Update {
                table: table(&schema, "rows"),
                identity: vec![value("id", Some("1"))],
                guards: vec![value("body", Some("before"))],
                set: vec![value("body", Some("cancelled"))],
            }],
        },
    };
    let apply_manager = manager.clone();
    let apply_spec = spec.clone();
    let apply = tokio::spawn(async move { apply_manager.apply(apply_spec, payload).await });
    tokio::time::sleep(Duration::from_millis(300)).await;
    let cancelled = manager.cancel_tab(connection_id, "cancel").await;
    assert!(cancelled.cancel_requested);
    assert_eq!(
        apply.await.expect("cancel task"),
        Err(ResultMutationError::Cancelled)
    );
    let unchanged: String = admin
        .client
        .query_one(&format!("SELECT body FROM {schema}.rows WHERE id = 1"), &[])
        .await
        .expect("cancel rollback")
        .get(0);
    assert_eq!(unchanged, "before");

    let dropped_manager = manager.clone();
    let dropped_spec = spec.clone();
    let dropped_payload = ApplyResultMutationsPayload {
        connection_id: connection_id.into(),
        tab_id: "cancel".into(),
        request_id: 3,
        confirmed: false,
        analysis_id: analysis.analysis_id,
        plan: MutationPlan {
            operations: vec![MutationOp::Update {
                table: table(&schema, "rows"),
                identity: vec![value("id", Some("1"))],
                guards: vec![value("body", Some("before"))],
                set: vec![value("body", Some("dropped"))],
            }],
        },
    };
    let dropped_apply =
        tokio::spawn(async move { dropped_manager.apply(dropped_spec, dropped_payload).await });
    tokio::time::sleep(Duration::from_millis(300)).await;
    dropped_apply.abort();
    let executor = manager
        .existing_executor(connection_id)
        .await
        .expect("executor remains owned by manager");
    tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            let notified = executor.notify.notified();
            tokio::pin!(notified);
            if executor.state.lock().await.active.is_none() {
                break;
            }
            notified.await;
        }
    })
    .await
    .expect("dropped apply finalizes");
    let unchanged_after_drop: String = admin
        .client
        .query_one(&format!("SELECT body FROM {schema}.rows WHERE id = 1"), &[])
        .await
        .expect("dropped apply rollback")
        .get(0);
    assert_eq!(unchanged_after_drop, "before");

    admin
        .client
        .batch_execute(&format!("DROP TRIGGER delay_mutation ON {schema}.rows"))
        .await
        .expect("remove delay trigger");
    manager
        .apply(
            spec.clone(),
            ApplyResultMutationsPayload {
                connection_id: connection_id.into(),
                tab_id: "cancel".into(),
                request_id: 4,
                confirmed: false,
                analysis_id: analysis.analysis_id,
                plan: MutationPlan {
                    operations: vec![MutationOp::Update {
                        table: table(&schema, "rows"),
                        identity: vec![value("id", Some("1"))],
                        guards: vec![value("body", Some("before"))],
                        set: vec![value("body", Some("recovered"))],
                    }],
                },
            },
        )
        .await
        .expect("socket usable after cancellation");

    admin
        .client
        .batch_execute(&format!(
            "CREATE TRIGGER delay_mutation BEFORE UPDATE ON {schema}.rows
             FOR EACH ROW EXECUTE FUNCTION {schema}.delay_mutation()"
        ))
        .await
        .expect("restore pre-commit delay trigger");
    let teardown_before_payload = ApplyResultMutationsPayload {
        connection_id: connection_id.into(),
        tab_id: "teardown-before".into(),
        request_id: 2,
        confirmed: false,
        analysis_id: manager
            .analyze(
                spec.clone(),
                statement_payload(
                    connection_id,
                    "teardown-before",
                    1,
                    format!("SELECT id, body FROM {schema}.rows"),
                ),
                empty_virtual_keys(),
            )
            .await
            .expect("pre-commit teardown analysis")
            .analysis_id,
        plan: MutationPlan {
            operations: vec![MutationOp::Update {
                table: table(&schema, "rows"),
                identity: vec![value("id", Some("1"))],
                guards: vec![value("body", Some("recovered"))],
                set: vec![value("body", Some("must-rollback"))],
            }],
        },
    };
    let before_manager = manager.clone();
    let before_spec = spec.clone();
    let teardown_before = tokio::spawn(async move {
        before_manager
            .apply(before_spec, teardown_before_payload)
            .await
    });
    tokio::time::sleep(Duration::from_millis(300)).await;
    manager.begin_connection_teardown(connection_id).await;
    assert_eq!(
        teardown_before.await.expect("pre-commit teardown task"),
        Err(ResultMutationError::ConnectionClosing)
    );
    manager.end_connection_teardown(connection_id).await;
    let after_precommit_teardown: String = admin
        .client
        .query_one(&format!("SELECT body FROM {schema}.rows WHERE id = 1"), &[])
        .await
        .expect("pre-commit teardown rollback")
        .get(0);
    assert_eq!(after_precommit_teardown, "recovered");

    admin
        .client
        .batch_execute(&format!(
            "DROP TRIGGER delay_mutation ON {schema}.rows;
             CREATE FUNCTION {schema}.delay_commit() RETURNS trigger LANGUAGE plpgsql AS $$
             BEGIN PERFORM pg_sleep(1); RETURN NEW; END $$;
             CREATE CONSTRAINT TRIGGER delay_mutation AFTER UPDATE ON {schema}.rows
             DEFERRABLE INITIALLY DEFERRED
             FOR EACH ROW EXECUTE FUNCTION {schema}.delay_commit()"
        ))
        .await
        .expect("install deferred commit trigger");
    let teardown_analysis = manager
        .analyze(
            spec.clone(),
            statement_payload(
                connection_id,
                "teardown",
                1,
                format!("SELECT id, body FROM {schema}.rows"),
            ),
            empty_virtual_keys(),
        )
        .await
        .expect("teardown analysis");
    let teardown_payload = ApplyResultMutationsPayload {
        connection_id: connection_id.into(),
        tab_id: "teardown".into(),
        request_id: 2,
        confirmed: false,
        analysis_id: teardown_analysis.analysis_id,
        plan: MutationPlan {
            operations: vec![MutationOp::Update {
                table: table(&schema, "rows"),
                identity: vec![value("id", Some("1"))],
                guards: vec![value("body", Some("recovered"))],
                set: vec![value("body", Some("teardown"))],
            }],
        },
    };
    let teardown_manager = manager.clone();
    let teardown_spec = spec.clone();
    let teardown_apply = tokio::spawn(async move {
        teardown_manager
            .apply(teardown_spec, teardown_payload)
            .await
    });
    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            let executor = manager
                .existing_executor(connection_id)
                .await
                .expect("post-admission executor");
            let admitted = executor
                .state
                .lock()
                .await
                .active
                .as_ref()
                .is_some_and(super::ActiveRequest::commit_admitted);
            if admitted {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("COMMIT admission observed");
    manager.begin_connection_teardown(connection_id).await;
    let committed = teardown_apply
        .await
        .expect("post-admission teardown task")
        .expect("COMMIT remains authoritative after admission");
    assert_eq!(committed.operations[0].rows_affected, 1);
    manager.end_connection_teardown(connection_id).await;
    let after_teardown: String = admin
        .client
        .query_one(&format!("SELECT body FROM {schema}.rows WHERE id = 1"), &[])
        .await
        .expect("teardown rollback")
        .get(0);
    assert_eq!(after_teardown, "teardown");

    admin
        .client
        .batch_execute(&format!("DROP TRIGGER delay_mutation ON {schema}.rows"))
        .await
        .expect("remove teardown trigger");
    let recovered_analysis = manager
        .analyze(
            spec,
            statement_payload(
                connection_id,
                "recovered",
                1,
                format!("SELECT id, body FROM {schema}.rows"),
            ),
            empty_virtual_keys(),
        )
        .await
        .expect("manager usable after teardown fence");
    assert_eq!(recovered_analysis.statement, AnalysisStatement::Analyzed);

    cleanup_schema(&admin, &schema).await;
}

#[tokio::test]
#[serial_test::serial]
#[ignore = "requires pnpm db:postgres"]
async fn safety_live_apply_strict_confirmation_and_audit() {
    let connection_id = format!("safety-apply-{}", uuid::Uuid::new_v4().simple());
    let schema = unique_schema();
    let (_directory, state) = crate::test_app_state().await;
    let strict_connection = crate::StoredConnection::PostgreSQL(crate::PgStoredConnection {
        organization: Default::default(),
        id: connection_id.clone(),
        name: "Safety apply".into(),
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
    });
    crate::commands::connections::save_connection_inner(&state, strict_connection.clone())
        .await
        .expect("save strict connection through command core");
    let default_spec = ResolvedPostgresConnectSpec::from_connection(&strict_connection)
        .expect("strict Postgres spec");
    let admin = session_postgres::connect(&default_spec)
        .await
        .expect("admin session");
    admin
        .client
        .batch_execute(&format!(
            "CREATE SCHEMA {schema}; \
             CREATE TABLE {schema}.rows(id integer PRIMARY KEY, body text NOT NULL); \
             INSERT INTO {schema}.rows VALUES (1, 'before')"
        ))
        .await
        .expect("fixture");

    let analysis = state
        .result_mutations
        .analyze(
            default_spec.clone(),
            relation_payload(&connection_id, "safety", 1, &schema, "rows"),
            empty_virtual_keys(),
        )
        .await
        .expect("analysis");
    let plan = MutationPlan {
        operations: vec![MutationOp::Update {
            table: table(&schema, "rows"),
            identity: vec![value("id", Some("1"))],
            guards: vec![value("body", Some("before"))],
            set: vec![value("body", Some("after"))],
        }],
    };
    let payload = |confirmed| ApplyResultMutationsPayload {
        connection_id: connection_id.clone(),
        tab_id: "safety".into(),
        request_id: 2,
        analysis_id: analysis.analysis_id,
        plan: plan.clone(),
        confirmed,
    };
    assert!(matches!(
        crate::commands::result_mutation::apply_result_mutations_inner(&state, payload(false))
            .await,
        Err(ResultMutationError::PolicyNeedsConfirmation { .. })
    ));
    let refused_row: String = admin
        .client
        .query_one(&format!("SELECT body FROM {schema}.rows WHERE id = 1"), &[])
        .await
        .expect("read refused row")
        .get(0);
    assert_eq!(refused_row, "before");
    assert!(storage::read_safety_overrides(&state.pool, &connection_id)
        .await
        .expect("audit rows after refusal")
        .is_empty());
    assert!(storage::read_connection_by_id(&state.pool, &connection_id)
        .await
        .expect("read refused connection")
        .expect("stored connection")
        .last_activity_at()
        .is_none());

    let applied =
        crate::commands::result_mutation::apply_result_mutations_inner(&state, payload(true))
            .await
            .expect("confirmed apply");
    assert_eq!(applied.operations[0].rows_affected, 1);
    let stored: String = admin
        .client
        .query_one(&format!("SELECT body FROM {schema}.rows WHERE id = 1"), &[])
        .await
        .expect("read applied row")
        .get(0);
    assert_eq!(stored, "after");

    let audit_rows = storage::read_safety_overrides(&state.pool, &connection_id)
        .await
        .expect("audit rows");
    assert_eq!(audit_rows.len(), 1);
    assert_eq!(audit_rows[0].command, "apply_result_mutations");
    assert_eq!(audit_rows[0].classes, vec!["dml"]);
    assert!(storage::read_connection_by_id(&state.pool, &connection_id)
        .await
        .expect("read active connection")
        .expect("stored connection")
        .last_activity_at()
        .is_some());

    state
        .result_mutations
        .close_connection(&connection_id)
        .await;
    cleanup_schema(&admin, &schema).await;
}
