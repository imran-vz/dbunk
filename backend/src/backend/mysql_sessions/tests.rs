//! Plan 031 step 4: MySQL session bounds, policy and lifecycle.
use super::rows::{self, CellKind, Collector};
use super::worker::{
    self, authorize, browse_sql, column_order, definition_sql, objects_of, page, pinned_database,
    timed_out, unique_key, BrowseOrder,
};
use super::*;
use crate::backend::{
    DevelopmentConnectionOrganization, DevelopmentEngineConnection, DevelopmentEnvironment,
    DevelopmentMySqlConnection, DevelopmentSafeMode, DevelopmentStorageMode,
};
use crate::safety::policy::AuditDisposition;
use crate::{Environment, MySqlStoredConnection, SafeMode, SshTunnelConfig};

fn stored(environment: Environment, safe_mode: SafeMode, read_only: bool) -> StoredConnection {
    StoredConnection::MySQL(MySqlStoredConnection {
        id: "mysql".into(),
        name: "Orders".into(),
        database: "orders".into(),
        host: "db.example".into(),
        port: 0,
        user: "app".into(),
        password: "secret".into(),
        role: "read/write".into(),
        environment,
        safe_mode,
        read_only,
        last_activity_at: None,
        organization: Default::default(),
        ssl: true,
        ssh_tunnel: SshTunnelConfig::default(),
    })
}

#[test]
fn values_render_as_text_hex_or_bit_within_the_cell_budget() {
    assert_eq!(rows::cell_kind("VARCHAR"), CellKind::Text);
    assert_eq!(rows::cell_kind("DECIMAL"), CellKind::Text);
    assert_eq!(rows::cell_kind("VARBINARY"), CellKind::Bytes);
    assert_eq!(rows::cell_kind("BIT"), CellKind::Bit);
    assert_eq!(rows::cell_kind("GEOMETRY"), CellKind::Hex);

    // Text protocol values are already formatted by the server.
    assert_eq!(
        rows::render(b"2026-10-04 12:00:00", CellKind::Text, 64),
        ("2026-10-04 12:00:00".into(), false)
    );
    assert_eq!(
        rows::render(b"12.50", CellKind::Text, 64),
        ("12.50".into(), false)
    );
    // Binary columns show printable text as text and anything else as hex.
    assert_eq!(
        rows::render(b"abc", CellKind::Bytes, 64),
        ("abc".into(), false)
    );
    assert_eq!(
        rows::render(&[0x00, 0xff, 0x10], CellKind::Bytes, 64),
        ("0x00ff10".into(), false)
    );
    assert_eq!(
        rows::render(&[0x01, 0x02], CellKind::Bit, 64),
        ("258".into(), false)
    );
    assert_eq!(
        rows::render(&[1], CellKind::Hex, 64),
        ("0x01".into(), false)
    );

    // Clipping respects UTF-8 boundaries and marks the cut.
    let (text, truncated) = rows::render("ééééé".as_bytes(), CellKind::Text, 6);
    assert!(truncated);
    assert!(text.len() <= 6);
    assert_eq!(text, "é…");
    let (text, truncated) = rows::render(&[0xab; 100], CellKind::Hex, 12);
    assert!(truncated);
    assert!(text.len() <= 12, "{text}");
    assert!(text.starts_with("0xabab") && text.ends_with('…'));
}

#[test]
fn collector_keeps_the_last_result_set_and_counts_dropped_rows() {
    let mut collector = Collector::default();
    // `UPDATE …; SELECT …` keeps the select and sums affected rows.
    collector.done(3);
    collector.push_for_test(&["a"], vec![Some("1".into())]);
    collector.done(0);
    collector.push_for_test(&["b", "c"], vec![Some("x".into()), None]);
    collector.push_for_test(&["b", "c"], vec![Some("y".into()), Some("z".into())]);
    collector.done(0);
    let result = collector.finish(5);
    assert_eq!(result.columns, ["b", "c"]);
    assert_eq!(result.rows.len(), 2);
    assert_eq!(result.rows[0][1], None);
    assert_eq!(result.result_sets, 3);
    assert_eq!(result.rows_affected, 3);
    assert!(!result.truncated);

    let mut collector = Collector::default();
    for index in 0..MYSQL_MAX_ROWS + 25 {
        collector.push_for_test(&["n"], vec![Some(index.to_string())]);
    }
    collector.done(0);
    let result = collector.finish(1);
    assert_eq!(result.rows.len(), MYSQL_MAX_ROWS);
    assert_eq!(result.total_rows, (MYSQL_MAX_ROWS + 25) as u64);
    assert!(result.truncated);

    // The byte budget stops retention before the row budget.
    let mut collector = Collector::default();
    let wide = "x".repeat(MYSQL_MAX_CELL_BYTES);
    for _ in 0..MYSQL_MAX_ROWS {
        collector.push_for_test(&["w"], vec![Some(wide.clone())]);
    }
    let result = collector.finish(1);
    assert_eq!(
        result.rows.len(),
        MYSQL_MAX_RESULT_BYTES / MYSQL_MAX_CELL_BYTES
    );
    assert!(result.truncated);
    assert_eq!(result.total_rows, MYSQL_MAX_ROWS as u64);
}

#[test]
fn browse_pages_read_one_row_ahead_and_order_by_the_primary_key() {
    assert_eq!(
        browse_sql("shop", "order`s", &["id".into(), "line".into()], 50, 25),
        "SELECT * FROM `shop`.`order``s` ORDER BY `id`, `line` LIMIT 26 OFFSET 50"
    );
    assert_eq!(
        browse_sql("shop", "log", &[], 0, 10),
        "SELECT * FROM `shop`.`log` LIMIT 11 OFFSET 0"
    );
    let rows = |count: usize| MySqlResult {
        rows: (0..count).map(|i| vec![Some(i.to_string())]).collect(),
        total_rows: count as u64,
        ..Default::default()
    };
    let full = page(rows(11), 10);
    assert_eq!(full.rows.len(), 10);
    assert!(full.has_more);
    assert!(!full.truncated);
    let last = page(rows(4), 10);
    assert_eq!(last.rows.len(), 4);
    assert!(!last.has_more);
    // A byte-budget cut inside the page stays visible.
    let mut cut = rows(3);
    cut.total_rows = 11;
    let cut = page(cut, 10);
    assert!(cut.has_more && cut.truncated);
    assert!(MYSQL_MAX_PAGE_ROWS < MYSQL_MAX_ROWS as u32);
}

#[test]
fn a_byte_cut_on_the_last_page_still_reports_the_dropped_rows_as_more() {
    // 5 rows exist after the offset, the byte budget kept 3: the next page
    // starts after the 3 kept rows, so there is more to read.
    let cut = page(
        MySqlResult {
            rows: (0..3).map(|i| vec![Some(i.to_string())]).collect(),
            total_rows: 5,
            ..Default::default()
        },
        10,
    );
    assert_eq!(cut.rows.len(), 3);
    assert_eq!(cut.total_rows, 5);
    assert!(cut.truncated);
    assert!(cut.has_more);
}

#[test]
fn browse_orders_by_the_first_unique_non_null_key_else_by_every_exact_column() {
    let row = |values: &[&str]| values.iter().map(|v| (*v).to_owned()).collect::<Vec<_>>();
    // The primary key comes first from the catalog and wins.
    assert_eq!(
        unique_key(vec![
            row(&["PRIMARY", "id", ""]),
            row(&["PRIMARY", "line", ""]),
            row(&["email", "email", ""]),
        ]),
        Some(vec!["id".to_owned(), "line".to_owned()])
    );
    // Nullable and functional key parts disqualify an index.
    assert_eq!(
        unique_key(vec![
            row(&["a_nullable", "code", "YES"]),
            row(&["b_functional", "", ""]),
            row(&["c_ok", "tenant", ""]),
            row(&["c_ok", "slug", ""]),
        ]),
        Some(vec!["tenant".to_owned(), "slug".to_owned()])
    );
    assert_eq!(unique_key(vec![row(&["u", "x", "YES"])]), None);
    assert_eq!(unique_key(Vec::new()), None);

    assert_eq!(
        column_order(vec![row(&["id", "int"]), row(&["name", "varchar"])]),
        BrowseOrder {
            columns: vec!["id".into(), "name".into()],
            approximate: false
        }
    );
    // Text, blob, JSON and spatial values are left out; the order is then
    // approximate, and with no orderable column at all there is no order.
    assert_eq!(
        column_order(vec![row(&["id", "int"]), row(&["body", "LONGTEXT"])]),
        BrowseOrder {
            columns: vec!["id".into()],
            approximate: true
        }
    );
    assert_eq!(
        column_order(vec![row(&["doc", "json"])]),
        BrowseOrder {
            columns: vec![],
            approximate: true
        }
    );
    assert!(column_order(Vec::new()).approximate);
}

#[test]
fn scripts_always_run_in_the_documents_database_or_the_connection_default() {
    assert_eq!(
        pinned_database(Some("shop".into()), Some("app"), Some("other")),
        Ok(Some("shop".into()))
    );
    // No document database: the connection default is pinned explicitly,
    // whatever another tab selected.
    assert_eq!(
        pinned_database(None, Some("app"), Some("other")),
        Ok(Some("app".into()))
    );
    assert_eq!(
        pinned_database(Some(" ".into()), Some("app"), None),
        Ok(Some("app".into()))
    );
    // Neither: runs only while the session has no database.
    assert_eq!(pinned_database(None, None, None), Ok(None));
    assert!(matches!(
        pinned_database(None, None, Some("other")),
        Err(reason) if reason.contains("`other`")
    ));
}

#[test]
fn an_expired_metadata_request_fails_alone_unless_it_cannot_be_stopped() {
    assert!(matches!(timed_out(Some(Ok(7))), Ok(7)));
    // Interrupted by the kill: a request error, the session stays open.
    assert!(matches!(
        timed_out::<()>(Some(Err(worker::Failure::Request(
            MySqlSessionError::Cancelled
        )))),
        Err(worker::Failure::Request(MySqlSessionError::Database(reason)))
            if reason.contains("still open")
    ));
    // Transport loss stays fatal, and so does a request that never returns.
    assert!(matches!(
        timed_out::<()>(Some(Err(worker::Failure::Fatal("gone".into())))),
        Err(worker::Failure::Fatal(reason)) if reason == "gone"
    ));
    assert!(matches!(
        timed_out::<()>(None),
        Err(worker::Failure::Fatal(_))
    ));
}

#[test]
fn the_tracker_withdraws_queued_requests_and_only_targets_the_running_one() {
    let mut tracker = Tracker::default();
    tracker.enqueue(1);
    tracker.enqueue(2);
    assert_eq!(tracker.cancel(2), Target::Queued);
    assert!(tracker.start(1));
    assert_eq!(tracker.running, Some(1));
    assert_eq!(tracker.cancel(1), Target::Running);
    // An unknown or finished id targets nothing.
    assert_eq!(tracker.cancel(9), Target::Idle);
    tracker.finish(1);
    assert_eq!(tracker.cancel(1), Target::Idle);
    // The cancelled request is skipped when dequeued and leaves no trace.
    assert!(!tracker.start(2));
    assert_eq!(tracker.running, None);
    assert!(tracker.queued.is_empty());
    // A stale finish never clears another request.
    tracker.enqueue(3);
    assert!(tracker.start(3));
    tracker.finish(1);
    assert_eq!(tracker.running, Some(3));
    // A request refused by a full queue is forgotten.
    tracker.enqueue(4);
    tracker.withdraw(4);
    assert_eq!(tracker.cancel(4), Target::Idle);
}

/// Cancelling a queued or finished request never opens a side connection,
/// so no `KILL QUERY` can reach whatever is running.
#[tokio::test]
async fn cancel_kills_only_the_running_request() {
    // Port 1 refuses: any attempt to connect would fail the cancel.
    let options = MySqlConnectOptions::new().host("127.0.0.1").port(1);
    let tracker = Mutex::new(Tracker::default());
    tracker.lock().await.enqueue(1);
    tracker.lock().await.enqueue(2);
    assert!(tracker.lock().await.start(1));
    assert_eq!(
        worker::cancel(&options, 42, &tracker, 2).await,
        Ok(MySqlCancel::Withdrawn)
    );
    assert_eq!(
        worker::cancel(&options, 42, &tracker, 7).await,
        Ok(MySqlCancel::Finished)
    );
    // The running request does need the side connection.
    assert!(worker::cancel(&options, 42, &tracker, 1).await.is_err());
    assert!(worker::cancel(&options, 0, &tracker, 1).await.is_err());
    assert!(!tracker.lock().await.start(2));
}

#[test]
fn definitions_quote_both_names() {
    assert_eq!(
        definition_sql("app", MySqlObjectKind::Trigger, "audit`x"),
        "SHOW CREATE TRIGGER `app`.`audit``x`"
    );
    assert_eq!(
        definition_sql("app", MySqlObjectKind::Function, "f"),
        "SHOW CREATE FUNCTION `app`.`f`"
    );
}

#[test]
fn catalog_rows_split_into_tree_kinds_and_report_truncation() {
    let row = |values: &[&str]| values.iter().map(|v| (*v).to_owned()).collect::<Vec<_>>();
    let objects = objects_of(
        "app".into(),
        vec![
            row(&["orders", "BASE TABLE"]),
            row(&["active_orders", "VIEW"]),
            row(&["history", "SYSTEM VERSIONED"]),
            row(&["seq", "SEQUENCE"]),
        ],
        vec![row(&["refresh", "PROCEDURE"]), row(&["total", "FUNCTION"])],
        vec![row(&["nightly"])],
        vec![row(&["orders_bi", "orders"])],
    );
    assert_eq!(objects.tables, ["orders", "history", "seq"]);
    assert_eq!(objects.views, ["active_orders"]);
    assert_eq!(
        objects.routines,
        [
            MySqlRoutine {
                name: "refresh".into(),
                kind: MySqlRoutineKind::Procedure
            },
            MySqlRoutine {
                name: "total".into(),
                kind: MySqlRoutineKind::Function
            }
        ]
    );
    assert_eq!(objects.events, ["nightly"]);
    assert_eq!(objects.triggers[0].table, "orders");
    assert!(!objects.truncated);

    let many = (0..=MYSQL_MAX_CATALOG_ITEMS)
        .map(|i| row(&[&format!("t{i}"), "BASE TABLE"]))
        .collect();
    let objects = objects_of("app".into(), many, vec![], vec![], vec![]);
    assert_eq!(objects.tables.len(), MYSQL_MAX_CATALOG_ITEMS);
    assert!(objects.truncated);
}

#[test]
fn connect_options_apply_defaults_and_leave_server_session_settings_alone() {
    let StoredConnection::MySQL(mut mysql) =
        stored(Environment::Development, SafeMode::Inherit, false)
    else {
        unreachable!()
    };
    let options = connect_options(&mysql).unwrap();
    assert_eq!(options.get_port(), 3306);
    assert_eq!(options.get_host(), "db.example");
    assert_eq!(options.get_database(), Some("orders"));
    assert!(matches!(options.get_ssl_mode(), MySqlSslMode::Preferred));

    mysql.database = " ".into();
    mysql.ssl = false;
    mysql.port = 3307;
    let options = connect_options(&mysql).unwrap();
    assert_eq!(options.get_database(), None);
    assert_eq!(options.get_port(), 3307);
    assert!(matches!(options.get_ssl_mode(), MySqlSslMode::Disabled));

    mysql.host = String::new();
    assert!(connect_options(&mysql).is_err());
}

#[test]
fn transport_failures_close_the_session_but_statement_errors_do_not() {
    let io = sqlx::Error::Io(std::io::Error::from(std::io::ErrorKind::ConnectionReset));
    assert!(worker::is_fatal(&io));
    assert!(matches!(
        worker::Failure::from_sqlx(io),
        worker::Failure::Fatal(reason) if reason.starts_with("Connection lost")
    ));
    assert!(worker::is_fatal(&sqlx::Error::Protocol(
        "bad packet".into()
    )));
    assert!(worker::is_fatal(&sqlx::Error::WorkerCrashed));
    assert!(!worker::is_fatal(&sqlx::Error::RowNotFound));
    assert!(matches!(
        worker::Failure::from_sqlx(sqlx::Error::RowNotFound),
        worker::Failure::Request(MySqlSessionError::Database(_))
    ));
}

#[test]
fn queries_follow_read_only_and_the_safety_policy() {
    let read_only = stored(Environment::Development, SafeMode::Disabled, true);
    let (_, audit, single_read) = authorize(&read_only, "SELECT * FROM orders", false).unwrap();
    assert_eq!(audit, AuditDisposition::NotRequired);
    assert!(single_read);
    for write in ["UPDATE orders SET paid = 1", "DROP TABLE orders"] {
        assert!(matches!(
            authorize(&read_only, write, true),
            Err(MySqlSessionError::ReadOnly(_))
        ));
    }

    // Production defaults to strict: writes need confirmation, then audit.
    let production = stored(Environment::Production, SafeMode::Inherit, false);
    assert!(authorize(&production, "SELECT 1", false).is_ok());
    assert!(matches!(
        authorize(&production, "INSERT INTO t VALUES (1)", false),
        Err(MySqlSessionError::NeedsConfirmation(statements)) if statements.len() == 1
    ));
    let (_, audit, single_read) = authorize(&production, "INSERT INTO t VALUES (1)", true).unwrap();
    assert_eq!(audit, AuditDisposition::RequiredAfterSuccess);
    assert!(!single_read);

    // Development with no policy runs anything without an audit.
    let development = stored(Environment::Development, SafeMode::Inherit, false);
    let (_, audit, _) = authorize(&development, "DELETE FROM t", false).unwrap();
    assert_eq!(audit, AuditDisposition::NotRequired);
}

#[test]
fn registry_retires_matching_sessions_and_refuses_after_close() {
    let registry = Registry::default();
    let (a, mut a_rx) = watch::channel(false);
    let (b, b_rx) = watch::channel(false);
    let key = registry.register("a", a).unwrap();
    registry.register("b", b).unwrap();
    registry.retire(Some("a"));
    assert!(*a_rx.borrow_and_update());
    assert!(!*b_rx.borrow());
    registry.remove(key);
    assert_eq!(registry.len(), 1);
    registry.close();
    assert!(*b_rx.borrow());
    let (c, _) = watch::channel(false);
    assert!(registry.register("c", c).is_none());
}

/// Live requests that are never cancelled share one id.
const ANY: MySqlRequestId = MySqlRequestId(0);

fn mysql_form(host: &str, port: u16) -> DevelopmentMySqlConnection {
    DevelopmentMySqlConnection {
        name: "Local MySQL".into(),
        host: host.into(),
        port,
        database: String::new(),
        user: "root".into(),
        environment: DevelopmentEnvironment::Development,
        safe_mode: DevelopmentSafeMode::Inherit,
        read_only: false,
        ssl: false,
        ssh_tunnel: None,
    }
}

/// Re-runs the named test in a child process: native profiles are one per
/// process.
fn in_child(name: &str, case: &str) -> bool {
    if std::env::var_os(case).is_some() {
        return false;
    }
    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            &format!("backend::mysql_sessions::tests::{name}"),
            "--nocapture",
            "--include-ignored",
        ])
        .env(case, "1")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    true
}

/// Open is one attempt: unknown, non-MySQL and unreachable connections fail
/// without leaving a registered session.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn open_refuses_unknown_and_non_mysql_records_and_reports_refused_sockets() {
    if in_child(
        "open_refuses_unknown_and_non_mysql_records_and_reports_refused_sockets",
        "DBUNK_MYSQL_SESSION_OPEN_TEST",
    ) {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();
    let backend = Backend::create_native_profile(&root.join("general"))
        .await
        .unwrap();
    backend
        .configure_development_credentials(DevelopmentStorageMode::PlainSqlite, None)
        .await
        .unwrap();
    assert!(matches!(
        backend.open_mysql_session("missing".into()).await,
        Err(MySqlSessionError::Unavailable(_))
    ));

    let file = root.join("app.sqlite");
    std::fs::write(&file, []).unwrap();
    let sqlite = backend
        .save_development_engine_connection(
            None,
            DevelopmentEngineConnection::SQLite(crate::backend::DevelopmentSqliteConnection {
                name: "File".into(),
                path: file.to_str().unwrap().into(),
                environment: DevelopmentEnvironment::Development,
                safe_mode: DevelopmentSafeMode::Inherit,
                read_only: false,
            }),
            String::new(),
            DevelopmentConnectionOrganization::default(),
        )
        .await
        .unwrap();
    assert_eq!(
        backend.open_mysql_session(sqlite.id).await.err(),
        Some(MySqlSessionError::Unavailable(
            "Connection is not a MySQL connection".into()
        ))
    );

    // Port 1 on loopback refuses immediately.
    let refused = backend
        .save_development_engine_connection(
            None,
            DevelopmentEngineConnection::MySQL(mysql_form("127.0.0.1", 1)),
            "pw".into(),
            DevelopmentConnectionOrganization::default(),
        )
        .await
        .unwrap();
    let started = std::time::Instant::now();
    match backend.open_mysql_session(refused.id).await {
        Err(MySqlSessionError::Unavailable(reason)) => {
            assert!(reason.contains("refused"), "{reason}");
            assert!(!reason.contains("pw"));
        }
        other => panic!("unexpected {:?}", other.map(|_| ())),
    }
    assert!(started.elapsed() < CONNECT_TIMEOUT);
    assert_eq!(backend.0.mysql.len(), 0);
    backend.shutdown().await.unwrap();
}

/// Live check against a disposable server: set `DBUNK_MYSQL_LIVE` to
/// `host:port:password` for a root account (for example a throwaway
/// `mysql:8` container). Exercises the tree, documents, cancel, explicit
/// disconnect and a server-side kill without any reconnect.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "needs DBUNK_MYSQL_LIVE"]
async fn live_session_tree_documents_cancel_and_failures() {
    let Some(target) = std::env::var("DBUNK_MYSQL_LIVE").ok() else {
        return;
    };
    if in_child(
        "live_session_tree_documents_cancel_and_failures",
        "DBUNK_MYSQL_SESSION_LIVE_TEST",
    ) {
        return;
    }
    let mut parts = target.splitn(3, ':');
    let host = parts.next().unwrap().to_owned();
    let port: u16 = parts.next().unwrap().parse().unwrap();
    let password = parts.next().unwrap_or_default().to_owned();

    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();
    let backend = Backend::create_native_profile(&root.join("general"))
        .await
        .unwrap();
    backend
        .configure_development_credentials(DevelopmentStorageMode::PlainSqlite, None)
        .await
        .unwrap();
    let saved = backend
        .save_development_engine_connection(
            None,
            DevelopmentEngineConnection::MySQL(mysql_form(&host, port)),
            password,
            DevelopmentConnectionOrganization::default(),
        )
        .await
        .unwrap();
    let session = backend.open_mysql_session(saved.id.clone()).await.unwrap();
    assert_eq!(session.status(), MySqlSessionStatus::Open);
    assert!(!session.server().version.is_empty());

    let setup = "DROP DATABASE IF EXISTS dbunk_live; CREATE DATABASE dbunk_live; \
        USE dbunk_live; \
        CREATE TABLE orders (id INT PRIMARY KEY, total DECIMAL(10,2), placed DATETIME, \
          note VARCHAR(20) NULL, raw VARBINARY(4), flags BIT(4)); \
        INSERT INTO orders VALUES (2, 12.50, '2026-10-04 12:00:00', NULL, 0x00ff, b'1010'), \
          (1, 3.00, '2026-10-03 08:30:00', 'first', 'ab', b'0001'); \
        CREATE VIEW big_orders AS SELECT * FROM orders WHERE total > 10; \
        CREATE PROCEDURE refresh() SELECT 1; \
        CREATE FUNCTION answer() RETURNS INT DETERMINISTIC RETURN 42; \
        CREATE TRIGGER orders_bi BEFORE INSERT ON orders FOR EACH ROW SET NEW.note = NEW.note; \
        CREATE EVENT nightly ON SCHEDULE EVERY 1 DAY DISABLE DO SELECT 1";
    let result = session.query(setup.into(), None, false, ANY).await.unwrap();
    assert!(result.result_sets >= 10, "{result:?}");
    // The script's `USE` is reported, and a tab with no database cannot run
    // in the database another script left selected.
    assert_eq!(result.database.as_deref(), Some("dbunk_live"));
    assert!(matches!(
        session.query("SELECT 1".into(), None, false, ANY).await,
        Err(MySqlSessionError::Database(reason)) if reason.contains("dbunk_live")
    ));

    let (databases, _) = session.databases().await.unwrap();
    assert!(databases.contains(&"dbunk_live".to_owned()));
    let objects = session.objects("dbunk_live".into()).await.unwrap();
    assert_eq!(objects.tables, ["orders"]);
    assert_eq!(objects.views, ["big_orders"]);
    assert_eq!(objects.routines.len(), 2);
    assert_eq!(objects.events, ["nightly"]);
    assert_eq!(objects.triggers[0].name, "orders_bi");

    // Values keep their server formatting; NULL stays distinct.
    let result = session
        .query(
            "SELECT id, total, placed, note, raw, flags FROM orders ORDER BY id".into(),
            Some("dbunk_live".into()),
            false,
            ANY,
        )
        .await
        .unwrap();
    assert_eq!(
        result.columns,
        ["id", "total", "placed", "note", "raw", "flags"]
    );
    assert_eq!(
        result.rows[1],
        [
            Some("2".into()),
            Some("12.50".into()),
            Some("2026-10-04 12:00:00".into()),
            None,
            Some("0x00ff".into()),
            Some("10".into())
        ]
    );
    let empty = session
        .query(
            "SELECT id FROM orders WHERE id < 0".into(),
            Some("dbunk_live".into()),
            false,
            ANY,
        )
        .await
        .unwrap();
    assert_eq!(empty.columns, ["id"]);
    assert!(empty.rows.is_empty());

    // A statement error keeps the session.
    assert!(matches!(
        session
            .query("SELEC 1".into(), Some("dbunk_live".into()), false, ANY)
            .await,
        Err(MySqlSessionError::Database(_))
    ));

    let first = session
        .browse("dbunk_live".into(), "orders".into(), 0, 1, ANY)
        .await
        .unwrap();
    assert_eq!(first.rows[0][0].as_deref(), Some("1"));
    assert!(first.has_more);
    let second = session
        .browse("dbunk_live".into(), "orders".into(), 1, 1, ANY)
        .await
        .unwrap();
    assert_eq!(second.rows[0][0].as_deref(), Some("2"));
    assert!(!second.has_more);

    let structure = session
        .structure("dbunk_live".into(), "orders".into())
        .await
        .unwrap();
    assert_eq!(structure.primary_key, ["id"]);
    assert_eq!(structure.columns.len(), 6);
    let view = session
        .definition(
            "dbunk_live".into(),
            MySqlObjectKind::View,
            "big_orders".into(),
        )
        .await
        .unwrap();
    assert!(view.contains("big_orders"), "{view}");
    let trigger = session
        .definition(
            "dbunk_live".into(),
            MySqlObjectKind::Trigger,
            "orders_bi".into(),
        )
        .await
        .unwrap();
    assert!(trigger.contains("BEFORE INSERT"), "{trigger}");

    // Cancel targets one request: a queued one is withdrawn and never runs,
    // the running one is interrupted, and the session survives.
    let live = Some("dbunk_live".to_owned());
    let sleeping = session.request_id();
    let running = {
        let (session, live) = (session.clone(), live.clone());
        tokio::spawn(async move {
            session
                .query("SELECT SLEEP(20)".into(), live, false, sleeping)
                .await
        })
    };
    tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    let queued_id = session.request_id();
    let queued = {
        let (session, live) = (session.clone(), live.clone());
        tokio::spawn(async move {
            session
                .query(
                    "INSERT INTO orders VALUES (3, 1, NOW(), NULL, NULL, NULL)".into(),
                    live,
                    false,
                    queued_id,
                )
                .await
        })
    };
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    assert_eq!(
        backend.cancel_mysql_query(&session, queued_id).await,
        Ok(MySqlCancel::Withdrawn)
    );
    assert_eq!(
        backend.cancel_mysql_query(&session, sleeping).await,
        Ok(MySqlCancel::Interrupted)
    );
    let cancelled = tokio::time::timeout(std::time::Duration::from_secs(5), running)
        .await
        .expect("cancel returned promptly")
        .unwrap();
    // SLEEP reports an interruption as the value 1, not an error.
    assert!(cancelled.is_ok() || matches!(cancelled, Err(MySqlSessionError::Cancelled)));
    assert_eq!(queued.await.unwrap(), Err(MySqlSessionError::Cancelled));
    // A finished request is never killed again.
    assert_eq!(
        backend.cancel_mysql_query(&session, sleeping).await,
        Ok(MySqlCancel::Finished)
    );
    let count = session
        .query("SELECT COUNT(*) FROM orders".into(), live, false, ANY)
        .await
        .unwrap();
    assert_eq!(
        count.rows[0][0].as_deref(),
        Some("2"),
        "withdrawn insert ran"
    );

    // Explicit disconnect retires the session: closed, no failure.
    backend
        .disconnect_development_connection(saved.id.clone())
        .await
        .unwrap();
    assert_eq!(
        tokio::time::timeout(std::time::Duration::from_secs(5), session.closed())
            .await
            .unwrap(),
        None
    );
    assert_eq!(
        session.query("SELECT 1".into(), None, false, ANY).await,
        Err(MySqlSessionError::Closed(None))
    );

    // A server-side kill fails the next request and closes for good.
    let session = backend.open_mysql_session(saved.id.clone()).await.unwrap();
    let observer = backend.open_mysql_session(saved.id.clone()).await.unwrap();
    let thread = session.0.cancel.1;
    observer
        .query(format!("KILL CONNECTION {thread}"), None, false, ANY)
        .await
        .unwrap();
    match session.query("SELECT 1".into(), None, false, ANY).await {
        Err(MySqlSessionError::Closed(Some(_))) => {}
        other => panic!("expected a closed session, got {other:?}"),
    }
    assert!(matches!(
        session.status(),
        MySqlSessionStatus::Closed(Some(_))
    ));
    assert!(matches!(
        session.query("SELECT 1".into(), None, false, ANY).await,
        Err(MySqlSessionError::Closed(Some(_)))
    ));
    observer
        .query("DROP DATABASE dbunk_live".into(), None, false, ANY)
        .await
        .unwrap();
    backend.shutdown().await.unwrap();
    assert_eq!(observer.closed().await, None);
    assert_eq!(backend.0.mysql.len(), 0);
}
