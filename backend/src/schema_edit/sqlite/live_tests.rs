//! Capture, plan and apply against real SQLite files (Plan 033 lane B
//! verification): the round trip `capture(apply(plan(baseline, draft)))`
//! matches the draft for every operation kind, and every failure mode
//! leaves the table unchanged.
use super::planner::{expected_table, REBUILD_PREFIX};
use super::verify::compare_tables;
use super::*;
use crate::schema_edit::{
    plan_review, CheckModel, DefaultValue, Deferrable, Editability, ForeignKeyModel, Generated,
    Identity, IndexTarget, KeyModel, NoteKind, PlanOptions, ReferentialAction, SchemaEditFailure,
    SchemaEngine, SqlFragment,
};
use sqlx::sqlite::SqliteConnectOptions;
use sqlx::Connection;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

struct Db {
    dir: tempfile::TempDir,
    connection: SqliteConnection,
    version: String,
}

async fn open(setup: &str) -> Db {
    let dir = tempfile::tempdir().unwrap();
    let options = SqliteConnectOptions::new()
        .filename(dir.path().join("edit.db"))
        .create_if_missing(true)
        .foreign_keys(false);
    let mut connection = SqliteConnection::connect_with(&options).await.unwrap();
    sqlx::raw_sql(setup).execute(&mut connection).await.unwrap();
    let version = sqlx::query_scalar::<_, String>("SELECT sqlite_version()")
        .fetch_one(&mut connection)
        .await
        .unwrap();
    Db {
        dir,
        connection,
        version,
    }
}

impl Db {
    fn path(&self) -> String {
        self.dir.path().join("edit.db").to_string_lossy().into_owned()
    }

    async fn run(&mut self, sql: &str) {
        sqlx::raw_sql(sql)
            .execute(&mut self.connection)
            .await
            .unwrap_or_else(|error| panic!("{sql}: {error}"));
    }

    async fn capture(&mut self, table: &str) -> SchemaBaseline {
        capture(&mut self.connection, "main", table, &self.version)
            .await
            .unwrap()
    }

    /// Single text column of every row.
    async fn texts(&mut self, sql: &str) -> Vec<String> {
        sqlx::query_scalar::<_, String>(sql)
            .persistent(false)
            .fetch_all(&mut self.connection)
            .await
            .unwrap_or_else(|error| panic!("{sql}: {error}"))
    }

    async fn number(&mut self, sql: &str) -> i64 {
        sqlx::query_scalar::<_, i64>(sql)
            .persistent(false)
            .fetch_one(&mut self.connection)
            .await
            .unwrap_or_else(|error| panic!("{sql}: {error}"))
    }

    async fn apply(&mut self, baseline: &SchemaBaseline, draft: &SchemaDraft) -> SchemaEditOutcome {
        let preview = plan_review(baseline, draft, &PlanOptions::default())
            .unwrap_or_else(|error| panic!("plan: {error:?}"));
        execute(&mut self.connection, baseline, draft, &preview).await
    }

    /// Applies, asserts success and that the re-captured table is the
    /// draft (modulo origins), and returns the new baseline.
    async fn round_trip(&mut self, baseline: &SchemaBaseline, draft: &SchemaDraft) -> SchemaBaseline {
        let outcome = self.apply(baseline, draft).await;
        assert!(outcome.applied(), "{outcome:?}");
        let recaptured = self.capture(&draft.table.name).await;
        assert_eq!(recaptured.editable, Editability::Editable);
        compare_tables(&expected_table(baseline, draft), &recaptured.table)
            .unwrap_or_else(|detail| panic!("round trip: {detail}"));
        recaptured
    }

    /// The table is exactly as captured in `baseline`.
    async fn unchanged(&mut self, baseline: &SchemaBaseline) {
        let now = self.capture(&baseline.table.name).await;
        assert_eq!(now.table, baseline.table);
        assert_eq!(now.drift, baseline.drift);
        let leftover = self
            .number(&format!(
                "SELECT count(*) FROM sqlite_master WHERE name = '{REBUILD_PREFIX}{}'",
                baseline.table.name
            ))
            .await;
        assert_eq!(leftover, 0);
    }

    async fn pragma(&mut self, name: &str) -> i64 {
        self.number(&format!("PRAGMA {name}")).await
    }
}

type Edit = Box<dyn Fn(&mut SchemaDraft)>;

fn edit(change: impl Fn(&mut SchemaDraft) + 'static) -> Edit {
    Box::new(change)
}

fn column(draft: &SchemaDraft, name: &str) -> Origin {
    draft
        .table
        .columns
        .iter()
        .find(|column| column.name == name)
        .unwrap_or_else(|| panic!("no column {name}"))
        .origin
}

const USERS: &str = "
    CREATE TABLE orgs (id INTEGER PRIMARY KEY, name TEXT NOT NULL);
    INSERT INTO orgs VALUES (1, 'acme'), (2, 'initech');
    CREATE TABLE users (
        id INTEGER PRIMARY KEY AUTOINCREMENT,
        email TEXT NOT NULL COLLATE NOCASE UNIQUE,
        name TEXT DEFAULT 'anon',
        score REAL DEFAULT (0.5 * 2) CHECK (score >= 0),
        created TEXT DEFAULT CURRENT_TIMESTAMP,
        org_id INTEGER REFERENCES orgs(id) ON DELETE CASCADE,
        upper_name TEXT GENERATED ALWAYS AS (upper(name)) VIRTUAL,
        CONSTRAINT users_email_len CHECK (length(email) > 3)
    );
    CREATE INDEX users_name_idx ON users (name DESC) WHERE name IS NOT NULL;
    CREATE INDEX users_lower ON users (lower(email));
    CREATE TRIGGER users_touch AFTER UPDATE OF name ON users BEGIN SELECT 1; END;
    CREATE VIEW named AS SELECT name FROM users;
    INSERT INTO users (email, name, score, org_id) VALUES
        ('ann@x.io', 'Ann', 1.5, 1), ('bo@x.io', NULL, 2, 2);
";

#[tokio::test]
async fn capture_reads_the_definition_and_cross_checks_it() {
    let mut db = open(USERS).await;
    let baseline = db.capture("users").await;
    assert_eq!(baseline.editable, Editability::Editable);
    assert_eq!(baseline.engine, SchemaEngine::Sqlite);
    let features = baseline.server.features;
    assert!(features.rename_column && features.drop_column && features.transactional_ddl);
    assert!(features.enforced_check && !features.column_reorder);
    assert_eq!(baseline.estimated_rows, Some(2));

    let table = &baseline.table;
    let names: Vec<&str> = table.columns.iter().map(|c| c.name.as_str()).collect();
    assert_eq!(
        names,
        vec!["id", "email", "name", "score", "created", "org_id", "upper_name"]
    );
    for (cid, column) in table.columns.iter().enumerate() {
        assert_eq!(column.origin, column_origin(cid as i64));
    }
    assert_eq!(table.columns[0].data_type, "INTEGER");
    assert_eq!(table.columns[0].identity, Identity::AutoIncrement);
    assert_eq!(table.columns[1].collation.as_deref(), Some("NOCASE"));
    assert!(!table.columns[1].nullable);
    assert_eq!(table.columns[2].default, DefaultValue::Literal("anon".into()));
    assert_eq!(
        table.columns[3].default,
        DefaultValue::Expression(SqlFragment::new("0.5 * 2"))
    );
    assert_eq!(
        table.columns[4].default,
        DefaultValue::Expression(SqlFragment::new("CURRENT_TIMESTAMP"))
    );
    assert_eq!(table.columns[6].data_type, "TEXT");
    assert_eq!(
        table.columns[6].generated,
        Some(Generated {
            expression: SqlFragment::new("upper(name)"),
            stored: false,
        })
    );

    assert_eq!(
        table.primary_key,
        Some(KeyModel {
            origin: object_origin(TAG_PRIMARY_KEY, 0),
            name: None,
            columns: vec![column_origin(0)],
        })
    );
    assert_eq!(
        table.uniques,
        vec![KeyModel {
            origin: object_origin(TAG_UNIQUE, 0),
            name: None,
            columns: vec![column_origin(1)],
        }]
    );
    assert_eq!(table.foreign_keys.len(), 1);
    let key = &table.foreign_keys[0];
    assert_eq!(key.origin, object_origin(TAG_FOREIGN_KEY, 0));
    assert_eq!(key.columns, vec![column_origin(5)]);
    assert_eq!(key.referenced_table, "orgs");
    assert_eq!(key.referenced_columns, vec!["id".to_owned()]);
    assert_eq!(key.on_delete, ReferentialAction::Cascade);
    let checks: Vec<(Option<&str>, &str)> = table
        .checks
        .iter()
        .map(|c| (c.name.as_deref(), c.expression.as_str()))
        .collect();
    assert_eq!(
        checks,
        vec![
            (None, "score >= 0"),
            (Some("users_email_len"), "length(email) > 3")
        ]
    );
    assert_eq!(table.checks[1].origin, object_origin(TAG_CHECK, 1));
    let indexes: Vec<&str> = table.indexes.iter().map(|i| i.name.as_str()).collect();
    assert_eq!(indexes, vec!["users_lower", "users_name_idx"]);
    assert_eq!(
        table.indexes[0].members[0].target,
        IndexTarget::Expression(SqlFragment::new("lower(email)"))
    );
    let by_name = &table.indexes[1];
    assert_eq!(by_name.origin, object_origin(TAG_INDEX, 1));
    assert_eq!(by_name.members[0].target, IndexTarget::Column(column_origin(2)));
    assert!(by_name.members[0].descending);
    assert_eq!(
        by_name.predicate.as_ref().map(SqlFragment::as_str),
        Some("name IS NOT NULL")
    );

    let dependents: Vec<(&str, &str)> = baseline
        .dependents
        .iter()
        .map(|d| (d.kind.as_str(), d.name.as_str()))
        .collect();
    assert_eq!(dependents, vec![("trigger", "users_touch"), ("view", "named")]);
    assert!(baseline.dependents[0]
        .definition
        .as_deref()
        .is_some_and(|sql| sql.starts_with("CREATE TRIGGER users_touch")));

    // Capturing twice gives the same baseline (stable ids, same drift).
    assert_eq!(db.capture("users").await, baseline);

    let orgs = db.capture("orgs").await;
    assert_eq!(orgs.inbound.len(), 1);
    assert_eq!(orgs.inbound[0].table, "users");
    assert_eq!(orgs.inbound[0].columns, vec!["id".to_owned()]);
}

#[tokio::test]
async fn capture_marks_what_it_cannot_reproduce_read_only() {
    let mut db = open(
        "CREATE TABLE conflict (a TEXT UNIQUE ON CONFLICT REPLACE);
         CREATE TABLE commaless (a INT, b INT, PRIMARY KEY (a) UNIQUE (b));
         CREATE TABLE counter (id INTEGER PRIMARY KEY AUTOINCREMENT);
         CREATE VIEW v AS SELECT 1 AS one;
         CREATE VIRTUAL TABLE docs USING fts5(body);
         CREATE TABLE kv (k TEXT PRIMARY KEY, v BLOB) STRICT, WITHOUT ROWID;",
    )
    .await;
    for (table, hint) in [
        ("conflict", "ON CONFLICT"),
        ("commaless", "cannot verify"),
        ("v", "Views"),
        ("docs", "irtual"),
        ("docs_content", "virtual table"),
        ("sqlite_sequence", "internal"),
    ] {
        let baseline = db.capture(table).await;
        match &baseline.editable {
            Editability::ReadOnly { reasons } => assert!(
                reasons.iter().any(|reason| reason.contains(hint)),
                "{table}: {reasons:?}"
            ),
            Editability::Editable => panic!("{table} should be read-only"),
        }
        // A read-only baseline still shows its columns, and refuses plans.
        assert!(!baseline.table.columns.is_empty(), "{table}");
        let draft = SchemaDraft::from_baseline(&baseline);
        assert!(matches!(
            plan_review(&baseline, &draft, &PlanOptions::default()),
            Err(crate::schema_edit::PlanError::ReadOnly { .. })
        ));
    }
    let kv = db.capture("kv").await;
    assert_eq!(kv.editable, Editability::Editable);
    assert!(kv.table.options.strict && kv.table.options.without_rowid);
    assert!(capture(&mut db.connection, "main", "missing", &db.version)
        .await
        .is_err());
}

const ITEMS: &str = "
    CREATE TABLE items (
        id INTEGER PRIMARY KEY,
        title TEXT NOT NULL,
        note TEXT,
        price REAL CHECK (price >= 0)
    );
    CREATE INDEX items_title ON items (title);
    INSERT INTO items (title, note, price) VALUES ('a', 'x', 1.0), ('b', NULL, 2.5);
";

#[tokio::test]
async fn fast_path_changes_round_trip() {
    let mut db = open(ITEMS).await;
    let baseline = db.capture("items").await;
    let mut draft = SchemaDraft::from_baseline(&baseline);
    let qty = draft.add_column("qty", "INTEGER");
    let added = draft.table.column_mut(qty).unwrap();
    added.nullable = false;
    added.default = DefaultValue::Expression(SqlFragment::new("0"));
    let title = column_origin(1);
    draft.table.column_mut(title).unwrap().name = "name".into();
    draft.table.indexes[0].name = "items_name".into();
    draft.table.name = "products".into();
    let preview = plan_review(&baseline, &draft, &PlanOptions::default()).unwrap();
    assert!(!preview.notes.iter().any(|n| n.kind == NoteKind::Rebuild));
    let products = db.round_trip(&baseline, &draft).await;
    assert_eq!(products.table.indexes[0].name, "items_name");
    assert_eq!(
        products.table.indexes[0].members[0].target,
        IndexTarget::Column(column_origin(1))
    );
    assert_eq!(
        db.texts("SELECT name || ':' || qty FROM products ORDER BY id").await,
        vec!["a:0", "b:0"]
    );

    // Drop a column nothing mentions.
    let mut draft = SchemaDraft::from_baseline(&products);
    draft.remove_column(column(&draft, "note"));
    let preview = plan_review(&products, &draft, &PlanOptions::default()).unwrap();
    assert!(!preview.notes.iter().any(|n| n.kind == NoteKind::Rebuild));
    db.round_trip(&products, &draft).await;
    assert_eq!(
        db.texts("SELECT name || ':' || price FROM products ORDER BY id").await,
        vec!["a:1.0", "b:2.5"]
    );
}

const PEOPLE: &str = "
    CREATE TABLE orgs (id INTEGER PRIMARY KEY, name TEXT);
    INSERT INTO orgs VALUES (1, 'a'), (2, 'b');
    CREATE TABLE people (
        id INTEGER PRIMARY KEY,
        name TEXT,
        age TEXT,
        org_id INTEGER,
        nick TEXT UNIQUE
    );
    CREATE INDEX people_name ON people (name);
    INSERT INTO people (name, age, org_id, nick) VALUES ('Ann', '31', 1, 'a'), ('Bo', '42', 2, 'b');
";

#[tokio::test]
async fn every_rebuild_operation_round_trips_and_keeps_the_rows() {
    let mut db = open(PEOPLE).await;
    let edits: Vec<(&str, Edit)> = vec![
        (
            "type",
            edit(|d: &mut SchemaDraft| {
                let age = column(d, "age");
                d.table.column_mut(age).unwrap().data_type = "INTEGER".into();
            }),
        ),
        (
            "not null",
            edit(|d: &mut SchemaDraft| {
                let name = column(d, "name");
                d.table.column_mut(name).unwrap().nullable = false;
            }),
        ),
        (
            "default",
            edit(|d: &mut SchemaDraft| {
                let name = column(d, "name");
                d.table.column_mut(name).unwrap().default = DefaultValue::Literal("anon".into());
            }),
        ),
        (
            "collation",
            edit(|d: &mut SchemaDraft| {
                let nick = column(d, "nick");
                d.table.column_mut(nick).unwrap().collation = Some("NOCASE".into());
            }),
        ),
        (
            "check",
            edit(|d: &mut SchemaDraft| {
                let origin = d.new_origin();
                d.table.checks.push(CheckModel {
                    origin,
                    name: Some("adult".into()),
                    expression: SqlFragment::new("age >= 18"),
                    enforced: true,
                });
            }),
        ),
        ("drop unique", edit(|d: &mut SchemaDraft| d.table.uniques.clear())),
        (
            "foreign key",
            edit(|d: &mut SchemaDraft| {
                let origin = d.new_origin();
                let org = column(d, "org_id");
                d.table.foreign_keys.push(ForeignKeyModel {
                    origin,
                    name: Some("people_org".into()),
                    columns: vec![org],
                    referenced_schema: None,
                    referenced_table: "orgs".into(),
                    referenced_columns: vec!["id".into()],
                    on_update: ReferentialAction::NoAction,
                    on_delete: ReferentialAction::Cascade,
                    deferrable: Deferrable::InitiallyDeferred,
                });
            }),
        ),
        (
            "composite unique",
            edit(|d: &mut SchemaDraft| {
                let origin = d.new_origin();
                let columns = vec![column(d, "name"), column(d, "org_id")];
                d.table.uniques.push(KeyModel {
                    origin,
                    name: Some("people_name_org".into()),
                    columns,
                });
            }),
        ),
        (
            "primary key name",
            edit(|d: &mut SchemaDraft| {
                d.table.primary_key.as_mut().unwrap().name = Some("people_pk".into())
            }),
        ),
        (
            "reorder",
            edit(|d: &mut SchemaDraft| {
                let org = column(d, "org_id");
                d.move_column(org, 1);
            }),
        ),
        (
            "stored generated column",
            edit(|d: &mut SchemaDraft| {
                let label = d.add_column("label", "TEXT");
                d.table.column_mut(label).unwrap().generated = Some(Generated {
                    expression: SqlFragment::new("upper(name)"),
                    stored: true,
                });
            }),
        ),
        ("strict on", edit(|d: &mut SchemaDraft| d.table.options.strict = true)),
        ("strict off", edit(|d: &mut SchemaDraft| d.table.options.strict = false)),
        (
            "without rowid on",
            edit(|d: &mut SchemaDraft| d.table.options.without_rowid = true),
        ),
        (
            "without rowid off",
            edit(|d: &mut SchemaDraft| d.table.options.without_rowid = false),
        ),
        (
            "rename with a rebuild",
            edit(|d: &mut SchemaDraft| {
                let nick = column(d, "nick");
                d.table.column_mut(nick).unwrap().name = "handle".into();
                let name = column(d, "name");
                d.table.column_mut(name).unwrap().data_type = "varchar(80)".into();
            }),
        ),
    ];
    let mut baseline = db.capture("people").await;
    for (label, edit) in &edits {
        let mut draft = SchemaDraft::from_baseline(&baseline);
        edit(&mut draft);
        let preview = plan_review(&baseline, &draft, &PlanOptions::default())
            .unwrap_or_else(|error| panic!("{label}: {error:?}"));
        assert!(
            preview.notes.iter().any(|n| n.kind == NoteKind::Rebuild),
            "{label} should rebuild"
        );
        let outcome = execute(&mut db.connection, &baseline, &draft, &preview).await;
        assert!(outcome.applied(), "{label}: {outcome:?}");
        let recaptured = db.capture("people").await;
        assert_eq!(recaptured.editable, Editability::Editable, "{label}");
        compare_tables(&expected_table(&baseline, &draft), &recaptured.table)
            .unwrap_or_else(|detail| panic!("{label}: {detail}"));
        assert_eq!(
            db.texts("SELECT name FROM people ORDER BY id").await,
            vec!["Ann", "Bo"],
            "{label}"
        );
        baseline = recaptured;
    }
    // The type change converted the stored values.
    assert_eq!(
        db.texts("SELECT typeof(age) FROM people ORDER BY id").await,
        vec!["integer", "integer"]
    );
    assert_eq!(
        db.texts("SELECT label FROM people ORDER BY id").await,
        vec!["ANN", "BO"]
    );
    // The index followed every rebuild.
    assert_eq!(
        db.number("SELECT count(*) FROM sqlite_master WHERE type = 'index' AND name = 'people_name'")
            .await,
        1
    );
}

#[tokio::test]
async fn a_rebuild_preserves_rowids_and_the_autoincrement_counter() {
    let mut db = open(
        "CREATE TABLE log (id INTEGER PRIMARY KEY AUTOINCREMENT, msg TEXT);
         INSERT INTO log (msg) VALUES ('a'), ('b'), ('c');
         DELETE FROM log WHERE id = 3;
         CREATE TABLE plain (v TEXT, w TEXT);
         INSERT INTO plain VALUES ('x', '1'), ('y', '2'), ('z', '3');
         DELETE FROM plain WHERE v = 'y';",
    )
    .await;
    let log = db.capture("log").await;
    let mut draft = SchemaDraft::from_baseline(&log);
    draft.table.column_mut(column_origin(1)).unwrap().data_type = "varchar(10)".into();
    db.round_trip(&log, &draft).await;
    db.run("INSERT INTO log (msg) VALUES ('d')").await;
    // id 3 was used once; AUTOINCREMENT never hands it out again.
    assert_eq!(
        db.texts("SELECT id || msg FROM log ORDER BY id").await,
        vec!["1a", "2b", "4d"]
    );

    let plain = db.capture("plain").await;
    let mut draft = SchemaDraft::from_baseline(&plain);
    draft.table.column_mut(column_origin(1)).unwrap().data_type = "INTEGER".into();
    let preview = plan_review(&plain, &draft, &PlanOptions::default()).unwrap();
    assert!(preview.notes[0].text.contains("Rowids are preserved"));
    db.round_trip(&plain, &draft).await;
    assert_eq!(
        db.texts("SELECT rowid || v FROM plain ORDER BY rowid").await,
        vec!["1x", "3z"]
    );
}

#[tokio::test]
async fn drift_between_review_and_apply_dispatches_nothing() {
    let mut db = open(ITEMS).await;
    let baseline = db.capture("items").await;
    let mut draft = SchemaDraft::from_baseline(&baseline);
    draft.add_column("qty", "INTEGER");
    let preview = plan_review(&baseline, &draft, &PlanOptions::default()).unwrap();
    db.run("CREATE TABLE other (x)").await;
    let outcome = execute(&mut db.connection, &baseline, &draft, &preview).await;
    assert_eq!(
        outcome,
        SchemaEditOutcome::NotDispatched {
            reason: SchemaEditFailure::BaselineChanged
        }
    );
    let now = db.capture("items").await;
    assert_eq!(now.table, baseline.table);
}

#[tokio::test]
async fn a_rebuild_failing_mid_copy_leaves_the_table_unchanged() {
    let mut db = open(ITEMS).await;
    let baseline = db.capture("items").await;
    let mut draft = SchemaDraft::from_baseline(&baseline);
    // `note` holds a NULL, so copying into NOT NULL fails.
    draft.table.column_mut(column_origin(2)).unwrap().nullable = false;
    let outcome = db.apply(&baseline, &draft).await;
    match &outcome {
        SchemaEditOutcome::RolledBack {
            reason: SchemaEditFailure::Database { message, .. },
        } => assert!(message.contains("NOT NULL"), "{message}"),
        other => panic!("expected a rolled back database error, got {other:?}"),
    }
    db.unchanged(&baseline).await;
    assert_eq!(
        db.texts("SELECT title FROM items ORDER BY id").await,
        vec!["a", "b"]
    );
    assert_eq!(db.pragma("legacy_alter_table").await, 0);
    assert_eq!(db.pragma("foreign_keys").await, 0);
}

#[tokio::test]
async fn foreign_key_violations_roll_back_and_foreign_keys_are_restored() {
    let mut db = open(
        "CREATE TABLE parent (id INTEGER PRIMARY KEY);
         CREATE TABLE child (id INTEGER PRIMARY KEY, parent_id INTEGER REFERENCES parent(id), note TEXT);
         INSERT INTO parent VALUES (1);
         INSERT INTO child VALUES (1, 1, 'ok'), (2, 99, 'orphan');
         PRAGMA foreign_keys = ON;",
    )
    .await;
    assert_eq!(db.pragma("foreign_keys").await, 1);
    let baseline = db.capture("child").await;
    let mut draft = SchemaDraft::from_baseline(&baseline);
    draft.table.column_mut(column_origin(2)).unwrap().data_type = "varchar(20)".into();
    let outcome = db.apply(&baseline, &draft).await;
    assert_eq!(
        outcome,
        SchemaEditOutcome::RolledBack {
            reason: SchemaEditFailure::ForeignKeyViolation
        }
    );
    db.unchanged(&baseline).await;
    assert_eq!(db.pragma("foreign_keys").await, 1);
    assert_eq!(db.pragma("legacy_alter_table").await, 0);

    db.run("DELETE FROM child WHERE id = 2").await;
    let baseline = db.capture("child").await;
    let mut draft = SchemaDraft::from_baseline(&baseline);
    draft.table.column_mut(column_origin(2)).unwrap().data_type = "varchar(20)".into();
    db.round_trip(&baseline, &draft).await;
    assert_eq!(db.pragma("foreign_keys").await, 1);
    assert_eq!(db.pragma("legacy_alter_table").await, 0);

    // The fast path restores it too.
    let baseline = db.capture("child").await;
    let mut draft = SchemaDraft::from_baseline(&baseline);
    draft.add_column("extra", "TEXT");
    db.round_trip(&baseline, &draft).await;
    assert_eq!(db.pragma("foreign_keys").await, 1);
}

#[tokio::test]
async fn an_open_user_transaction_is_refused_and_left_open() {
    let mut db = open(ITEMS).await;
    let baseline = db.capture("items").await;
    let mut draft = SchemaDraft::from_baseline(&baseline);
    draft.add_column("qty", "INTEGER");
    db.run("BEGIN; INSERT INTO items (title) VALUES ('pending');").await;
    let outcome = db.apply(&baseline, &draft).await;
    assert_eq!(
        outcome,
        SchemaEditOutcome::NotDispatched {
            reason: SchemaEditFailure::OpenTransaction
        }
    );
    // The user's transaction is still theirs to commit.
    db.run("COMMIT").await;
    assert_eq!(
        db.texts("SELECT title FROM items ORDER BY id").await,
        vec!["a", "b", "pending"]
    );
    assert_eq!(db.capture("items").await.table, baseline.table);
}

#[tokio::test]
async fn cancellation_through_the_progress_handler_rolls_back() {
    let mut db = open(
        "CREATE TABLE big (id INTEGER PRIMARY KEY, v TEXT);
         WITH RECURSIVE n(x) AS (SELECT 1 UNION ALL SELECT x + 1 FROM n WHERE x < 20000)
         INSERT INTO big (v) SELECT 'row ' || x FROM n;",
    )
    .await;
    let cancel = Arc::new(AtomicBool::new(false));
    {
        let flag = cancel.clone();
        let mut handle = db.connection.lock_handle().await.unwrap();
        handle.set_progress_handler(1_000, move || !flag.load(Ordering::Acquire));
    }
    let baseline = db.capture("big").await;
    let mut draft = SchemaDraft::from_baseline(&baseline);
    draft.table.column_mut(column_origin(1)).unwrap().data_type = "varchar(20)".into();
    let preview = plan_review(&baseline, &draft, &PlanOptions::default()).unwrap();
    cancel.store(true, Ordering::Release);
    let outcome = execute(&mut db.connection, &baseline, &draft, &preview).await;
    cancel.store(false, Ordering::Release);
    assert!(
        matches!(
            outcome,
            SchemaEditOutcome::RolledBack {
                reason: SchemaEditFailure::Cancelled
            } | SchemaEditOutcome::NotDispatched {
                reason: SchemaEditFailure::Cancelled
            }
        ),
        "{outcome:?}"
    );
    db.unchanged(&baseline).await;
    assert_eq!(db.number("SELECT count(*) FROM big").await, 20_000);
}

#[tokio::test]
async fn a_rebuild_recreates_triggers_and_keeps_views_working() {
    let mut db = open(
        "CREATE TABLE audit (msg TEXT);
         CREATE TABLE notes (id INTEGER PRIMARY KEY, body TEXT, rank TEXT);
         CREATE TRIGGER notes_audit AFTER INSERT ON notes BEGIN INSERT INTO audit VALUES (new.body); END;
         CREATE VIEW note_bodies AS SELECT body FROM notes;
         INSERT INTO notes (body, rank) VALUES ('one', '1');",
    )
    .await;
    let baseline = db.capture("notes").await;
    let mut draft = SchemaDraft::from_baseline(&baseline);
    draft.table.column_mut(column_origin(2)).unwrap().data_type = "INTEGER".into();
    let preview = plan_review(&baseline, &draft, &PlanOptions::default()).unwrap();
    assert!(preview
        .statements
        .last()
        .unwrap()
        .sql
        .starts_with("CREATE TRIGGER \"main\".notes_audit"));
    db.round_trip(&baseline, &draft).await;
    db.run("INSERT INTO notes (body, rank) VALUES ('two', 2)").await;
    assert_eq!(
        db.texts("SELECT msg FROM audit ORDER BY rowid").await,
        vec!["one", "two"]
    );
    assert_eq!(
        db.texts("SELECT body FROM note_bodies ORDER BY body").await,
        vec!["one", "two"]
    );
}

#[tokio::test]
async fn the_session_worker_captures_and_applies() {
    use crate::backend::sqlite_session::{SqliteSession, SqliteSessionConfig};
    use crate::schema_edit::SessionApply;
    use crate::{ConnectionPolicy, Environment, SafeMode};
    use std::time::Duration;

    let db = open(ITEMS).await;
    let path = db.path();
    let config = |read_only: bool| SqliteSessionConfig {
        connection_id: "schema-edit-test".into(),
        path: path.clone(),
        read_only,
        policy: crate::safety::policy::resolve_policy(ConnectionPolicy {
            environment: Environment::Development,
            safe_mode: SafeMode::Inherit,
            read_only,
        }),
        audit: None,
    };
    let session = SqliteSession::open(config(false)).await.unwrap();
    let baseline = session
        .schema_baseline(session.ticket(), "main".into(), "items".into())
        .await
        .unwrap();
    assert_eq!(baseline.editable, Editability::Editable);
    let mut draft = SchemaDraft::from_baseline(&baseline);
    draft.add_column("qty", "INTEGER");
    let preview = plan_review(&baseline, &draft, &PlanOptions::default()).unwrap();
    let result = session
        .apply_schema_edit(
            session.ticket(),
            baseline.clone(),
            draft.clone(),
            preview.clone(),
            false,
        )
        .await
        .unwrap();
    assert!(
        matches!(result, SessionApply::Finished(SchemaEditOutcome::Applied { .. })),
        "{result:?}"
    );
    let after = session
        .schema_baseline(session.ticket(), "main".into(), "items".into())
        .await
        .unwrap();
    assert_eq!(after.table.columns.last().unwrap().name, "qty");
    session.close(Duration::from_secs(5)).await.unwrap();

    let read_only = SqliteSession::open(config(true)).await.unwrap();
    let result = read_only
        .apply_schema_edit(read_only.ticket(), after.clone(), draft, preview, false)
        .await
        .unwrap();
    assert!(matches!(result, SessionApply::Blocked(_)), "{result:?}");
    read_only.close(Duration::from_secs(5)).await.unwrap();
}
