use super::*;

fn tokens(text: &str) -> Vec<Vec<u8>> {
    text.split_whitespace()
        .map(|word| word.as_bytes().to_vec())
        .collect()
}

const READ_ONLY: RedisPolicy = RedisPolicy {
    read_only: true,
    confirm_writes: true,
};

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
    assert_eq!(admit(&tokens("GET k"), false, OPEN, false), None);
    assert_eq!(admit(&tokens("SET k v"), false, OPEN, false), None);
    // Destructive commands need confirmation even on an open connection.
    assert!(matches!(
        admit(&tokens("flushdb"), false, OPEN, false),
        Some(RedisConsoleOutcome::NeedsConfirmation { command, .. }) if command == "FLUSHDB"
    ));
    assert_eq!(admit(&tokens("FLUSHDB"), true, OPEN, false), None);
    // Read-only fails closed for writes and unknown commands, even confirmed.
    for text in ["SET k v", "DEL k", "UNKNOWNCMD"] {
        assert!(
            matches!(
                admit(&tokens(text), true, READ_ONLY, false),
                Some(RedisConsoleOutcome::Refused { .. })
            ),
            "{text}"
        );
    }
    assert_eq!(admit(&tokens("HGETALL h"), false, READ_ONLY, false), None);
    // Protected connections confirm writes, not reads.
    let protected = RedisPolicy {
        read_only: false,
        confirm_writes: true,
    };
    assert!(matches!(
        admit(&tokens("set k v"), false, protected, false),
        Some(RedisConsoleOutcome::NeedsConfirmation { command, .. }) if command == "SET"
    ));
    assert_eq!(admit(&tokens("SET k v"), true, protected, false), None);
    assert_eq!(admit(&tokens("GET k"), false, protected, false), None);
    // Pub/Sub, blocking commands and oversized input never run.
    for refused in [
        tokens("SUBSCRIBE ch"),
        tokens("BLPOP q 0"),
        vec![
            b"SET".to_vec(),
            b"k".to_vec(),
            vec![b'x'; REDIS_COMMAND_BYTES],
        ],
        vec![b"MGET".to_vec(); REDIS_COMMAND_TOKENS + 1],
    ] {
        assert!(matches!(
            admit(&refused, true, OPEN, false),
            Some(RedisConsoleOutcome::Refused { .. })
        ));
    }
}

#[test]
fn read_only_refuses_before_asking_for_confirmation() {
    // Destructive commands on a read-only connection are refused outright,
    // not offered for confirmation first.
    for text in ["FLUSHDB", "CONFIG SET maxmemory 1", "SCRIPT FLUSH"] {
        assert!(
            matches!(
                admit(&tokens(text), false, READ_ONLY, false),
                Some(RedisConsoleOutcome::Refused { .. })
            ),
            "{text}"
        );
    }
    // Read-only scripts cannot write but can loop: refused on read-only,
    // run unconfirmed elsewhere.
    for text in ["EVAL_RO return 1 0", "EVALSHA_RO abc 0", "FCALL_RO f 0"] {
        assert!(
            matches!(
                admit(&tokens(text), true, READ_ONLY, false),
                Some(RedisConsoleOutcome::Refused { .. })
            ),
            "{text}"
        );
        assert_eq!(admit(&tokens(text), false, OPEN, false), None, "{text}");
    }
    // KEYS is a read: confirmed on any connection, with a SCAN suggestion.
    assert!(matches!(
        admit(&tokens("KEYS *"), false, READ_ONLY, false),
        Some(RedisConsoleOutcome::NeedsConfirmation { reason, .. }) if reason.contains("SCAN")
    ));
}

#[test]
fn select_is_refused_inside_multi_and_outside_the_tracked_range() {
    assert_eq!(admit(&tokens("SELECT 3"), false, OPEN, false), None);
    assert!(matches!(
        admit(&tokens("select 3"), false, OPEN, true),
        Some(RedisConsoleOutcome::Refused { .. })
    ));
    assert!(matches!(
        admit(&tokens("SELECT 300"), false, OPEN, false),
        Some(RedisConsoleOutcome::Refused { .. })
    ));
    // Other commands queue normally inside MULTI.
    assert_eq!(admit(&tokens("GET k"), false, OPEN, true), None);
}

#[test]
fn non_utf8_arguments_are_admitted_as_raw_bytes() {
    let command = vec![b"GET".to_vec(), vec![0xff, 0xfe, b'k']];
    assert_eq!(admit(&command, false, OPEN, false), None);
    let check = preflight(&command).expect("GET is measured");
    assert_eq!(check.keys, vec![vec![0xff, 0xfe, b'k']]);
    assert!(check.hint.contains(r#""\xff\xfek""#), "{}", check.hint);
}

#[test]
fn whole_value_reads_are_measured_and_bounded_forms_are_not() {
    let probe = |text: &str| preflight(&tokens(text)).map(|check| (check.probe, check.window));
    assert_eq!(probe("GET k"), Some(("STRLEN", Window::All)));
    assert_eq!(probe("getdel k"), Some(("STRLEN", Window::All)));
    assert_eq!(
        preflight(&tokens("MGET a b c")).map(|check| check.keys.len()),
        Some(3)
    );
    assert_eq!(
        probe("LRANGE l 0 -1"),
        Some(("LLEN", Window::Rank { start: 0, stop: -1 }))
    );
    assert_eq!(probe("LRANGE l 0 99"), None);
    assert_eq!(probe("LRANGE l"), None, "malformed: the server answers");
    assert_eq!(probe("HGETALL h"), Some(("HLEN", Window::All)));
    assert_eq!(probe("HVALS h"), Some(("HLEN", Window::All)));
    assert_eq!(probe("HKEYS h"), Some(("HLEN", Window::All)));
    assert_eq!(probe("SMEMBERS s"), Some(("SCARD", Window::All)));
    assert_eq!(
        probe("ZRANGE z 0 -1 WITHSCORES"),
        Some(("ZCARD", Window::Rank { start: 0, stop: -1 }))
    );
    assert_eq!(
        probe("ZRANGE z -inf +inf BYSCORE"),
        Some(("ZCARD", Window::All))
    );
    assert_eq!(probe("ZRANGE z -inf +inf BYSCORE LIMIT 0 50"), None);
    assert_eq!(
        probe("ZRANGEBYSCORE z -inf +inf"),
        Some(("ZCARD", Window::All))
    );
    assert_eq!(probe("ZRANGEBYLEX z - + LIMIT 0 10"), None);
    assert_eq!(probe("XRANGE s - +"), Some(("XLEN", Window::All)));
    assert_eq!(probe("XRANGE s - + COUNT 10"), None);
    assert_eq!(probe("KEYS user:*"), Some(("DBSIZE", Window::All)));
    assert_eq!(probe("HGET h f"), None);
    assert_eq!(probe("GETRANGE k 0 10"), None);
}

#[test]
fn measured_reads_over_their_limit_are_refused_with_a_bounded_form() {
    let check = preflight(&tokens("GET big")).unwrap();
    assert_eq!(judge(&check, REDIS_CONSOLE_STRING_BYTES), None);
    let refusal = judge(&check, REDIS_CONSOLE_STRING_BYTES + 1).unwrap();
    assert!(refusal.contains("GETRANGE big 0"), "{refusal}");

    let check = preflight(&tokens("LRANGE l 0 -1")).unwrap();
    assert_eq!(judge(&check, REDIS_CONSOLE_ELEMENTS), None);
    assert!(judge(&check, REDIS_CONSOLE_ELEMENTS + 1)
        .unwrap()
        .contains("LRANGE l 0 99"));
    // A tail window of a huge list is small.
    let check = preflight(&tokens("LRANGE l -10 -1")).unwrap();
    assert_eq!(judge(&check, 1_000_000), None);

    let refusal = judge(&preflight(&tokens("HGETALL h")).unwrap(), 50_000).unwrap();
    assert!(refusal.contains("HSCAN h 0 COUNT 100"), "{refusal}");
    let refusal = judge(&preflight(&tokens("KEYS user:*")).unwrap(), 50_000).unwrap();
    assert!(
        refusal.contains("SCAN 0 MATCH user:* COUNT 100"),
        "{refusal}"
    );
}

#[test]
fn rank_ranges_follow_redis_index_rules() {
    assert_eq!(range_len(100, 0, -1), 100);
    assert_eq!(range_len(100, -10, -1), 10);
    assert_eq!(range_len(100, 90, 1_000), 10);
    assert_eq!(range_len(100, -1_000, 4), 5);
    assert_eq!(range_len(100, 50, 10), 0);
    assert_eq!(range_len(100, 200, 300), 0);
    assert_eq!(range_len(0, 0, -1), 0);
    assert_eq!(range_len(u64::MAX, i64::MIN, i64::MAX), i64::MAX as u64);
}

#[test]
fn quoted_arguments_escape_what_the_tokenizer_would_split_or_decode() {
    assert_eq!(quote_arg(b"user:1"), "user:1");
    assert_eq!(quote_arg(b""), r#""""#);
    assert_eq!(quote_arg(b"a b"), r#""a b""#);
    assert_eq!(quote_arg(b"say \"hi\"\n"), r#""say \"hi\"\n""#);
    assert_eq!(quote_arg(b"it's"), r#""it's""#);
    assert_eq!(quote_arg(&[0xff, 0x00]), r#""\xff\x00""#);
}

#[test]
fn raw_replies_are_bounded_while_walking_the_value() {
    // Arrays stop at the node budget and report the rest.
    let mut budget = Budget::default();
    let large = redis::Value::Array(
        (0..REDIS_REPLY_NODES + 50)
            .map(|value| redis::Value::Int(value as i64))
            .collect(),
    );
    let RedisValue::Array(items) = bound_reply(large, &mut budget) else {
        panic!("array expected");
    };
    assert!(budget.truncated);
    assert!(matches!(items.last(), Some(RedisValue::Omitted(n)) if *n > 50));

    // Text is cut on a character boundary.
    let mut budget = Budget::default();
    let text = "é".repeat(REDIS_REPLY_BYTES).into_bytes();
    let RedisValue::Text(cut) = bound_reply(redis::Value::BulkString(text), &mut budget) else {
        panic!("text expected");
    };
    assert!(budget.truncated);
    assert!(cut.ends_with('…'));
    assert!(cut.len() <= REDIS_REPLY_BYTES + '…'.len_utf8());

    // Binary values become hex, and only the kept prefix is expanded.
    let mut budget = Budget::default();
    assert_eq!(
        bound_reply(redis::Value::BulkString(vec![0x00, 0xff]), &mut budget),
        RedisValue::Bytes("0x00ff".into())
    );
    assert!(!budget.truncated);
    let mut budget = Budget::default();
    let RedisValue::Bytes(hex) = bound_reply(
        redis::Value::BulkString(vec![0xff; REDIS_REPLY_BYTES]),
        &mut budget,
    ) else {
        panic!("bytes expected");
    };
    assert!(budget.truncated);
    assert!(hex.len() <= REDIS_REPLY_BYTES + '…'.len_utf8());
    assert!(hex.starts_with("0xffff") && hex.ends_with('…'));

    // Once the byte budget is spent, later elements are counted, not kept.
    let mut budget = Budget::default();
    let reply = redis::Value::Array(vec![
        redis::Value::BulkString(vec![b'a'; REDIS_REPLY_BYTES + 10]),
        redis::Value::BulkString(b"b".to_vec()),
        redis::Value::BulkString(b"c".to_vec()),
    ]);
    let RedisValue::Array(items) = bound_reply(reply, &mut budget) else {
        panic!("array expected");
    };
    assert_eq!(items.len(), 2);
    assert_eq!(items[1], RedisValue::Omitted(2));

    // Maps flatten to field/value pairs; statuses, nils and ints keep their kind.
    let mut budget = Budget::default();
    assert_eq!(
        bound_reply(
            redis::Value::Map(vec![(
                redis::Value::BulkString(b"f".to_vec()),
                redis::Value::Nil
            )]),
            &mut budget
        ),
        RedisValue::Array(vec![RedisValue::Text("f".into()), RedisValue::Nil])
    );
    assert_eq!(
        bound_reply(redis::Value::Okay, &mut budget),
        RedisValue::Status("OK".into())
    );
    assert_eq!(
        bound_reply(redis::Value::BulkString(b"a\x01".to_vec()), &mut budget),
        RedisValue::Bytes("0x6101".into())
    );
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
