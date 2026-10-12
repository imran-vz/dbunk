//! End-to-end SQLite seeds against temporary database files.
use super::*;

struct Fixture {
    _dir: tempfile::TempDir,
    stored: crate::SqliteStoredConnection,
}

async fn fixture(schema: &str) -> Fixture {
    let dir = tempfile::tempdir().expect("temp dir");
    let path = dir.path().join("seed.db");
    let options = SqliteConnectOptions::new()
        .filename(&path)
        .create_if_missing(true);
    let mut connection = SqliteConnection::connect_with(&options)
        .await
        .expect("create database");
    for statement in schema.split(';').filter(|s| !s.trim().is_empty()) {
        connection.execute(statement).await.expect(statement);
    }
    connection.close().await.expect("close");
    let stored = serde_json::from_value(serde_json::json!({
        "id": "local",
        "name": "Local file",
        "database": path.to_str().expect("utf-8 path"),
        "role": "",
    }))
    .expect("stored connection");
    Fixture { _dir: dir, stored }
}

fn target() -> TableSeedConnection {
    TableSeedConnection {
        engine: TableSeedEngine::Sqlite,
        connection_name: "Local file".into(),
        host: "seed.db".into(),
        port: 0,
        database: "main".into(),
        user: String::new(),
        environment: "Development".into(),
        safe_mode: "Inherit".into(),
        read_only: false,
    }
}

fn intent(table: &str, rows: u32, columns: Vec<TableSeedColumnSpec>) -> TableSeedIntent {
    TableSeedIntent::new(
        TableSeedEndpoint {
            connection_id: "local".into(),
            schema: "main".into(),
            table: table.into(),
        },
        rows,
        Some(42),
        columns,
    )
    .expect("intent")
}

async fn query(fixture: &Fixture, sql: &str) -> Vec<Vec<Option<String>>> {
    let options = SqliteConnectOptions::new().filename(&fixture.stored.database);
    let mut connection = SqliteConnection::connect_with(&options)
        .await
        .expect("open");
    let rows = connection.fetch_all(sql).await.expect(sql);
    let values = rows
        .iter()
        .map(|row| {
            (0..row.len())
                .map(|i| {
                    row.try_get::<Option<String>, _>(i)
                        .or_else(|_| {
                            row.try_get::<Option<i64>, _>(i)
                                .map(|v| v.map(|v| v.to_string()))
                        })
                        .or_else(|_| {
                            row.try_get::<Option<f64>, _>(i)
                                .map(|v| v.map(|v| v.to_string()))
                        })
                        .expect("text, integer or real")
                })
                .collect()
        })
        .collect();
    connection.close().await.expect("close");
    values
}

async fn seed(fixture: &Fixture, intent: TableSeedIntent) -> (Arc<Plan>, Execution) {
    let control = Arc::new(Control::default());
    let plan = Arc::new(
        inspect(&fixture.stored, intent, target(), &control)
            .await
            .unwrap_or_else(|f| panic!("inspect: {:?}", f.error)),
    );
    assert_eq!(plan.issue, None);
    let execution = execute(&fixture.stored, plan.clone(), &control).await;
    (plan, execution)
}

const SHOP: &str = "
    CREATE TABLE customers (id INTEGER PRIMARY KEY, name TEXT NOT NULL);
    INSERT INTO customers (name) VALUES ('Ada'), ('Grace'), ('Linus');
    CREATE TABLE orders (
        id INTEGER PRIMARY KEY,
        customer_id INTEGER NOT NULL REFERENCES customers(id),
        email TEXT NOT NULL UNIQUE,
        total NUMERIC(8,2) NOT NULL,
        label TEXT GENERATED ALWAYS AS ('#' || id) VIRTUAL
    )";

#[tokio::test]
async fn seeds_rows_with_parents_and_skips_database_columns() {
    let fixture = fixture(SHOP).await;
    let (plan, execution) = seed(&fixture, intent("orders", 250, vec![])).await;
    assert_eq!(execution.outcome, TableSeedOutcome::Completed { rows: 250 });
    let description = plan.description.as_ref().expect("description");
    assert_eq!(description.connection.engine, TableSeedEngine::Sqlite);
    assert_eq!(
        description.table_fingerprint.as_ref().map(String::len),
        Some(64)
    );
    let id = plan.columns.iter().find(|c| c.name == "id").unwrap();
    assert_eq!(id.action, TableSeedColumnAction::Default, "rowid alias");
    let label = plan.columns.iter().find(|c| c.name == "label").unwrap();
    assert_eq!(label.action, TableSeedColumnAction::Default, "generated");

    let counts = query(
        &fixture,
        "SELECT count(*), count(DISTINCT email) FROM orders",
    )
    .await;
    assert_eq!(counts[0], vec![Some("250".into()), Some("250".into())]);
    let orphans = query(
        &fixture,
        "SELECT count(*) FROM orders WHERE customer_id NOT IN (SELECT id FROM customers)",
    )
    .await;
    assert_eq!(orphans[0][0].as_deref(), Some("0"));
}

#[tokio::test]
async fn the_same_seed_writes_the_same_rows() {
    let first = fixture(SHOP).await;
    let second = fixture(SHOP).await;
    seed(&first, intent("orders", 40, vec![])).await;
    seed(&second, intent("orders", 40, vec![])).await;
    let sql = "SELECT customer_id, email, total FROM orders ORDER BY id";
    assert_eq!(query(&first, sql).await, query(&second, sql).await);
}

#[tokio::test]
async fn a_rejected_row_rolls_the_whole_seed_back() {
    let fixture = fixture(
        "CREATE TABLE notes (id INTEGER PRIMARY KEY, body TEXT NOT NULL CHECK (length(body) < 3))",
    )
    .await;
    let (_, execution) = seed(&fixture, intent("notes", 50, vec![])).await;
    assert_eq!(execution.outcome, TableSeedOutcome::RolledBack);
    let failure = execution.failure.expect("failure");
    assert_eq!(failure.error, TableSeedError::Database);
    // SQLITE_CONSTRAINT_CHECK
    assert_eq!(
        failure.diagnostic.and_then(|d| d.sqlstate).as_deref(),
        Some("275")
    );
    assert_eq!(
        query(&fixture, "SELECT count(*) FROM notes").await[0][0].as_deref(),
        Some("0")
    );
}

#[tokio::test]
async fn a_changed_table_is_refused_inside_the_transaction() {
    let fixture = fixture(SHOP).await;
    let control = Arc::new(Control::default());
    let plan = Arc::new(
        inspect(
            &fixture.stored,
            intent("orders", 10, vec![]),
            target(),
            &control,
        )
        .await
        .unwrap_or_else(|f| panic!("{:?}", f.error)),
    );
    query(&fixture, "ALTER TABLE orders ADD COLUMN note TEXT").await;
    let execution = execute(&fixture.stored, plan, &control).await;
    assert_eq!(execution.outcome, TableSeedOutcome::RolledBack);
    assert_eq!(
        execution.failure.unwrap().error,
        TableSeedError::TargetChanged
    );
    assert_eq!(
        query(&fixture, "SELECT count(*) FROM orders").await[0][0].as_deref(),
        Some("0")
    );
}

#[tokio::test]
async fn an_empty_parent_fails_before_writing() {
    let fixture = fixture(SHOP).await;
    query(&fixture, "DELETE FROM customers").await;
    let (_, execution) = seed(&fixture, intent("orders", 10, vec![])).await;
    assert_eq!(execution.outcome, TableSeedOutcome::RolledBack);
    let failure = execution.failure.unwrap();
    assert_eq!(failure.error, TableSeedError::EmptyParent);
    let diagnostic = failure.diagnostic.unwrap();
    assert_eq!(diagnostic.parent_table.as_deref(), Some("customers"));
}

#[tokio::test]
async fn a_cancelled_seed_writes_nothing() {
    let fixture = fixture(SHOP).await;
    let control = Arc::new(Control::default());
    let plan = Arc::new(
        inspect(
            &fixture.stored,
            intent("orders", 1000, vec![]),
            target(),
            &control,
        )
        .await
        .unwrap_or_else(|f| panic!("{:?}", f.error)),
    );
    control.cancel();
    let execution = execute(&fixture.stored, plan, &control).await;
    assert_ne!(
        execution.outcome,
        TableSeedOutcome::Completed { rows: 1000 }
    );
    assert_eq!(execution.failure.unwrap().error, TableSeedError::Cancelled);
    assert_eq!(
        query(&fixture, "SELECT count(*) FROM orders").await[0][0].as_deref(),
        Some("0")
    );
}

#[tokio::test]
async fn views_virtual_tables_and_other_schemas_are_refused() {
    let fixture = fixture("CREATE TABLE t (a TEXT); CREATE VIEW v AS SELECT a FROM t").await;
    let control = Arc::new(Control::default());
    for (schema, table) in [("main", "v"), ("temp", "t"), ("main", "missing")] {
        let mut intent = intent(table, 1, vec![]);
        intent.endpoint.schema = schema.into();
        let error = inspect(&fixture.stored, intent, target(), &control)
            .await
            .err()
            .map(|f| f.error);
        assert_eq!(
            error,
            Some(TableSeedError::UnsupportedTarget),
            "{schema}.{table}"
        );
    }
}

#[tokio::test]
async fn length_ranges_and_locales_reach_the_rows() {
    let fixture =
        fixture("CREATE TABLE people (id INTEGER PRIMARY KEY, email VARCHAR(80) NOT NULL)").await;
    let intent = intent(
        "people",
        100,
        vec![TableSeedColumnSpec {
            column: "email".into(),
            source: TableSeedSource::Auto { generator: None },
            null_rate: None,
            length: Some(TableSeedLength {
                min: Some(40),
                max: Some(60),
            }),
        }],
    )
    .with_generation(vec![TableSeedLocale::Ja], TableSeedRealism::FictionalSafe)
    .unwrap();
    let (_, execution) = seed(&fixture, intent).await;
    assert_eq!(execution.outcome, TableSeedOutcome::Completed { rows: 100 });
    for row in query(&fixture, "SELECT email FROM people").await {
        let email = row[0].clone().unwrap();
        assert!((40..=60).contains(&email.len()), "{email}");
        assert!(email.contains('@'));
    }
}
