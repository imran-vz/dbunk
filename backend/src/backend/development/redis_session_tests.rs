use super::*;

fn tokens(text: &str) -> Vec<String> {
    text.split_whitespace().map(str::to_owned).collect()
}

const OPEN: RedisPolicy = RedisPolicy {
    read_only: false,
    confirm_writes: false,
};

#[test]
fn keyspace_info_parses_totals_and_ignores_noise() {
    let info = "# Keyspace\r\ndb0:keys=12,expires=1,avg_ttl=0\r\ndb3:keys=4,expires=0\r\nbogus\r\ndbx:keys=1\r\n";
    assert_eq!(parse_keyspace(info), vec![(0, 12), (3, 4)]);
    assert_eq!(
        info_field("# Server\r\nredis_version:7.2.4\r\n", "redis_version").as_deref(),
        Some("7.2.4")
    );
}

#[test]
fn database_list_is_bounded_and_falls_back_without_config() {
    let keyspace = [(0, 12), (3, 4)];
    let listed = databases(Some(64), &keyspace, 0);
    assert_eq!(listed.len(), usize::from(REDIS_MAX_DATABASES));
    assert_eq!(listed[3], RedisDatabase { index: 3, keys: 4 });
    assert_eq!(listed[1].keys, 0);
    // CONFIG GET refused: databases with keys plus the default one.
    let indexes: Vec<u8> = databases(None, &keyspace, 5)
        .into_iter()
        .map(|db| db.index)
        .collect();
    assert_eq!(indexes, vec![0, 3, 5]);
}

#[test]
fn console_admission_applies_guard_policy_and_bounds() {
    // Reads always run; writes run on an open connection.
    assert_eq!(admit(&tokens("GET k"), false, OPEN), None);
    assert_eq!(admit(&tokens("SET k v"), false, OPEN), None);
    // Destructive commands need confirmation even on an open connection.
    assert!(matches!(
        admit(&tokens("flushdb"), false, OPEN),
        Some(RedisConsoleOutcome::NeedsConfirmation { command, .. }) if command == "FLUSHDB"
    ));
    assert_eq!(admit(&tokens("FLUSHDB"), true, OPEN), None);
    // Read-only fails closed for writes and unknown commands, even confirmed.
    let read_only = RedisPolicy {
        read_only: true,
        confirm_writes: true,
    };
    for text in ["SET k v", "DEL k", "UNKNOWNCMD"] {
        assert!(
            matches!(
                admit(&tokens(text), true, read_only),
                Some(RedisConsoleOutcome::Refused { .. })
            ),
            "{text}"
        );
    }
    assert_eq!(admit(&tokens("HGETALL h"), false, read_only), None);
    // Protected connections confirm writes, not reads.
    let protected = RedisPolicy {
        read_only: false,
        confirm_writes: true,
    };
    assert!(matches!(
        admit(&tokens("set k v"), false, protected),
        Some(RedisConsoleOutcome::NeedsConfirmation { command, .. }) if command == "SET"
    ));
    assert_eq!(admit(&tokens("SET k v"), true, protected), None);
    assert_eq!(admit(&tokens("GET k"), false, protected), None);
    // Pub/Sub, blocking commands and oversized input never run.
    for refused in [
        tokens("SUBSCRIBE ch"),
        tokens("BLPOP q 0"),
        vec!["SET".into(), "k".into(), "x".repeat(REDIS_COMMAND_BYTES)],
        vec!["MGET".into(); REDIS_COMMAND_TOKENS + 1],
    ] {
        assert!(matches!(
            admit(&refused, true, OPEN),
            Some(RedisConsoleOutcome::Refused { .. })
        ));
    }
}

#[test]
fn replies_are_bounded_by_nodes_and_bytes() {
    let mut budget = Budget::default();
    let large = SerializedValue::Array {
        value: (0..REDIS_REPLY_NODES + 50)
            .map(|value| SerializedValue::Int {
                value: value as i64,
            })
            .collect(),
    };
    let RedisValue::Array(items) = bound(large, &mut budget) else {
        panic!("array expected");
    };
    assert!(budget.truncated);
    assert!(matches!(items.last(), Some(RedisValue::Omitted(n)) if *n > 50));

    let mut budget = Budget::default();
    let text = "é".repeat(REDIS_REPLY_BYTES); // two bytes per char
    let RedisValue::Text(cut) = bound(
        SerializedValue::String {
            value: text,
            encoding: "utf8".into(),
        },
        &mut budget,
    ) else {
        panic!("text expected");
    };
    assert!(budget.truncated);
    assert!(cut.ends_with('…'));
    assert!(cut.len() <= REDIS_REPLY_BYTES + '…'.len_utf8());

    let mut budget = Budget::default();
    assert_eq!(
        bound(
            SerializedValue::String {
                value: "0x00ff".into(),
                encoding: "hex".into(),
            },
            &mut budget
        ),
        RedisValue::Bytes("0x00ff".into())
    );
    assert!(!budget.truncated);
}

/// Against a disposable server only; it writes and flushes db 9:
/// `DBUNK_REDIS_TEST_PORT=16379 cargo test --features isolated-profile \
///   redis_session -- --ignored`
#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs a disposable Redis on DBUNK_REDIS_TEST_PORT"]
async fn live_session_browses_runs_inspects_and_latches_loss() {
    let port: u16 = std::env::var("DBUNK_REDIS_TEST_PORT")
        .expect("DBUNK_REDIS_TEST_PORT")
        .parse()
        .unwrap();
    let stored = RedisStoredConnection {
        organization: Default::default(),
        id: "live".into(),
        name: "live".into(),
        database: String::new(),
        host: "127.0.0.1".into(),
        port,
        user: String::new(),
        password: String::new(),
        role: String::new(),
        environment: crate::Environment::Development,
        safe_mode: crate::SafeMode::Inherit,
        last_activity_at: None,
        db_number: 9,
        use_tls: false,
        verify_tls_cert: true,
        read_only: false,
        ssh_tunnel: crate::SshTunnelConfig::default(),
    };
    let session = RedisSession::connect(stored.clone(), None).await.unwrap();
    let run = |text: &str, confirmed: bool| session.run(tokens(text), confirmed);
    assert!(matches!(
        run("FLUSHDB", true).await.unwrap(),
        RedisConsoleOutcome::Reply { .. }
    ));
    for (command, _) in [
        ("SET s hello", ()),
        ("HSET h a 1 b 2", ()),
        ("RPUSH l x y z", ()),
        ("SADD set m", ()),
        ("ZADD z 1 one", ()),
        ("XADD st * f v", ()),
    ] {
        run(command, false).await.unwrap();
    }
    // A server error is a reply, not a session failure.
    let RedisConsoleOutcome::Reply { value, db, .. } = run("HGET s x", false).await.unwrap() else {
        panic!("reply expected");
    };
    assert!(matches!(value, RedisValue::Error(text) if text.contains("WRONGTYPE")));
    assert_eq!(db, 9);

    let overview = session.overview().await.unwrap();
    assert_eq!(
        overview
            .databases
            .iter()
            .find(|db| db.index == 9)
            .unwrap()
            .keys,
        6
    );
    let mut kinds = Vec::new();
    let mut cursor = None;
    loop {
        let page = session.scan(9, cursor, "*").await.unwrap();
        kinds.extend(page.keys.into_iter().map(|key| key.kind));
        cursor = page.next_cursor;
        if cursor.is_none() {
            break;
        }
    }
    kinds.sort();
    assert_eq!(kinds, ["hash", "list", "set", "stream", "string", "zset"]);

    let hash = session.inspect(9, "h".into()).await.unwrap();
    assert_eq!(hash.kind, "hash");
    assert!(matches!(hash.value, RedisKeyValue::Hash(ref entries) if entries.len() == 2));
    let missing = session.inspect(9, "nope".into()).await.unwrap();
    assert_eq!(missing.value, RedisKeyValue::Missing);
    // The console's SELECT does not move the browse lane.
    run("SELECT 8", false).await.unwrap();
    assert_eq!(session.inspect(9, "s".into()).await.unwrap().kind, "string");

    // A killed connection latches the session; nothing reconnects.
    let RedisConsoleOutcome::Reply {
        value: RedisValue::Int(console),
        ..
    } = run("CLIENT ID", false).await.unwrap()
    else {
        panic!("client id expected");
    };
    let killer = RedisSession::connect(stored, None).await.unwrap();
    killer
        .run(tokens(&format!("CLIENT KILL ID {console}")), true)
        .await
        .unwrap();
    let error = session.run(tokens("PING"), false).await.unwrap_err();
    assert!(matches!(error, RedisSessionError::Lost(_)), "{error:?}");
    // The browse lane is healthy, but the session stays closed.
    assert!(matches!(
        session.scan(9, None, "*").await,
        Err(RedisSessionError::Lost(_))
    ));
    killer.run(tokens("FLUSHDB"), true).await.unwrap();
}

/// The full open path from a saved record in a general profile: stored
/// policy, the credential journal and a single bounded attempt. With
/// `DBUNK_REDIS_TEST_PORT` it also opens a real session.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn saved_records_open_with_their_policy_and_fail_once() {
    const CASE: &str = "DBUNK_REDIS_SESSION_TEST";
    if std::env::var_os(CASE).is_none() {
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "backend::development::connections::redis_session::tests::saved_records_open_with_their_policy_and_fail_once",
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
    let refused = {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        listener.local_addr().unwrap().port()
    };
    let form = |port: u16, read_only: bool| {
        DevelopmentEngineConnection::Redis(DevelopmentRedisConnection {
            name: "Cache".into(),
            host: "127.0.0.1".into(),
            port,
            db_number: 9,
            user: String::new(),
            environment: DevelopmentEnvironment::Staging,
            safe_mode: DevelopmentSafeMode::Inherit,
            read_only,
            use_tls: false,
            verify_tls_cert: true,
            ssh_tunnel: None,
        })
    };
    let save = |form| {
        backend.save_development_engine_connection(
            None,
            form,
            String::new(),
            DevelopmentConnectionOrganization::default(),
        )
    };
    let down = save(form(refused, false)).await.unwrap();
    let started = std::time::Instant::now();
    let error = match backend.open_redis_session(down.id.clone()).await {
        Ok(_) => panic!("nothing listens on {refused}"),
        Err(error) => error,
    };
    assert!(error.contains("refused"), "{error}");
    assert!(started.elapsed() < REDIS_OPEN_TIMEOUT);

    let sqlite = root.join("app.sqlite");
    std::fs::write(&sqlite, []).unwrap();
    let other = save(DevelopmentEngineConnection::SQLite(
        DevelopmentSqliteConnection {
            name: "File".into(),
            path: sqlite.to_str().unwrap().into(),
            environment: DevelopmentEnvironment::Development,
            safe_mode: DevelopmentSafeMode::Inherit,
            read_only: false,
        },
    ))
    .await
    .unwrap();
    assert!(backend.open_redis_session(other.id).await.is_err());
    assert!(backend.open_redis_session("missing".into()).await.is_err());

    let Some(port) = std::env::var("DBUNK_REDIS_TEST_PORT")
        .ok()
        .and_then(|port| port.parse().ok())
    else {
        return;
    };
    let live = save(form(port, true)).await.unwrap();
    let session = backend.open_redis_session(live.id).await.unwrap();
    assert_eq!(
        session.policy(),
        RedisPolicy {
            read_only: true,
            confirm_writes: true,
        }
    );
    assert_eq!(session.default_db(), 9);
    assert!(matches!(
        session.run(tokens("SET k v"), true).await.unwrap(),
        RedisConsoleOutcome::Refused { .. }
    ));
    assert!(matches!(
        session.run(tokens("GET k"), false).await.unwrap(),
        RedisConsoleOutcome::Reply { .. }
    ));
}
