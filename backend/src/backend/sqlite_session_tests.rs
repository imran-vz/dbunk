use super::*;
use crate::safety::policy::resolve_policy;
use crate::{ConnectionPolicy, Environment, SafeMode};

const DEADLINE: Duration = Duration::from_secs(5);

fn policy(environment: Environment, safe_mode: SafeMode, read_only: bool) -> ResolvedSafetyPolicy {
    resolve_policy(ConnectionPolicy {
        environment,
        safe_mode,
        read_only,
    })
}

async fn fixture(dir: &tempfile::TempDir) -> String {
    let path = dir.path().join("app.db");
    let options = SqliteConnectOptions::new()
        .filename(&path)
        .create_if_missing(true);
    let mut connection = SqliteConnection::connect_with(&options).await.unwrap();
    sqlx::raw_sql(
        "CREATE TABLE authors (id INTEGER PRIMARY KEY, name TEXT NOT NULL DEFAULT 'anon');
         CREATE TABLE books (
             id INTEGER PRIMARY KEY AUTOINCREMENT,
             author_id INTEGER REFERENCES authors(id) ON DELETE CASCADE,
             title TEXT,
             price REAL,
             cover BLOB,
             UNIQUE (author_id, title)
         );
         CREATE INDEX books_title ON books (title);
         CREATE INDEX books_lower ON books (lower(title));
         CREATE VIEW cheap AS SELECT title FROM books WHERE price < 10;
         CREATE TRIGGER books_touch AFTER UPDATE ON books BEGIN SELECT 1; END;
         CREATE TABLE \"odd \"\"name\" (v TEXT);
         INSERT INTO authors (id, name) VALUES (1, 'Ann'), (2, 'Bo');
         INSERT INTO books (author_id, title, price, cover) VALUES
             (1, 'NULL', 3.0, x'00FF'),
             (1, NULL, 12.5, NULL),
             (2, 'Gamma', 7.25, NULL);
         INSERT INTO \"odd \"\"name\" VALUES ('x');",
    )
    .execute(&mut connection)
    .await
    .unwrap();
    connection.close().await.unwrap();
    path.to_string_lossy().into_owned()
}

fn config(path: &str, policy: ResolvedSafetyPolicy) -> SqliteSessionConfig {
    SqliteSessionConfig {
        connection_id: "sqlite-test".into(),
        path: path.into(),
        read_only: policy.read_only,
        policy,
        audit: None,
    }
}

async fn open(path: &str) -> SqliteSession {
    SqliteSession::open(config(
        path,
        policy(Environment::Development, SafeMode::Inherit, false),
    ))
    .await
    .unwrap()
}

async fn run(session: &SqliteSession, sql: &str) -> SqliteExecution {
    session
        .execute(session.ticket(), sql.into(), false)
        .await
        .unwrap()
}

fn completed(execution: SqliteExecution) -> (Vec<SqliteResultSet>, u64) {
    match execution {
        SqliteExecution::Completed {
            sets,
            rows_affected,
            ..
        } => (sets, rows_affected),
        other => panic!("expected completion, got {other:?}"),
    }
}

/// A statement that keeps the VM busy until interrupted.
const ENDLESS: &str = "WITH RECURSIVE n(x) AS (SELECT 1 UNION ALL SELECT x + 1 FROM n) \
                       SELECT count(*) FROM n";

#[tokio::test]
async fn objects_list_main_and_attached_databases_by_kind() {
    let dir = tempfile::tempdir().unwrap();
    let path = fixture(&dir).await;
    let other = dir.path().join("other.db");
    let session = open(&path).await;
    // The session opened without SQLITE_OPEN_CREATE, so ATTACH cannot
    // create a database file either.
    assert!(session
        .execute(
            session.ticket(),
            format!("ATTACH DATABASE '{}' AS extra", other.display()),
            false,
        )
        .await
        .is_err());
    assert!(!other.exists());
    std::fs::File::create(&other).unwrap();
    completed(
        run(
            &session,
            &format!(
                "ATTACH DATABASE '{}' AS extra; CREATE TABLE extra.notes (body TEXT);",
                other.display()
            ),
        )
        .await,
    );
    let objects = session.objects(session.ticket()).await.unwrap();
    let names: Vec<&str> = objects.databases.iter().map(|d| d.name.as_str()).collect();
    assert_eq!(names, ["main", "extra"]);
    let main = &objects.databases[0];
    let tables: Vec<&str> = main.tables.iter().map(|o| o.name.as_str()).collect();
    // sqlite_sequence (AUTOINCREMENT) and autoindexes are SQLite's own.
    assert_eq!(tables, ["authors", "books", "odd \"name"]);
    assert_eq!(main.views[0].name, "cheap");
    let indexes: Vec<(&str, &str)> = main
        .indexes
        .iter()
        .map(|o| (o.name.as_str(), o.table.as_str()))
        .collect();
    assert_eq!(
        indexes,
        [("books_lower", "books"), ("books_title", "books")]
    );
    assert_eq!(main.triggers[0].name, "books_touch");
    assert_eq!(main.triggers[0].table, "books");
    assert!(main.file.ends_with("app.db"));
    assert_eq!(objects.databases[1].tables[0].name, "notes");
    session.close(DEADLINE).await.unwrap();
}

#[tokio::test]
async fn execute_keeps_null_distinct_from_text_and_reports_each_result_set() {
    let dir = tempfile::tempdir().unwrap();
    let path = fixture(&dir).await;
    let session = open(&path).await;
    let (sets, affected) = completed(
        run(
            &session,
            "SELECT title, price, cover FROM books ORDER BY id; \
             UPDATE books SET price = price + 1 WHERE author_id = 1; \
             SELECT count(*) AS n FROM authors;",
        )
        .await,
    );
    assert_eq!(sets.len(), 2);
    assert_eq!(sets[0].columns, ["title", "price", "cover"]);
    assert_eq!(
        sets[0].rows[0],
        [
            Some("NULL".into()),
            Some("3.0".into()),
            Some("x'00FF'".into())
        ]
    );
    assert_eq!(sets[0].rows[1], [None, Some("12.5".into()), None]);
    assert_eq!(sets[1].rows, [[Some("2".to_string())]]);
    // The trailing SELECT does not repeat the UPDATE's count.
    assert_eq!(affected, 2);
    session.close(DEADLINE).await.unwrap();
}

#[tokio::test]
async fn execute_bounds_rows_but_still_runs_every_statement() {
    let dir = tempfile::tempdir().unwrap();
    let path = fixture(&dir).await;
    let session = open(&path).await;
    let total = SQLITE_MAX_ROWS_PER_SET as u64 + 25;
    let (sets, affected) = completed(
        run(
            &session,
            &format!(
                "WITH RECURSIVE n(x) AS (SELECT 1 UNION ALL SELECT x + 1 FROM n LIMIT {total}) \
                 SELECT x FROM n; INSERT INTO authors (name) VALUES ('after');"
            ),
        )
        .await,
    );
    assert_eq!(sets[0].rows.len(), SQLITE_MAX_ROWS_PER_SET);
    assert_eq!(sets[0].row_count, total);
    assert_eq!(sets[0].omitted_rows, 25);
    assert_eq!(affected, 1);
    let (sets, _) = completed(run(&session, "SELECT count(*) FROM authors").await);
    assert_eq!(sets[0].rows[0][0].as_deref(), Some("3"));
    session.close(DEADLINE).await.unwrap();
}

#[tokio::test]
async fn long_text_cells_are_cut_and_counted() {
    let dir = tempfile::tempdir().unwrap();
    let path = fixture(&dir).await;
    let session = open(&path).await;
    let (sets, _) = completed(
        run(
            &session,
            &format!(
                "SELECT replace(hex(zeroblob({})), '00', 'é') AS t",
                SQLITE_MAX_CELL_BYTES
            ),
        )
        .await,
    );
    let text = sets[0].rows[0][0].as_ref().unwrap();
    assert!(text.len() <= SQLITE_MAX_CELL_BYTES);
    assert!(text.chars().all(|c| c == 'é'));
    assert_eq!(sets[0].truncated_cells, 1);
    session.close(DEADLINE).await.unwrap();
}

#[tokio::test]
async fn empty_read_still_names_its_columns() {
    let dir = tempfile::tempdir().unwrap();
    let path = fixture(&dir).await;
    let session = open(&path).await;
    let (sets, _) = completed(run(&session, "SELECT id, name FROM authors WHERE 0").await);
    assert_eq!(sets.len(), 1);
    assert_eq!(sets[0].columns, ["id", "name"]);
    assert!(sets[0].rows.is_empty());
    session.close(DEADLINE).await.unwrap();
}

#[tokio::test]
async fn sql_errors_are_reported_and_the_session_stays_usable() {
    let dir = tempfile::tempdir().unwrap();
    let path = fixture(&dir).await;
    let session = open(&path).await;
    let error = session
        .execute(session.ticket(), "SELECT * FROM missing".into(), false)
        .await
        .unwrap_err();
    assert_eq!(
        error,
        SqliteSessionError::Failed("no such table: missing".into())
    );
    let (sets, _) = completed(run(&session, "SELECT 1").await);
    assert_eq!(sets[0].rows[0][0].as_deref(), Some("1"));
    session.close(DEADLINE).await.unwrap();
}

#[tokio::test]
async fn strict_policy_requires_confirmation_before_any_write_runs() {
    let dir = tempfile::tempdir().unwrap();
    let path = fixture(&dir).await;
    let session = SqliteSession::open(config(
        &path,
        policy(Environment::Production, SafeMode::Inherit, false),
    ))
    .await
    .unwrap();
    let sql = "SELECT 1; UPDATE authors SET name = 'Bee' WHERE id = 2;";
    let refused = session
        .execute(session.ticket(), sql.into(), false)
        .await
        .unwrap();
    assert!(matches!(
        refused,
        SqliteExecution::NeedsConfirmation { ref statements } if statements.len() == 2
    ));
    let (sets, _) = completed(run(&session, "SELECT name FROM authors WHERE id = 2").await);
    assert_eq!(sets[0].rows[0][0].as_deref(), Some("Bo"), "nothing ran");
    let (_, affected) = completed(
        session
            .execute(session.ticket(), sql.into(), true)
            .await
            .unwrap(),
    );
    assert_eq!(affected, 1);
    session.close(DEADLINE).await.unwrap();
}

#[tokio::test]
async fn read_only_connections_block_writes_and_open_the_file_read_only() {
    let dir = tempfile::tempdir().unwrap();
    let path = fixture(&dir).await;
    let session = SqliteSession::open(config(
        &path,
        policy(Environment::Development, SafeMode::Disabled, true),
    ))
    .await
    .unwrap();
    assert!(session.info().read_only);
    let refused = session
        .execute(session.ticket(), "DELETE FROM authors".into(), true)
        .await
        .unwrap();
    assert!(matches!(refused, SqliteExecution::Blocked { .. }));
    let (sets, _) = completed(run(&session, "SELECT count(*) FROM authors").await);
    assert_eq!(sets[0].rows[0][0].as_deref(), Some("2"));
    session.close(DEADLINE).await.unwrap();
    // The engine refuses writes too, independent of the classifier.
    let mut engine_only = config(
        &path,
        policy(Environment::Development, SafeMode::Disabled, false),
    );
    engine_only.read_only = true;
    let session = SqliteSession::open(engine_only).await.unwrap();
    let error = session
        .execute(session.ticket(), "DELETE FROM authors".into(), false)
        .await
        .unwrap_err();
    assert!(
        matches!(&error, SqliteSessionError::Failed(message) if message.contains("readonly")),
        "{error:?}"
    );
    session.close(DEADLINE).await.unwrap();
}

#[tokio::test]
async fn browse_pages_in_natural_order_and_quotes_names() {
    let dir = tempfile::tempdir().unwrap();
    let path = fixture(&dir).await;
    let session = open(&path).await;
    let first = session
        .browse(session.ticket(), "main".into(), "books".into(), 0, 2)
        .await
        .unwrap();
    assert!(first.has_more);
    assert_eq!(first.set.rows.len(), 2);
    assert_eq!(first.set.omitted_rows, 0);
    assert_eq!(first.set.rows[0][0].as_deref(), Some("1"));
    let second = session
        .browse(session.ticket(), "main".into(), "books".into(), 2, 2)
        .await
        .unwrap();
    assert!(!second.has_more);
    assert_eq!(second.set.rows.len(), 1);
    assert_eq!(second.set.rows[0][0].as_deref(), Some("3"));
    let odd = session
        .browse(session.ticket(), "main".into(), "odd \"name".into(), 0, 50)
        .await
        .unwrap();
    assert_eq!(odd.set.rows, [[Some("x".to_string())]]);
    let empty = session
        .browse(session.ticket(), "main".into(), "books".into(), 100, 50)
        .await
        .unwrap();
    assert!(empty.set.rows.is_empty());
    assert_eq!(empty.set.columns[0], "id");
    let injected = session
        .browse(
            session.ticket(),
            "main".into(),
            "books\"; DROP TABLE authors; --".into(),
            0,
            50,
        )
        .await;
    assert!(injected.is_err());
    let (sets, _) = completed(run(&session, "SELECT count(*) FROM authors").await);
    assert_eq!(sets[0].rows[0][0].as_deref(), Some("2"));
    session.close(DEADLINE).await.unwrap();
}

#[tokio::test]
async fn structure_reports_columns_keys_indexes_and_triggers() {
    let dir = tempfile::tempdir().unwrap();
    let path = fixture(&dir).await;
    let session = open(&path).await;
    let books = session
        .structure(session.ticket(), "main".into(), "books".into())
        .await
        .unwrap();
    assert_eq!(books.kind, "table");
    assert!(books
        .definition
        .as_deref()
        .unwrap()
        .starts_with("CREATE TABLE books"));
    let id = &books.columns[0];
    assert_eq!((id.name.as_str(), id.primary_key), ("id", 1));
    assert_eq!(books.columns[3].declared_type, "REAL");
    let lower = books
        .indexes
        .iter()
        .find(|index| index.name == "books_lower")
        .unwrap();
    assert_eq!(lower.columns, ["<expression>"]);
    assert!(books.indexes.iter().any(|index| index.unique
        && index.origin == "u"
        && index.columns == ["author_id", "title"]));
    assert_eq!(books.foreign_keys.len(), 1);
    assert_eq!(books.foreign_keys[0].table, "authors");
    assert_eq!(books.foreign_keys[0].columns, ["author_id"]);
    assert_eq!(books.foreign_keys[0].on_delete, "CASCADE");
    assert_eq!(books.triggers[0].name, "books_touch");
    let authors = session
        .structure(session.ticket(), "main".into(), "authors".into())
        .await
        .unwrap();
    assert!(authors.columns[1].not_null);
    assert_eq!(authors.columns[1].default_value.as_deref(), Some("'anon'"));
    let view = session
        .structure(session.ticket(), "main".into(), "cheap".into())
        .await
        .unwrap();
    assert_eq!(view.kind, "view");
    assert_eq!(view.columns[0].name, "title");
    let missing = session
        .structure(session.ticket(), "main".into(), "gone".into())
        .await;
    assert!(matches!(missing, Err(SqliteSessionError::Failed(_))));
    session.close(DEADLINE).await.unwrap();
}

#[tokio::test]
async fn cancel_interrupts_only_the_named_request() {
    let dir = tempfile::tempdir().unwrap();
    let path = fixture(&dir).await;
    let session = Arc::new(open(&path).await);
    let ticket = session.ticket();
    let running = {
        let session = session.clone();
        tokio::spawn(async move { session.execute(ticket, ENDLESS.into(), false).await })
    };
    tokio::time::sleep(Duration::from_millis(100)).await;
    // Issued but never submitted: tickets are process-wide, so cancelling a
    // made-up number could hit a parallel test's request.
    let unrelated = session.ticket();
    session.cancel(unrelated);
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert!(
        !running.is_finished(),
        "an unrelated cancel must not interrupt"
    );
    session.cancel(ticket);
    let result = tokio::time::timeout(DEADLINE, running)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(result, Err(SqliteSessionError::Cancelled));
    let (sets, _) = completed(run(&session, "SELECT 2").await);
    assert_eq!(sets[0].rows[0][0].as_deref(), Some("2"));
    session.close(DEADLINE).await.unwrap();
}

async fn author_count(session: &SqliteSession) -> Option<String> {
    let (sets, _) = completed(run(session, "SELECT count(*) FROM authors").await);
    sets[0].rows[0][0].clone()
}

#[tokio::test]
async fn a_request_stopped_while_queued_never_runs_even_after_later_cancels() {
    let dir = tempfile::tempdir().unwrap();
    let path = fixture(&dir).await;
    let session = Arc::new(open(&path).await);
    let blocker = session.ticket();
    let running = {
        let session = session.clone();
        tokio::spawn(async move { session.execute(blocker, ENDLESS.into(), false).await })
    };
    tokio::time::sleep(Duration::from_millis(100)).await;
    let insert = session.ticket();
    let queued = {
        let session = session.clone();
        tokio::spawn(async move {
            session
                .execute(
                    insert,
                    "INSERT INTO authors (name) VALUES ('queued')".into(),
                    false,
                )
                .await
        })
    };
    tokio::time::sleep(Duration::from_millis(50)).await;
    session.cancel(insert);
    // A later request and its own cancel must not revive the stopped one
    // (a single shared cancel slot used to be overwritten here).
    let later = session.ticket();
    let tree = {
        let session = session.clone();
        tokio::spawn(async move { session.objects(later).await })
    };
    tokio::time::sleep(Duration::from_millis(20)).await;
    session.cancel(later);
    session.cancel(blocker);
    assert_eq!(running.await.unwrap(), Err(SqliteSessionError::Cancelled));
    assert_eq!(queued.await.unwrap(), Err(SqliteSessionError::Cancelled));
    assert_eq!(tree.await.unwrap(), Err(SqliteSessionError::Cancelled));
    assert_eq!(author_count(&session).await.as_deref(), Some("2"));
    session.close(DEADLINE).await.unwrap();
}

#[tokio::test]
async fn a_ticket_cancelled_before_submission_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let path = fixture(&dir).await;
    let session = open(&path).await;
    let ticket = session.ticket();
    session.cancel(ticket);
    let result = session
        .execute(
            ticket,
            "INSERT INTO authors (name) VALUES ('late')".into(),
            false,
        )
        .await;
    assert_eq!(result, Err(SqliteSessionError::Cancelled));
    assert_eq!(author_count(&session).await.as_deref(), Some("2"));
    session.close(DEADLINE).await.unwrap();
}

#[tokio::test]
async fn tickets_are_unique_across_sessions() {
    let dir = tempfile::tempdir().unwrap();
    let path = fixture(&dir).await;
    let first = open(&path).await;
    let second = open(&path).await;
    let old = first.ticket();
    let new = second.ticket();
    assert_ne!(old, new);
    // Stop with a ticket from an older session touches nothing here.
    second.cancel(old);
    assert_eq!(author_count(&second).await.as_deref(), Some("2"));
    first.close(DEADLINE).await.unwrap();
    second.close(DEADLINE).await.unwrap();
}

#[tokio::test]
async fn dropping_a_session_interrupts_its_running_statement() {
    let dir = tempfile::tempdir().unwrap();
    let path = fixture(&dir).await;
    let session = open(&path).await;
    // Reading `authors` holds a shared lock for as long as it runs.
    let endless = "WITH RECURSIVE n(x) AS (SELECT 1 UNION ALL SELECT x + 1 FROM n) \
                   SELECT count(*) FROM n, authors";
    let started = tokio::time::timeout(
        Duration::from_millis(100),
        session.execute(session.ticket(), endless.into(), false),
    )
    .await;
    assert!(started.is_err(), "the statement is still running");
    drop(session);
    // A writer needs that lock released; a statement left running would
    // keep it past the busy timeout.
    let writer = open(&path).await;
    completed(run(&writer, "INSERT INTO authors (name) VALUES ('after drop')").await);
    assert_eq!(author_count(&writer).await.as_deref(), Some("3"));
    writer.close(DEADLINE).await.unwrap();
}

#[tokio::test]
async fn foreign_keys_are_off_like_the_sqlite_shell() {
    let dir = tempfile::tempdir().unwrap();
    let path = fixture(&dir).await;
    let session = open(&path).await;
    let (sets, _) = completed(run(&session, "PRAGMA foreign_keys").await);
    assert_eq!(sets[0].rows[0][0].as_deref(), Some("0"));
    let (_, affected) = completed(
        run(
            &session,
            "INSERT INTO books (author_id, title) VALUES (99, 'orphan')",
        )
        .await,
    );
    assert_eq!(affected, 1);
    session.close(DEADLINE).await.unwrap();
}

#[tokio::test]
async fn byte_bounded_pages_advance_by_the_rows_they_kept() {
    let dir = tempfile::tempdir().unwrap();
    let path = fixture(&dir).await;
    let session = open(&path).await;
    // 300 rows of 64 KiB: more than the 16 MiB bound fits in one page.
    completed(
        run(
            &session,
            "CREATE TABLE big (id INTEGER PRIMARY KEY, body TEXT); \
             WITH RECURSIVE n(x) AS (SELECT 1 UNION ALL SELECT x + 1 FROM n LIMIT 300) \
             INSERT INTO big SELECT x, replace(hex(zeroblob(32768)), '0', 'a') FROM n;",
        )
        .await,
    );
    let mut offset = 0u64;
    let mut seen = Vec::new();
    let mut shortened = false;
    loop {
        let page = session
            .browse(
                session.ticket(),
                "main".into(),
                "big".into(),
                offset,
                SQLITE_MAX_PAGE_ROWS,
            )
            .await
            .unwrap();
        assert_eq!(page.offset, offset);
        assert_eq!(page.set.omitted_rows, 0);
        assert_eq!(page.set.row_count, page.set.rows.len() as u64);
        assert!(
            !page.has_more || !page.set.rows.is_empty(),
            "a page always advances"
        );
        shortened |= page.has_more && page.set.rows.len() < SQLITE_MAX_PAGE_ROWS as usize;
        seen.extend(
            page.set
                .rows
                .iter()
                .map(|row| row[0].as_deref().unwrap().parse::<u64>().unwrap()),
        );
        offset += page.set.rows.len() as u64;
        if !page.has_more {
            break;
        }
    }
    assert!(shortened, "the byte bound applied");
    assert_eq!(
        seen,
        (1..=300).collect::<Vec<u64>>(),
        "no row skipped or repeated"
    );
    session.close(DEADLINE).await.unwrap();
}

#[tokio::test]
async fn confirmed_overrides_are_audited_even_when_a_later_statement_fails() {
    let dir = tempfile::tempdir().unwrap();
    let path = fixture(&dir).await;
    let audit = sqlx::SqlitePool::connect_with(
        SqliteConnectOptions::new()
            .filename(dir.path().join("audit.db"))
            .create_if_missing(true),
    )
    .await
    .unwrap();
    sqlx::raw_sql(
        "CREATE TABLE safety_overrides (
             id INTEGER PRIMARY KEY AUTOINCREMENT,
             connection_id TEXT NOT NULL,
             command TEXT NOT NULL,
             classes TEXT NOT NULL,
             occurred_at TEXT NOT NULL
         )",
    )
    .execute(&audit)
    .await
    .unwrap();
    let mut settings = config(
        &path,
        policy(Environment::Production, SafeMode::Inherit, false),
    );
    settings.audit = Some(audit.clone());
    let session = SqliteSession::open(settings).await.unwrap();
    let error = session
        .execute(
            session.ticket(),
            "UPDATE authors SET name = 'Bee' WHERE id = 2; SELECT * FROM missing;".into(),
            true,
        )
        .await
        .unwrap_err();
    assert_eq!(
        error,
        SqliteSessionError::Failed("no such table: missing".into())
    );
    let (sets, _) = completed(run(&session, "SELECT name FROM authors WHERE id = 2").await);
    assert_eq!(
        sets[0].rows[0][0].as_deref(),
        Some("Bee"),
        "the update committed"
    );
    let audited: i64 = sqlx::query_scalar("SELECT count(*) FROM safety_overrides")
        .fetch_one(&audit)
        .await
        .unwrap();
    assert_eq!(audited, 1);
    session.close(DEADLINE).await.unwrap();
    audit.close().await;
}

#[tokio::test]
async fn full_queue_refuses_instead_of_growing() {
    let dir = tempfile::tempdir().unwrap();
    let path = fixture(&dir).await;
    let session = Arc::new(open(&path).await);
    let blocker = session.ticket();
    let running = {
        let session = session.clone();
        tokio::spawn(async move { session.execute(blocker, ENDLESS.into(), false).await })
    };
    tokio::time::sleep(Duration::from_millis(100)).await;
    let mut queued = Vec::new();
    for _ in 0..QUEUE {
        let session = session.clone();
        queued.push(tokio::spawn(async move {
            session.objects(session.ticket()).await
        }));
    }
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert_eq!(
        session.objects(session.ticket()).await,
        Err(SqliteSessionError::Busy)
    );
    session.cancel(blocker);
    assert_eq!(running.await.unwrap(), Err(SqliteSessionError::Cancelled));
    for task in queued {
        assert!(task.await.unwrap().is_ok());
    }
    session.close(DEADLINE).await.unwrap();
}

#[tokio::test]
async fn close_interrupts_running_work_and_refuses_later_requests() {
    let dir = tempfile::tempdir().unwrap();
    let path = fixture(&dir).await;
    let session = Arc::new(open(&path).await);
    completed(
        run(
            &session,
            "BEGIN; INSERT INTO authors (name) VALUES ('uncommitted');",
        )
        .await,
    );
    let ticket = session.ticket();
    let running = {
        let session = session.clone();
        tokio::spawn(async move { session.execute(ticket, ENDLESS.into(), false).await })
    };
    tokio::time::sleep(Duration::from_millis(100)).await;
    tokio::time::timeout(DEADLINE, session.close(DEADLINE))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(running.await.unwrap(), Err(SqliteSessionError::Closed));
    assert!(session.is_closed());
    assert_eq!(
        session.objects(session.ticket()).await,
        Err(SqliteSessionError::Closed)
    );
    // Closing rolled back the open transaction.
    let reopened = open(&path).await;
    let (sets, _) = completed(run(&reopened, "SELECT count(*) FROM authors").await);
    assert_eq!(sets[0].rows[0][0].as_deref(), Some("2"));
    reopened.close(DEADLINE).await.unwrap();
}

#[tokio::test]
async fn open_never_creates_a_missing_database() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("missing.db");
    let error = SqliteSession::open(config(
        &path.to_string_lossy(),
        policy(Environment::Development, SafeMode::Inherit, false),
    ))
    .await
    .unwrap_err();
    assert!(error.contains("Could not open"), "{error}");
    assert!(!path.exists());
}

#[tokio::test]
async fn non_database_files_fail_to_open() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("notes.txt");
    std::fs::write(&path, b"this is not a database file at all, just text").unwrap();
    let error = SqliteSession::open(config(
        &path.to_string_lossy(),
        policy(Environment::Development, SafeMode::Inherit, false),
    ))
    .await
    .unwrap_err();
    assert!(
        error.contains("unreadable") || error.contains("Could not open"),
        "{error}"
    );
}

#[test]
fn blob_previews_are_bounded_hex() {
    let (short, cut) = blob_preview(&[0x00, 0xab]);
    assert_eq!((short.as_str(), cut), ("x'00AB'", false));
    let long = vec![0x11u8; SQLITE_BLOB_PREVIEW_BYTES + 10];
    let (text, cut) = blob_preview(&long);
    assert!(cut);
    assert!(text.ends_with(&format!(" … {} bytes", long.len())));
    assert_eq!(text.matches("11").count(), SQLITE_BLOB_PREVIEW_BYTES);
}

#[test]
fn reals_keep_their_decimal_point() {
    assert_eq!(format_real(3.0), "3.0");
    assert_eq!(format_real(-0.5), "-0.5");
    assert_eq!(format_real(1e20), "100000000000000000000");
}
