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
