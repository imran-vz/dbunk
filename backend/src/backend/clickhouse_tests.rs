//! Native ClickHouse sessions against a loopback fake that speaks just enough
//! of the HTTP interface: one request per connection, basic auth, and canned
//! `JSONCompactEachRowWithNamesAndTypes` bodies.
use super::*;
use crate::backend::{
    DevelopmentClickHouseConnection, DevelopmentConnectionOrganization,
    DevelopmentEngineConnection, DevelopmentEnvironment, DevelopmentSafeMode,
    DevelopmentStorageMode,
};
use base64::Engine as _;
use std::sync::Mutex;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

#[test]
fn clickhouse_heads_are_classified_for_policy() {
    use crate::postgres::sql_class::StatementClass as Class;
    assert_eq!(classify("DESCRIBE TABLE t"), vec![Class::Read]);
    assert_eq!(classify("exists db.t"), vec![Class::Read]);
    assert_eq!(classify("SELECT 1"), vec![Class::Read]);
    assert_eq!(
        classify("OPTIMIZE TABLE t FINAL"),
        vec![Class::Ddl { destructive: false }]
    );
    assert_eq!(
        classify("DETACH TABLE t"),
        vec![Class::Ddl { destructive: true }]
    );
    assert_eq!(classify("SELECT 1; SELECT 2").len(), 2);
    assert!(classify("  -- nothing\n").is_empty());
}

#[test]
fn data_destroying_clickhouse_statements_are_destructive() {
    use crate::postgres::sql_class::StatementClass as Class;
    let destructive = Class::Ddl { destructive: true };
    for sql in [
        "ALTER TABLE t DELETE WHERE id = 1",
        "ALTER TABLE db.t ON CLUSTER c UPDATE x = 1 WHERE 1",
        "ALTER TABLE t DROP PARTITION 202401",
        "ALTER TABLE t DROP PART 'all_1_1_0'",
        "ALTER TABLE t DROP COLUMN c",
        "ALTER TABLE t ADD COLUMN c UInt8, DROP INDEX i",
        "ALTER TABLE t CLEAR COLUMN c IN PARTITION 1",
        "ALTER TABLE t DETACH PARTITION 1",
        "ALTER TABLE t REPLACE PARTITION 1 FROM other",
        "ALTER TABLE t MOVE PARTITION 1 TO TABLE other",
        "ALTER TABLE t MODIFY TTL d + INTERVAL 1 DAY",
        "CREATE OR REPLACE TABLE t (x UInt8) ENGINE = Memory",
        "REPLACE TABLE t AS SELECT 1",
        "EXCHANGE TABLES a AND b",
        "RENAME TABLE a TO b",
        "SYSTEM DROP REPLICA 'r' FROM TABLE t",
        "SYSTEM STOP MERGES",
        "KILL QUERY WHERE user = 'x'",
        "KILL MUTATION WHERE 1",
        "OPTIMIZE TABLE t FINAL DEDUPLICATE",
        "DETACH TABLE t",
    ] {
        assert_eq!(classify(sql), vec![destructive.clone()], "{sql}");
    }
    let additive = Class::Ddl { destructive: false };
    for sql in [
        "ALTER TABLE t ADD COLUMN c UInt8",
        "ALTER TABLE t MODIFY COMMENT 'x'",
        // Keywords inside parentheses are not ALTER actions.
        "ALTER TABLE t ADD COLUMN c UInt8 DEFAULT (SELECT 1 FROM drop_log)",
        "CREATE TABLE t (x UInt8) ENGINE = Memory",
        "OPTIMIZE TABLE t FINAL",
        "ATTACH TABLE t",
    ] {
        assert_eq!(classify(sql), vec![additive.clone()], "{sql}");
    }
}

#[test]
fn reads_through_external_table_functions_are_not_reads() {
    use crate::postgres::sql_class::StatementClass as Class;
    let write = Class::Dml {
        unbounded: false,
        destructive: false,
    };
    for sql in [
        "SELECT * FROM url('http://169.254.169.254/', CSV)",
        "SELECT * FROM s3('https://bucket/x.csv')",
        "SELECT count() FROM s3Cluster('c', 'https://b/x')",
        "SELECT * FROM file('/etc/passwd', LineAsString)",
        "SELECT * FROM remote('other:9000', db.t)",
        "SELECT * FROM remoteSecure('other:9440', db.t)",
        "SELECT * FROM mysql('h:3306', 'db', 't', 'u', 'p')",
        "SELECT * FROM postgresql('h:5432', 'db', 't', 'u', 'p')",
        "SELECT * FROM hdfs('hdfs://h/x', TSV)",
        "SELECT * FROM azureBlobStorage('conn', 'c', 'b')",
        "WITH x AS (SELECT * FROM url('http://h/')) SELECT * FROM x",
        "SELECT * FROM t WHERE id IN (SELECT id FROM url('http://h/', CSV))",
        "DESCRIBE TABLE s3('https://bucket/x.parquet')",
    ] {
        assert_eq!(classify(sql), vec![write.clone()], "{sql}");
    }
    // Plain reads, and identifiers that merely share a name, stay reads.
    for sql in [
        "SELECT url, file FROM logs",
        "SELECT * FROM remote_hosts",
        "SELECT `url` FROM t",
    ] {
        assert_eq!(classify(sql), vec![Class::Read], "{sql}");
    }
}

#[test]
fn browse_orders_by_choice_then_sorting_key() {
    assert_eq!(
        browse_sql("db", "t", None, "", 0, 50),
        "SELECT * FROM `db`.`t` LIMIT 50 OFFSET 0"
    );
    assert_eq!(
        browse_sql("db", "t", None, "day, toStartOfHour(ts)", 200, 101),
        "SELECT * FROM `db`.`t` ORDER BY day, toStartOfHour(ts) LIMIT 101 OFFSET 200"
    );
    assert_eq!(
        browse_sql("db", "t", Some(("n", false)), "day", 0, 10_000),
        format!("SELECT * FROM `db`.`t` ORDER BY `n`, day LIMIT {BROWSE_PAGE_ROWS} OFFSET 0")
    );
}

#[test]
fn server_config_dictionaries_open_without_a_database() {
    assert_eq!(
        browse_sql("", "geo's", Some(("id", true)), "", 0, 50),
        "SELECT * FROM dictionary('geo\\'s') ORDER BY `id` DESC LIMIT 50 OFFSET 0"
    );
    let described = ClickHouseRows {
        columns: ["name", "type", "default_type", "default_expression"]
            .into_iter()
            .map(|name| ClickHouseColumn {
                name: name.into(),
                type_name: "String".into(),
            })
            .collect(),
        rows: vec![
            vec![
                Some("id".into()),
                Some("UInt64".into()),
                Some(String::new()),
                Some(String::new()),
            ],
            vec![
                Some("name".into()),
                Some("String".into()),
                Some("DEFAULT".into()),
                Some("'x'".into()),
            ],
        ],
        ..Default::default()
    };
    let structure = config_dictionary_columns(described);
    assert_eq!(structure.columns.len(), 2);
    assert_eq!(structure.columns[0].type_name, "UInt64");
    assert_eq!(structure.columns[0].default, None);
    assert_eq!(structure.columns[1].default.as_deref(), Some("DEFAULT 'x'"));
    assert!(structure.ddl.is_empty() && structure.sorting_key.is_empty());
}

#[test]
fn connect_failures_never_echo_server_text() {
    let auth = connect_failure(&ClickHouseError::new(
        ClickHouseErrorKind::Server,
        "Code: 516. DB::Exception: default: Authentication failed: password hunter2",
    ));
    assert_eq!(auth.message, "Authentication failed");
    let other = connect_failure(&ClickHouseError::new(
        ClickHouseErrorKind::Server,
        "Code: 999. DB::Exception: secret detail",
    ));
    assert_eq!(
        other.message,
        "ClickHouse refused the connection (code 999)"
    );
    let lost = connect_failure(&ClickHouseError::new(
        ClickHouseErrorKind::Lost,
        "ClickHouse connection failed: tcp connect error",
    ));
    assert_eq!(lost.message, "Could not reach ClickHouse");
}

#[test]
fn identifiers_and_literals_are_escaped() {
    assert_eq!(quote_identifier("we`ird\\"), "`we\\`ird\\\\`");
    assert_eq!(literal("it's"), "'it\\'s'");
    // A trailing backslash cannot escape the closing quote.
    assert_eq!(literal("name\\"), "'name\\\\'");
}

#[derive(Clone, Default)]
struct Fake {
    /// Bodies of every request the fake received, in order.
    requests: Arc<Mutex<Vec<String>>>,
}

const AUTH: &str = "default:ch-secret";

fn rows(names: &[&str], types: &[&str], data: &[&str]) -> String {
    let mut body = format!(
        "{}\n{}\n",
        serde_json::to_string(names).unwrap(),
        serde_json::to_string(types).unwrap()
    );
    for row in data {
        body.push_str(row);
        body.push('\n');
    }
    body
}

fn respond(sql: &str) -> (u16, String) {
    if sql == "SELECT 1" {
        (200, rows(&["1"], &["UInt8"], &["[1]"]))
    } else if sql.contains("FROM system.databases ORDER BY") {
        (
            200,
            rows(
                &["name", "engine"],
                &["String", "String"],
                &[r#"["analytics","Atomic"]"#, r#"["system","Atomic"]"#],
            ),
        )
    } else if sql.contains("FROM system.tables") && sql.contains("is_temporary") {
        (
            200,
            rows(
                &["database", "name", "engine", "ddl", "uuid"],
                &["String"; 5],
                &[
                    r#"["analytics","daily","SummingMergeTree","",""]"#,
                    r#"["analytics","mv","MaterializedView","CREATE MATERIALIZED VIEW analytics.mv TO analytics.daily AS SELECT 1",""]"#,
                    r#"["analytics","recent","View","",""]"#,
                ],
            ),
        )
    } else if sql.contains("system.dictionaries") {
        (
            500,
            "Code: 497. DB::Exception: Not enough privileges. (ACCESS_DENIED)".into(),
        )
    } else if sql.starts_with("SELECT * FROM `analytics`.`daily`") {
        (
            200,
            rows(
                &["day", "hits"],
                &["Date", "UInt64"],
                &[r#"["2026-10-01","3"]"#, r#"["2026-10-02",null]"#],
            ),
        )
    } else if sql.starts_with("SELECT engine, create_table_query") {
        (
            200,
            rows(
                &["engine", "ddl", "rows", "bytes"],
                &["String"; 4],
                &[r#"["SummingMergeTree","CREATE TABLE analytics.daily (...)","2","1024"]"#],
            ),
        )
    } else if sql.starts_with("SELECT sorting_key, partition_key") {
        (
            200,
            rows(
                &["sorting_key", "partition_key", "sampling_key", "engine"],
                &["String"; 4],
                &[r#"["day","toYYYYMM(day)","","SummingMergeTree"]"#],
            ),
        )
    } else if sql.starts_with("SELECT sorting_key FROM system.tables") {
        // `recent` (a view) has no sorting key.
        let key = if sql.contains("'daily'") { "day" } else { "" };
        let row = format!("[\"{key}\"]");
        (200, rows(&["sorting_key"], &["String"], &[row.as_str()]))
    } else if sql.starts_with("SELECT * FROM `analytics`.`recent`") {
        (200, rows(&["n"], &["UInt8"], &["[1]"]))
    } else if sql.contains("FROM system.columns") {
        (
            200,
            rows(
                &[
                    "name",
                    "type",
                    "default_kind",
                    "default_expression",
                    "position",
                    "is_in_sorting_key",
                ],
                &["String", "String", "String", "String", "UInt64", "UInt8"],
                &[
                    r#"["day","Date","","","1",1]"#,
                    r#"["hits","UInt64","DEFAULT","0","2",0]"#,
                ],
            ),
        )
    } else if sql.contains("system.data_skipping_indices") {
        (
            200,
            rows(
                &["name", "expr", "type"],
                &["String"; 3],
                &[r#"["hits_idx","hits","minmax"]"#],
            ),
        )
    } else if sql.starts_with("INSERT") || sql.starts_with("KILL") {
        (200, String::new())
    } else {
        (
            500,
            format!("Code: 62. DB::Exception: Syntax error near {sql:?}"),
        )
    }
}

impl Fake {
    async fn start(self) -> u16 {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(async move {
            while let Ok((mut socket, _)) = listener.accept().await {
                let requests = self.requests.clone();
                tokio::spawn(async move {
                    let mut buffer = Vec::new();
                    let mut chunk = [0u8; 4096];
                    let (head_end, length) = loop {
                        let read = socket.read(&mut chunk).await.unwrap_or(0);
                        if read == 0 {
                            return;
                        }
                        buffer.extend_from_slice(&chunk[..read]);
                        if let Some(end) = buffer.windows(4).position(|w| w == b"\r\n\r\n") {
                            let head = String::from_utf8_lossy(&buffer[..end]).to_lowercase();
                            let length = head
                                .lines()
                                .find_map(|line| line.strip_prefix("content-length:"))
                                .and_then(|value| value.trim().parse::<usize>().ok())
                                .unwrap_or(0);
                            break (end + 4, length);
                        }
                    };
                    while buffer.len() < head_end + length {
                        let read = socket.read(&mut chunk).await.unwrap_or(0);
                        if read == 0 {
                            return;
                        }
                        buffer.extend_from_slice(&chunk[..read]);
                    }
                    let head = String::from_utf8_lossy(&buffer[..head_end]).to_string();
                    let sql =
                        String::from_utf8_lossy(&buffer[head_end..head_end + length]).to_string();
                    requests.lock().unwrap().push(sql.clone());
                    if sql == "SELECT slow" {
                        // Long enough that only an abort or KILL ends it.
                        tokio::time::sleep(std::time::Duration::from_secs(30)).await;
                    }
                    let expected = format!(
                        "authorization: basic {}",
                        base64::engine::general_purpose::STANDARD.encode(AUTH)
                    );
                    let (status, body) = if head
                        .lines()
                        .any(|line| line.eq_ignore_ascii_case(&expected))
                    {
                        respond(&sql)
                    } else {
                        (
                            516,
                            "Code: 516. DB::Exception: default: Authentication failed: password is incorrect".into(),
                        )
                    };
                    let response = format!(
                        "HTTP/1.1 {status} X\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                        body.len()
                    );
                    let _ = socket.write_all(response.as_bytes()).await;
                    let _ = socket.shutdown().await;
                });
            }
        });
        port
    }

    fn requests(&self) -> Vec<String> {
        self.requests.lock().unwrap().clone()
    }
}

fn form(
    port: u16,
    environment: DevelopmentEnvironment,
    read_only: bool,
) -> DevelopmentEngineConnection {
    DevelopmentEngineConnection::ClickHouse(DevelopmentClickHouseConnection {
        name: format!("Events {port} {environment:?} {read_only}"),
        host: "127.0.0.1".into(),
        port,
        database: String::new(),
        user: "default".into(),
        environment,
        safe_mode: DevelopmentSafeMode::Inherit,
        read_only,
        use_https: false,
        url_path: String::new(),
        ssh_tunnel: None,
    })
}

/// The process admits one native profile, so the scenario runs in a child.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn sessions_read_bounded_catalogs_and_enforce_policy() {
    const CASE: &str = "DBUNK_CLICKHOUSE_SESSION_TEST";
    if std::env::var_os(CASE).is_none() {
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "backend::clickhouse::tests::sessions_read_bounded_catalogs_and_enforce_policy",
                "--nocapture",
            ])
            .env(CASE, "1")
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
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
    let fake = Fake::default();
    let port = fake.clone().start().await;
    let save = |form: DevelopmentEngineConnection, secret: &str| {
        let backend = backend.clone();
        let secret = secret.to_string();
        async move {
            backend
                .save_development_engine_connection(
                    None,
                    form,
                    secret,
                    DevelopmentConnectionOrganization::default(),
                )
                .await
                .unwrap()
                .id
        }
    };
    let development = save(
        form(port, DevelopmentEnvironment::Development, false),
        "ch-secret",
    )
    .await;
    let read_only = save(
        form(port, DevelopmentEnvironment::Development, true),
        "ch-secret",
    )
    .await;
    let production = save(
        form(port, DevelopmentEnvironment::Production, false),
        "ch-secret",
    )
    .await;
    let wrong = save(form(port, DevelopmentEnvironment::Test, false), "nope").await;

    // Connect proves the endpoint once; a wrong password is classified.
    let session = backend
        .open_clickhouse_session(development.clone())
        .await
        .unwrap();
    assert_eq!(fake.requests(), ["SELECT 1"]);
    let failure = backend.open_clickhouse_session(wrong).await.err().unwrap();
    assert_eq!(failure.message, "Authentication failed");

    // Catalog: every database, kinds separate, MV target, and an unreadable
    // system.dictionaries reported rather than failing the tree.
    let catalog = session.catalog().await.unwrap();
    assert_eq!(catalog.databases.len(), 2);
    let analytics = &catalog.databases[0];
    assert_eq!(analytics.tables[0].engine, "SummingMergeTree");
    assert_eq!(analytics.views[0].name, "recent");
    assert_eq!(
        analytics.materialized_views[0].target.as_deref(),
        Some("analytics.daily")
    );
    assert!(catalog.dictionaries_error.unwrap().contains("privileges"));

    // Browse is a bounded read of one page with quoted identifiers, ordered
    // by the chosen column and then the table's sorting key.
    let page = session
        .browse("analytics", "daily", Some(("hits", true)), 0, 50, "b1")
        .await
        .unwrap();
    assert_eq!(page.rows[1], vec![Some("2026-10-02".into()), None]);
    assert!(!page.approximate_order);
    assert!(fake.requests().iter().any(|sql| sql
        == "SELECT * FROM `analytics`.`daily` ORDER BY `hits` DESC, day LIMIT 50 OFFSET 0"));
    // Unsorted pages follow the sorting key; the key is read once.
    session
        .browse("analytics", "daily", None, 100, 50, "b2")
        .await
        .unwrap();
    assert!(fake
        .requests()
        .iter()
        .any(|sql| sql == "SELECT * FROM `analytics`.`daily` ORDER BY day LIMIT 50 OFFSET 100"));
    let key_reads = |name: &str| {
        fake.requests()
            .iter()
            .filter(|sql| sql.starts_with("SELECT sorting_key FROM") && sql.contains(name))
            .count()
    };
    assert_eq!(key_reads("'daily'"), 1);
    // No sorting key and no chosen column: the page says its order is
    // approximate.
    let unordered = session
        .browse("analytics", "recent", None, 0, 50, "b3")
        .await
        .unwrap();
    assert!(unordered.approximate_order);
    assert!(fake
        .requests()
        .iter()
        .any(|sql| sql == "SELECT * FROM `analytics`.`recent` LIMIT 50 OFFSET 0"));

    // Structure: sorting key, defaults, skip indexes, engine and DDL, all
    // through the bounded reader.
    let structure = session.structure("analytics", "daily", "s1").await.unwrap();
    assert_eq!(structure.engine, "SummingMergeTree");
    assert_eq!(structure.total_rows, Some(2));
    assert_eq!(structure.sorting_key, ["day"]);
    assert_eq!(structure.partition_by.as_deref(), Some("toYYYYMM(day)"));
    assert!(structure.columns[0].in_sorting_key);
    assert_eq!(structure.columns[1].default.as_deref(), Some("DEFAULT 0"));
    assert_eq!(structure.skip_indexes[0].kind, "minmax");
    assert!(structure.ddl.starts_with("CREATE TABLE"));

    // Server errors keep their text for the query document.
    let error = session.query("SELEC 1", false, "q1").await.unwrap_err();
    assert_eq!(error.kind, ClickHouseErrorKind::Server);
    assert!(error.message.contains("Syntax error"));
    let multiple = session.query("SELECT 1; SELECT 2", false, "q2").await;
    assert_eq!(multiple.unwrap_err().kind, ClickHouseErrorKind::Refused);

    // Read-only refuses a write before any request is sent.
    let guarded = backend.open_clickhouse_session(read_only).await.unwrap();
    let before = fake.requests().len();
    let refused = guarded
        .query("INSERT INTO t VALUES (1)", true, "q3")
        .await
        .unwrap_err();
    assert_eq!(refused.kind, ClickHouseErrorKind::Refused);
    assert_eq!(fake.requests().len(), before);

    // Production asks first, runs once confirmed, and audits the override.
    let strict = backend
        .open_clickhouse_session(production.clone())
        .await
        .unwrap();
    let before = fake.requests().len();
    assert!(matches!(
        strict
            .query("INSERT INTO t VALUES (1)", false, "q4")
            .await
            .unwrap(),
        ClickHouseQueryOutcome::NeedsConfirmation(_)
    ));
    assert_eq!(fake.requests().len(), before);
    assert!(matches!(
        strict
            .query("INSERT INTO t VALUES (1)", true, "q5")
            .await
            .unwrap(),
        ClickHouseQueryOutcome::Rows(_)
    ));
    let audit = backend.load_safety_audit(production, None).await.unwrap();
    assert_eq!(audit.rows[0].command, AUDIT_COMMAND);

    // Dropping a running statement (document closed, stop, session ended)
    // sends one best-effort KILL for its id; a later explicit cancel of the
    // same id sends nothing more.
    let kills = |id: &str| {
        let statement = format!("KILL QUERY WHERE query_id = '{id}' ASYNC");
        fake.requests()
            .iter()
            .filter(|sql| **sql == statement)
            .count()
    };
    let started = |fake: &Fake, count: usize| {
        let fake = fake.clone();
        async move {
            for _ in 0..500 {
                let seen = fake
                    .requests()
                    .iter()
                    .filter(|sql| *sql == "SELECT slow")
                    .count();
                if seen >= count {
                    return;
                }
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            }
            panic!("slow statement never reached the server");
        }
    };
    let wait_for_kill = |id: &'static str| {
        let fake = fake.clone();
        async move {
            for _ in 0..500 {
                let statement = format!("KILL QUERY WHERE query_id = '{id}' ASYNC");
                if fake.requests().iter().any(|sql| *sql == statement) {
                    return;
                }
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            }
            panic!("no KILL for {id}");
        }
    };
    let running = {
        let session = session.clone();
        tokio::spawn(async move { session.query("SELECT slow", false, "slow-1").await })
    };
    started(&fake, 1).await;
    running.abort();
    wait_for_kill("slow-1").await;
    session.cancel("slow-1").await;
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    assert_eq!(kills("slow-1"), 1);

    // Closing a session kills what is still running before the route goes.
    let running = {
        let session = session.clone();
        tokio::spawn(async move { session.query("SELECT slow", false, "slow-2").await })
    };
    started(&fake, 2).await;
    session.close().await;
    assert_eq!(kills("slow-2"), 1);
    running.abort();
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    assert_eq!(kills("slow-2"), 1, "close already killed it");

    // A closed session refuses work without contacting the server.
    session.close().await;
    let before = fake.requests().len();
    assert_eq!(
        session.catalog().await.unwrap_err().kind,
        ClickHouseErrorKind::Refused
    );
    assert_eq!(fake.requests().len(), before);

    // An unreachable endpoint fails once, classified, with no retry.
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let closed_port = listener.local_addr().unwrap().port();
    drop(listener);
    let unreachable = save(
        form(closed_port, DevelopmentEnvironment::Development, false),
        "x",
    )
    .await;
    let failure = backend
        .open_clickhouse_session(unreachable)
        .await
        .err()
        .unwrap();
    assert_eq!(failure.kind, ClickHouseErrorKind::Lost);
    assert_eq!(failure.message, "Could not reach ClickHouse");
}
