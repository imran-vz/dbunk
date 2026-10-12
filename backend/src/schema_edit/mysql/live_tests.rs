//! Live round trips against the owned test-db MySQL fixture
//! (`make -C infrastructure/test-db mysql`, MySQL 8.4 on 127.0.0.1:13306).
use super::{capture, execute};
use crate::schema_edit::*;
use sqlx::mysql::{MySqlConnectOptions, MySqlConnection};
use sqlx::{ConnectOptions, Connection, Executor};

fn options() -> MySqlConnectOptions {
    MySqlConnectOptions::new()
        .host("127.0.0.1")
        .port(13306)
        .username("dbunk")
        .password("dbunk")
        .database("dbunk_demo")
}

async fn connect() -> MySqlConnection {
    options().connect().await.expect("MySQL fixture")
}

async fn version(connection: &mut MySqlConnection) -> String {
    sqlx::query_scalar::<_, String>("SELECT VERSION()")
        .fetch_one(&mut *connection)
        .await
        .unwrap()
}

async fn fresh(connection: &mut MySqlConnection, table: &str) {
    connection
        .execute(format!("DROP TABLE IF EXISTS `{table}`, `{table}_orgs`").as_str())
        .await
        .unwrap();
    connection
        .execute(
            format!(
                "CREATE TABLE `{table}_orgs` (id BIGINT PRIMARY KEY) ENGINE=InnoDB;
                 CREATE TABLE `{table}` (
                     id BIGINT NOT NULL AUTO_INCREMENT,
                     email VARCHAR(100) CHARACTER SET utf8mb4 COLLATE utf8mb4_bin NOT NULL COMMENT 'login',
                     name VARCHAR(50) DEFAULT 'anon',
                     score INT NOT NULL DEFAULT 0,
                     updated_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP ON UPDATE CURRENT_TIMESTAMP,
                     org_id BIGINT NULL,
                     PRIMARY KEY (id),
                     UNIQUE KEY `{table}_email_key` (email),
                     KEY `{table}_name_idx` (name),
                     CONSTRAINT `{table}_score_check` CHECK (score >= 0)
                 ) ENGINE=InnoDB COMMENT='people';
                 INSERT INTO `{table}_orgs` VALUES (1);
                 INSERT INTO `{table}` (email, name, score, org_id) VALUES ('a@x', 'Ann', 3, 1), ('b@x', NULL, 5, NULL);"
            )
            .as_str(),
        )
        .await
        .unwrap();
}

fn origin(draft: &SchemaDraft, name: &str) -> Origin {
    draft
        .table
        .columns
        .iter()
        .find(|c| c.name == name)
        .unwrap_or_else(|| panic!("column {name}"))
        .origin
}

#[tokio::test]
#[ignore = "requires make -C infrastructure/test-db mysql"]
async fn round_trip_applies_every_kind_and_keeps_rows() {
    let table = "schema_edit_rt";
    let mut connection = connect().await;
    let server = version(&mut connection).await;
    fresh(&mut connection, table).await;
    let baseline = capture(&mut connection, &server, "dbunk_demo", table)
        .await
        .unwrap();
    assert_eq!(
        baseline.editable,
        Editability::Editable,
        "{:?}",
        baseline.editable
    );

    let mut draft = SchemaDraft::from_baseline(&baseline);
    let name = origin(&draft, "name");
    let score = origin(&draft, "score");
    let org = origin(&draft, "org_id");
    let column = draft.table.column_mut(name).unwrap();
    column.name = "full_name".into();
    column.data_type = "varchar(80)".into();
    draft.table.column_mut(score).unwrap().data_type = "bigint".into();
    let nick = draft.add_column("nick", "varchar(20)");
    draft.table.column_mut(nick).unwrap().default = DefaultValue::Literal("x".into());
    draft.move_column(nick, 2);
    draft.table.comment = Some("people v2".into());
    let fk = draft.new_origin();
    draft.table.foreign_keys.push(ForeignKeyModel {
        origin: fk,
        name: Some(format!("{table}_org_fk")),
        columns: vec![org],
        referenced_schema: None,
        referenced_table: format!("{table}_orgs"),
        referenced_columns: vec!["id".into()],
        on_update: ReferentialAction::NoAction,
        on_delete: ReferentialAction::SetNull,
        deferrable: Deferrable::Never,
    });
    let index = draft.new_origin();
    draft.table.indexes.push(IndexModel {
        origin: index,
        name: format!("{table}_score_idx"),
        unique: false,
        method: None,
        members: vec![IndexMember {
            target: IndexTarget::Column(score),
            descending: true,
            prefix_length: None,
            collation: None,
        }],
        include: Vec::new(),
        predicate: None,
    });

    let preview = plan_review(&baseline, &draft, &PlanOptions::default()).unwrap();
    let outcome = execute(&options(), &baseline, &draft, &preview).await;
    assert!(outcome.applied(), "{outcome:?}\n{}", preview.sql_text());

    let after = capture(&mut connection, &server, "dbunk_demo", table)
        .await
        .unwrap();
    assert_eq!(
        after.editable,
        Editability::Editable,
        "{:?}",
        after.editable
    );
    let names: Vec<&str> = after
        .table
        .columns
        .iter()
        .map(|c| c.name.as_str())
        .collect();
    assert_eq!(
        names,
        [
            "id",
            "email",
            "nick",
            "full_name",
            "score",
            "updated_at",
            "org_id"
        ]
    );
    // Attributes a MODIFY must restate survived.
    let email = after
        .table
        .columns
        .iter()
        .find(|c| c.name == "email")
        .unwrap();
    assert_eq!(email.comment.as_deref(), Some("login"));
    assert_eq!(email.collation.as_deref(), Some("utf8mb4_bin"));
    let updated = after
        .table
        .columns
        .iter()
        .find(|c| c.name == "updated_at")
        .unwrap();
    assert!(updated.extra.on_update.is_some());
    assert_eq!(after.table.comment.as_deref(), Some("people v2"));
    assert_eq!(after.table.foreign_keys.len(), 1);
    let rows: i64 = sqlx::query_scalar(&format!("SELECT COUNT(*) FROM `{table}`"))
        .fetch_one(&mut connection)
        .await
        .unwrap();
    assert_eq!(rows, 2);
    connection.close().await.unwrap();
}

#[tokio::test]
#[ignore = "requires make -C infrastructure/test-db mysql"]
async fn a_table_changed_after_capture_is_not_dispatched() {
    let table = "schema_edit_drift";
    let mut connection = connect().await;
    let server = version(&mut connection).await;
    fresh(&mut connection, table).await;
    let baseline = capture(&mut connection, &server, "dbunk_demo", table)
        .await
        .unwrap();
    let mut draft = SchemaDraft::from_baseline(&baseline);
    draft.add_column("extra", "int");
    let preview = plan_review(&baseline, &draft, &PlanOptions::default()).unwrap();
    connection
        .execute(format!("ALTER TABLE `{table}` ADD COLUMN sneaky INT").as_str())
        .await
        .unwrap();
    let outcome = execute(&options(), &baseline, &draft, &preview).await;
    assert_eq!(
        outcome,
        SchemaEditOutcome::NotDispatched {
            reason: SchemaEditFailure::BaselineChanged
        }
    );
    let after = capture(&mut connection, &server, "dbunk_demo", table)
        .await
        .unwrap();
    assert!(after.table.columns.iter().all(|c| c.name != "extra"));
    connection.close().await.unwrap();
}

#[tokio::test]
#[ignore = "requires make -C infrastructure/test-db mysql"]
async fn a_rejected_statement_leaves_the_table_unchanged() {
    let table = "schema_edit_reject";
    let mut connection = connect().await;
    let server = version(&mut connection).await;
    fresh(&mut connection, table).await;
    let baseline = capture(&mut connection, &server, "dbunk_demo", table)
        .await
        .unwrap();
    let mut draft = SchemaDraft::from_baseline(&baseline);
    // `name` holds NULL, so NOT NULL is rejected by the server.
    let name = origin(&draft, "name");
    draft.table.column_mut(name).unwrap().nullable = false;
    draft.add_column("extra", "int");
    let preview = plan_review(&baseline, &draft, &PlanOptions::default()).unwrap();
    let outcome = execute(&options(), &baseline, &draft, &preview).await;
    assert!(
        matches!(outcome, SchemaEditOutcome::RolledBack { .. }),
        "{outcome:?}"
    );
    let after = capture(&mut connection, &server, "dbunk_demo", table)
        .await
        .unwrap();
    assert_eq!(after.drift, baseline.drift);
    connection.close().await.unwrap();
}

#[tokio::test]
#[ignore = "requires make -C infrastructure/test-db mysql"]
async fn retype_with_default_and_rename_without_a_default_database() {
    let table = "schema_edit_price";
    let mut connection = connect().await;
    let server = version(&mut connection).await;
    connection
        .execute(
            format!(
                "DROP TABLE IF EXISTS `{table}`, `{table}_v2`;
                 CREATE TABLE `{table}` (id INT PRIMARY KEY, price DECIMAL(10,2) NOT NULL DEFAULT '0.00')"
            )
            .as_str(),
        )
        .await
        .unwrap();
    let baseline = capture(&mut connection, &server, "dbunk_demo", table)
        .await
        .unwrap();
    assert_eq!(
        baseline.editable,
        Editability::Editable,
        "{:?}",
        baseline.editable
    );
    let mut draft = SchemaDraft::from_baseline(&baseline);
    let price = origin(&draft, "price");
    draft.table.column_mut(price).unwrap().data_type = "decimal(12,4)".into();
    draft.table.name = format!("{table}_v2");
    let preview = plan_review(&baseline, &draft, &PlanOptions::default()).unwrap();
    // No default database: the rename must still land in dbunk_demo.
    let mut bare = MySqlConnectOptions::new()
        .host("127.0.0.1")
        .port(13306)
        .username("dbunk")
        .password("dbunk");
    bare = bare.statement_cache_capacity(0);
    let outcome = execute(&bare, &baseline, &draft, &preview).await;
    assert!(outcome.applied(), "{outcome:?}\n{}", preview.sql_text());
    let after = capture(
        &mut connection,
        &server,
        "dbunk_demo",
        &format!("{table}_v2"),
    )
    .await
    .unwrap();
    assert_eq!(after.editable, Editability::Editable);
    connection.close().await.unwrap();
}

#[tokio::test]
#[ignore = "requires make -C infrastructure/test-db mysql"]
async fn a_failure_after_the_first_statement_reports_the_committed_prefix() {
    let table = "schema_edit_partial";
    let mut connection = connect().await;
    let server = version(&mut connection).await;
    fresh(&mut connection, table).await;
    // No org 99, so the foreign key add (its own, last statement) fails
    // after the column add has committed.
    connection
        .execute(format!("INSERT INTO `{table}` (email, org_id) VALUES ('c@x', 99)").as_str())
        .await
        .unwrap();
    let baseline = capture(&mut connection, &server, "dbunk_demo", table)
        .await
        .unwrap();
    let mut draft = SchemaDraft::from_baseline(&baseline);
    draft.add_column("extra", "int");
    let org = origin(&draft, "org_id");
    let fk = draft.new_origin();
    draft.table.foreign_keys.push(ForeignKeyModel {
        origin: fk,
        name: Some(format!("{table}_org_fk")),
        columns: vec![org],
        referenced_schema: None,
        referenced_table: format!("{table}_orgs"),
        referenced_columns: vec!["id".into()],
        on_update: ReferentialAction::NoAction,
        on_delete: ReferentialAction::NoAction,
        deferrable: Deferrable::Never,
    });
    let preview = plan_review(&baseline, &draft, &PlanOptions::default()).unwrap();
    assert_eq!(preview.statements.len(), 2, "{}", preview.sql_text());
    let outcome = execute(&options(), &baseline, &draft, &preview).await;
    match &outcome {
        SchemaEditOutcome::Stopped {
            committed: 1,
            failed_statement: 1,
            reason: SchemaEditFailure::Database { .. },
        } => {}
        other => panic!("expected Stopped after one statement, got {other:?}"),
    }
    assert!(outcome.may_have_changed());
    let after = capture(&mut connection, &server, "dbunk_demo", table)
        .await
        .unwrap();
    assert!(after.table.columns.iter().any(|c| c.name == "extra"));
    assert!(after.table.foreign_keys.is_empty());
    connection.close().await.unwrap();
}

#[tokio::test]
#[ignore = "requires make -C infrastructure/test-db mysql"]
async fn a_held_metadata_lock_times_out_and_changes_nothing() {
    let table = "schema_edit_locked";
    let mut connection = connect().await;
    let server = version(&mut connection).await;
    fresh(&mut connection, table).await;
    let baseline = capture(&mut connection, &server, "dbunk_demo", table)
        .await
        .unwrap();
    let mut draft = SchemaDraft::from_baseline(&baseline);
    draft.add_column("extra", "int");
    let preview = plan_review(&baseline, &draft, &PlanOptions::default()).unwrap();
    // An open transaction that read the table holds a shared metadata lock
    // until it ends; the ALTER waits lock_wait_timeout seconds for it.
    let mut holder = connect().await;
    holder.execute("BEGIN").await.unwrap();
    holder
        .execute(format!("SELECT count(*) FROM `{table}`").as_str())
        .await
        .unwrap();
    let outcome = execute(&options(), &baseline, &draft, &preview).await;
    holder.execute("ROLLBACK").await.unwrap();
    assert_eq!(
        outcome,
        SchemaEditOutcome::RolledBack {
            reason: SchemaEditFailure::LockTimeout
        }
    );
    let after = capture(&mut connection, &server, "dbunk_demo", table)
        .await
        .unwrap();
    assert_eq!(after.drift, baseline.drift);
    holder.close().await.unwrap();
    connection.close().await.unwrap();
}

#[tokio::test]
#[ignore = "requires make -C infrastructure/test-db mysql"]
async fn a_declared_row_format_captures_editable_and_round_trips() {
    let table = "schema_edit_row_format";
    let mut connection = connect().await;
    let server = version(&mut connection).await;
    connection
        .execute(
            format!(
                "DROP TABLE IF EXISTS `{table}`;
                 CREATE TABLE `{table}` (id INT PRIMARY KEY, note TEXT) ROW_FORMAT=COMPACT"
            )
            .as_str(),
        )
        .await
        .unwrap();
    let baseline = capture(&mut connection, &server, "dbunk_demo", table)
        .await
        .unwrap();
    assert_eq!(
        baseline.editable,
        Editability::Editable,
        "{:?}",
        baseline.editable
    );
    assert_eq!(
        baseline.table.options.row_format.as_deref(),
        Some("COMPACT")
    );
    let mut draft = SchemaDraft::from_baseline(&baseline);
    draft.table.options.row_format = Some("DYNAMIC".into());
    let preview = plan_review(&baseline, &draft, &PlanOptions::default()).unwrap();
    let outcome = execute(&options(), &baseline, &draft, &preview).await;
    assert!(outcome.applied(), "{outcome:?}\n{}", preview.sql_text());
    let after = capture(&mut connection, &server, "dbunk_demo", table)
        .await
        .unwrap();
    assert_eq!(after.editable, Editability::Editable);
    assert_eq!(after.table.options.row_format.as_deref(), Some("DYNAMIC"));
    connection.close().await.unwrap();
}

/// Applies `draft` (verified by the executor's re-capture), then checks the
/// new capture is editable and its own fixed point.
async fn step(
    connection: &mut MySqlConnection,
    server: &str,
    table: &str,
    baseline: &SchemaBaseline,
    draft: &SchemaDraft,
    plan: &PlanOptions,
) -> SchemaBaseline {
    let preview = plan_review(baseline, draft, plan).unwrap();
    let outcome = execute(&options(), baseline, draft, &preview).await;
    assert!(outcome.applied(), "{outcome:?}\n{}", preview.sql_text());
    let after = capture(connection, server, "dbunk_demo", table)
        .await
        .unwrap();
    assert_eq!(
        after.editable,
        Editability::Editable,
        "{:?}",
        after.editable
    );
    assert_eq!(
        plan_review(
            &after,
            &SchemaDraft::from_baseline(&after),
            &PlanOptions::default()
        ),
        Err(PlanError::NoChanges)
    );
    after
}

fn named<'a>(table: &'a TableModel, name: &str) -> &'a ColumnModel {
    table
        .columns
        .iter()
        .find(|c| c.name == name)
        .unwrap_or_else(|| panic!("column {name}"))
}

#[tokio::test]
#[ignore = "requires make -C infrastructure/test-db mysql"]
async fn drops_checks_keys_generated_columns_and_charsets_round_trip() {
    let table = "schema_edit_kinds";
    let mut connection = connect().await;
    let server = version(&mut connection).await;
    fresh(&mut connection, table).await;
    let baseline = capture(&mut connection, &server, "dbunk_demo", table)
        .await
        .unwrap();

    // Drop a column with its check, a unique key and an index; move an
    // existing column; change a charset; add a check and a generated column.
    let mut draft = SchemaDraft::from_baseline(&baseline);
    draft.table.checks.clear();
    assert!(draft.remove_column(origin(&draft, "score")));
    draft.table.uniques.clear();
    draft.table.indexes.clear();
    let org = origin(&draft, "org_id");
    draft.move_column(org, 1);
    let name = origin(&draft, "name");
    let column = draft.table.column_mut(name).unwrap();
    column.extra.charset = Some("latin1".into());
    column.collation = Some("latin1_swedish_ci".into());
    let check = draft.new_origin();
    draft.table.checks.push(CheckModel {
        origin: check,
        name: Some(format!("{table}_name_check")),
        expression: SqlFragment::new("name <> ''"),
        enforced: true,
    });
    let length = draft.add_column("email_length", "int");
    draft.table.column_mut(length).unwrap().generated = Some(Generated {
        expression: SqlFragment::new("char_length(email)"),
        stored: false,
    });
    let after = step(
        &mut connection,
        &server,
        table,
        &baseline,
        &draft,
        &PlanOptions::default(),
    )
    .await;
    let t = &after.table;
    let names: Vec<&str> = t.columns.iter().map(|c| c.name.as_str()).collect();
    assert_eq!(
        names,
        [
            "id",
            "org_id",
            "email",
            "name",
            "updated_at",
            "email_length"
        ]
    );
    assert!(t.uniques.is_empty() && t.indexes.is_empty());
    assert_eq!(t.checks.len(), 1);
    assert_eq!(named(t, "name").extra.charset.as_deref(), Some("latin1"));
    assert!(named(t, "email_length").generated.is_some());

    // Drop the primary key (and AUTO_INCREMENT, which needs a key) and the
    // generated column.
    let mut draft = SchemaDraft::from_baseline(&after);
    draft.table.primary_key = None;
    let id = origin(&draft, "id");
    draft.table.column_mut(id).unwrap().identity = Identity::None;
    assert!(draft.remove_column(origin(&draft, "email_length")));
    let after = step(
        &mut connection,
        &server,
        table,
        &after,
        &draft,
        &PlanOptions::default(),
    )
    .await;
    assert!(after.table.primary_key.is_none());
    assert_eq!(named(&after.table, "id").identity, Identity::None);

    // A composite primary key, with the online-only guard on an add the
    // server can do in place.
    let mut draft = SchemaDraft::from_baseline(&after);
    let id = origin(&draft, "id");
    let email = origin(&draft, "email");
    let key = draft.new_origin();
    draft.table.primary_key = Some(KeyModel {
        origin: key,
        name: None,
        columns: vec![id, email],
    });
    let after = step(
        &mut connection,
        &server,
        table,
        &after,
        &draft,
        &PlanOptions::default(),
    )
    .await;
    let pk = after.table.primary_key.as_ref().unwrap();
    assert_eq!(after.table.column_names(&pk.columns), ["id", "email"]);

    let mut draft = SchemaDraft::from_baseline(&after);
    draft.add_column("note", "varchar(10)");
    let online = PlanOptions {
        online_only: true,
        ..PlanOptions::default()
    };
    let after = step(&mut connection, &server, table, &after, &draft, &online).await;
    assert_eq!(after.table.columns.last().unwrap().name, "note");

    let rows: i64 = sqlx::query_scalar(&format!("SELECT COUNT(*) FROM `{table}`"))
        .fetch_one(&mut connection)
        .await
        .unwrap();
    assert_eq!(rows, 2);
    connection.close().await.unwrap();
}
