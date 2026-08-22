use std::time::Duration;

use crate::postgres::connect_spec::ResolvedPostgresConnectSpec;
use crate::query_session::postgres as session_postgres;
use crate::PgDriverOptions;

use super::postgres;
use super::protocol::*;
use super::TableBrowseManager;

fn live_spec(port: u16, tls_prefer: bool, connection_id: &str) -> ResolvedPostgresConnectSpec {
    ResolvedPostgresConnectSpec {
        connection_id: connection_id.into(),
        host: "127.0.0.1".into(),
        port,
        database: "dbunk_demo".into(),
        user: "dbunk".into(),
        password: "dbunk".into(),
        tls_prefer,
        connect_timeout: Some(Duration::from_secs(5)),
        driver_options: PgDriverOptions::default(),
        safety_policy: Default::default(),
    }
}

fn browse_payload(connection_id: &str, schema: &str, table: &str) -> BrowseTableDataPayload {
    BrowseTableDataPayload {
        connection_id: connection_id.into(),
        tab_id: "tab".into(),
        request_id: 1,
        schema: schema.into(),
        table: table.into(),
        filters: Vec::new(),
        sort: Vec::new(),
        page_request: BrowsePageRequest::Offset { page: 1 },
        page_size: 25,
        count_policy: BrowseCountPolicy::None,
        refresh_structure: false,
    }
}

#[tokio::test]
#[ignore = "requires pnpm db:postgres"]
async fn table_browse_live_typed_filters_casts_and_identity() {
    let spec = live_spec(15432, true, "browse-typed");
    let manager = TableBrowseManager::new();
    let mut payload = browse_payload("browse-typed", "crm", "accounts");
    payload.filters = vec![
        BrowseFilter::Comparison {
            column: "tier".into(),
            operator: ComparisonOperator::Eq,
            value: "enterprise".into(),
        },
        BrowseFilter::Comparison {
            column: "credit_limit".into(),
            operator: ComparisonOperator::Gte,
            value: "1".into(),
        },
        BrowseFilter::TextMatch {
            column: "name".into(),
            operator: TextMatchOperator::Contains,
            value: "Acme".into(),
        },
        BrowseFilter::TextMatch {
            column: "tags".into(),
            operator: TextMatchOperator::Contains,
            value: "priority".into(),
        },
    ];
    let result = manager.browse(spec.clone(), payload).await.expect("browse");
    assert_eq!(result.identity.kind, BrowseIdentityKind::PrimaryKey);
    assert_eq!(result.rows.len(), 1);
    assert!(result.rows[0]
        .iter()
        .any(|cell| cell.is_none() || cell.as_deref() != Some("NULL")));
    assert!(result.inspection.sql.contains("($1::text)::"));
    assert!(!result.inspection.sql.contains("enterprise"));

    let mut contacts = browse_payload("browse-typed", "crm", "contacts");
    contacts.tab_id = "contacts".into();
    contacts.request_id = 2;
    contacts.filters = vec![
        BrowseFilter::Comparison {
            column: "is_primary".into(),
            operator: ComparisonOperator::Eq,
            value: "true".into(),
        },
        BrowseFilter::InList {
            column: "id".into(),
            values: vec!["101".into(), "201".into(), "301".into()],
        },
    ];
    let contacts = manager.browse(spec, contacts).await.expect("contacts");
    assert!(!contacts.rows.is_empty());
}

#[tokio::test]
#[ignore = "requires pnpm db:postgres"]
async fn table_browse_live_ilike_escape_and_raw_sql_boundaries() {
    let spec = live_spec(15432, true, "browse-raw");
    let manager = TableBrowseManager::new();
    let mut payload = browse_payload("browse-raw", "crm", "accounts");
    payload.filters = vec![BrowseFilter::TextMatch {
        column: "name".into(),
        operator: TextMatchOperator::Contains,
        value: r"Acme%".into(),
    }];
    let escaped = manager
        .browse(spec.clone(), payload.clone())
        .await
        .expect("escaped ilike");
    assert!(escaped.rows.is_empty());

    payload.filters = vec![BrowseFilter::RawSql {
        text: "name ILIKE 'Acme%'".into(),
    }];
    payload.request_id = 2;
    let raw = manager
        .browse(spec.clone(), payload.clone())
        .await
        .expect("raw");
    assert_eq!(raw.rows.len(), 1);

    payload.filters = vec![BrowseFilter::RawSql {
        text: "1=1); DROP TABLE crm.accounts; --".into(),
    }];
    payload.request_id = 3;
    let smuggle = manager.browse(spec.clone(), payload).await;
    assert!(smuggle.is_err());

    let admin = session_postgres::connect(&spec).await.expect("admin");
    let schema = format!("browse_live_{}", uuid::Uuid::new_v4().simple());
    admin
        .client
        .batch_execute(&format!(
            r#"
            CREATE SCHEMA {schema};
            CREATE TABLE {schema}.t (id int PRIMARY KEY);
            INSERT INTO {schema}.t VALUES (2);
            CREATE FUNCTION {schema}.touch() RETURNS boolean
            LANGUAGE plpgsql AS $$
            BEGIN
              INSERT INTO {schema}.t VALUES (1);
              RETURN true;
            END $$;
            "#
        ))
        .await
        .expect("schema");
    let mut write = browse_payload("browse-raw", &schema, "t");
    write.tab_id = "write".into();
    write.request_id = 4;
    write.filters = vec![BrowseFilter::RawSql {
        text: format!("{schema}.touch()"),
    }];
    let rejected = manager.browse(spec, write).await;
    assert!(rejected.is_err(), "{rejected:?}");
    if let Err(TableBrowseError::Database { code, .. }) = &rejected {
        assert_eq!(code.as_deref(), Some("25006"));
    }
    let leftover = admin
        .client
        .query_one(&format!("SELECT count(*) FROM {schema}.t"), &[])
        .await
        .expect("count");
    let count: i64 = leftover.get(0);
    assert_eq!(count, 1);
    admin
        .client
        .batch_execute(&format!("DROP SCHEMA {schema} CASCADE"))
        .await
        .ok();
}

#[tokio::test]
#[ignore = "requires pnpm db:postgres"]
async fn table_browse_live_keyset_offset_counts_and_structure() {
    let spec = live_spec(15432, true, "browse-pages");
    let manager = TableBrowseManager::new();
    let mut payload = browse_payload("browse-pages", "crm", "accounts");
    payload.page_size = 1;
    payload.page_request = BrowsePageRequest::Keyset { cursor: None };
    payload.count_policy = BrowseCountPolicy::Estimated;
    let first = manager
        .browse(spec.clone(), payload.clone())
        .await
        .expect("first");
    assert_eq!(first.page_info.mode, BrowsePageMode::Keyset);
    assert!(first.page_info.has_more);
    let cursor = first.page_info.next_cursor.expect("cursor");

    payload.request_id = 2;
    payload.page_request = BrowsePageRequest::Keyset {
        cursor: Some(cursor.clone()),
    };
    let second = manager
        .browse(spec.clone(), payload.clone())
        .await
        .expect("second");
    assert_ne!(first.row_identity, second.row_identity);
    assert_eq!(first.rows.len(), 1);

    payload.request_id = 3;
    payload.sort = vec![BrowseSortKey {
        column: "name".into(),
        direction: BrowseSortDirection::Asc,
        nulls: BrowseNulls::Default,
    }];
    payload.page_request = BrowsePageRequest::Keyset { cursor: None };
    let sorted = manager.browse(spec.clone(), payload).await.expect("sorted");
    assert_eq!(sorted.page_info.mode, BrowsePageMode::Offset);

    let mut keyless = browse_payload("browse-pages", "browse_fixture", "keyless_dupes");
    keyless.tab_id = "keyless".into();
    keyless.page_size = 10;
    keyless.page_request = BrowsePageRequest::Keyset { cursor: None };
    let keyless_page = manager
        .browse(spec.clone(), keyless.clone())
        .await
        .expect("ctid");
    assert_eq!(keyless_page.identity.kind, BrowseIdentityKind::Virtual);
    if keyless_page.identity.columns == ["ctid"] {
        let connection = postgres::connect(&spec).await.expect("connect");
        let descriptor = postgres::load_descriptor(
            connection.inner.client.as_ref(),
            "browse_fixture",
            "keyless_dupes",
        )
        .await
        .expect("descriptor");
        if descriptor.server_version_num >= super::builder::CTID_KEYSET_VERSION {
            assert_eq!(keyless_page.page_info.mode, BrowsePageMode::Keyset);
            keyless.request_id = 2;
            keyless.page_request = BrowsePageRequest::Keyset {
                cursor: keyless_page.page_info.next_cursor.clone(),
            };
            let next = manager
                .browse(spec.clone(), keyless)
                .await
                .expect("ctid next");
            let first_ids = keyless_page.row_identity.clone().unwrap_or_default();
            let next_ids = next.row_identity.clone().unwrap_or_default();
            assert!(first_ids.iter().all(|id| !next_ids.contains(id)));
        }
    }

    let mut expr = browse_payload("browse-pages", "browse_fixture", "expr_unique");
    expr.tab_id = "expr".into();
    let expr = manager.browse(spec.clone(), expr).await.expect("expr");
    assert_eq!(expr.identity.kind, BrowseIdentityKind::Virtual);

    let mut parts = browse_payload("browse-pages", "browse_fixture", "keyless_parts");
    parts.tab_id = "parts".into();
    parts.page_size = 5;
    parts.page_request = BrowsePageRequest::Keyset { cursor: None };
    let parts_page = manager
        .browse(spec.clone(), parts.clone())
        .await
        .expect("parts");
    assert_eq!(parts_page.identity.kind, BrowseIdentityKind::Virtual);
    assert_eq!(parts_page.identity.columns, ["tableoid", "ctid"]);
    let part_ids = parts_page.row_identity.clone().unwrap_or_default();
    let unique_part_ids = part_ids
        .iter()
        .cloned()
        .collect::<std::collections::HashSet<_>>();
    assert_eq!(unique_part_ids.len(), part_ids.len());
    if parts_page.page_info.mode == BrowsePageMode::Keyset {
        parts.request_id = 2;
        parts.page_request = BrowsePageRequest::Keyset {
            cursor: parts_page.page_info.next_cursor.clone(),
        };
        let next = manager
            .browse(spec.clone(), parts)
            .await
            .expect("parts next");
        let next_ids = next.row_identity.clone().unwrap_or_default();
        assert!(part_ids.iter().all(|id| !next_ids.contains(id)));
    }

    let admin = session_postgres::connect(&spec).await.expect("admin");
    let schema = format!("browse_live_{}", uuid::Uuid::new_v4().simple());
    admin
        .client
        .batch_execute(&format!(
            "CREATE SCHEMA {schema}; CREATE TABLE {schema}.fresh (id int PRIMARY KEY, body text);"
        ))
        .await
        .expect("fresh");
    let mut unknown = browse_payload("browse-pages", &schema, "fresh");
    unknown.tab_id = "fresh".into();
    unknown.count_policy = BrowseCountPolicy::Estimated;
    let unknown = manager
        .browse(spec.clone(), unknown)
        .await
        .expect("unanalyzed");
    assert_eq!(unknown.count.kind, BrowseCountKind::Unknown);

    admin
        .client
        .batch_execute("ANALYZE browse_fixture.large_rows")
        .await
        .ok();
    let mut large = browse_payload("browse-pages", "browse_fixture", "large_rows");
    large.tab_id = "large".into();
    large.page_size = 5;
    large.count_policy = BrowseCountPolicy::Estimated;
    let large = manager.browse(spec.clone(), large).await.expect("large");
    assert_eq!(large.count.kind, BrowseCountKind::Estimated);
    assert!(large.count.value.unwrap_or(0) >= 1000);

    let count = manager
        .count(
            spec.clone(),
            CountTableBrowseRowsPayload {
                connection_id: "browse-pages".into(),
                tab_id: "count".into(),
                request_id: 99,
                schema: "crm".into(),
                table: "accounts".into(),
                filters: Vec::new(),
            },
        )
        .await
        .expect("exact");
    assert_eq!(count.kind, BrowseCountKind::Exact);
    assert!(count.value >= 1);

    admin
        .client
        .batch_execute(&format!(
            "CREATE TABLE {schema}.dropcol (id int PRIMARY KEY, gone text); INSERT INTO {schema}.dropcol VALUES (1, 'x');"
        ))
        .await
        .expect("dropcol");
    let mut dropcol = browse_payload("browse-pages", &schema, "dropcol");
    dropcol.tab_id = "dropcol".into();
    manager
        .browse(spec.clone(), dropcol.clone())
        .await
        .expect("before drop");
    admin
        .client
        .batch_execute(&format!("ALTER TABLE {schema}.dropcol DROP COLUMN gone"))
        .await
        .expect("drop");
    dropcol.request_id = 2;
    let after = manager.browse(spec, dropcol).await.expect("after drop");
    assert_eq!(after.columns.len(), 1);
    admin
        .client
        .batch_execute(&format!("DROP SCHEMA {schema} CASCADE"))
        .await
        .ok();
}

#[tokio::test]
#[ignore = "requires pnpm db:postgres"]
async fn table_browse_live_cancel_supersede_truncation_and_teardown() {
    let spec = live_spec(15432, true, "browse-cancel");
    let manager = TableBrowseManager::new();
    let mut sleepy = browse_payload("browse-cancel", "crm", "accounts");
    sleepy.filters = vec![BrowseFilter::RawSql {
        text: "(SELECT pg_sleep(20))::text IS NULL".into(),
    }];
    let first = {
        let manager = manager.clone();
        let spec = spec.clone();
        let payload = sleepy.clone();
        tokio::spawn(async move { manager.browse(spec, payload).await })
    };
    tokio::time::sleep(Duration::from_millis(200)).await;
    sleepy.request_id = 2;
    sleepy.filters.clear();
    let second = manager.browse(spec.clone(), sleepy.clone()).await;
    assert!(second.is_ok());
    let first = first.await.expect("join");
    assert!(matches!(first, Err(TableBrowseError::Superseded)));

    sleepy.request_id = 3;
    sleepy.filters = vec![BrowseFilter::RawSql {
        text: "(SELECT pg_sleep(20))::text IS NULL".into(),
    }];
    let pending = {
        let manager = manager.clone();
        let spec = spec.clone();
        let payload = sleepy.clone();
        tokio::spawn(async move { manager.browse(spec, payload).await })
    };
    tokio::time::sleep(Duration::from_millis(200)).await;
    let cancel = manager.cancel_tab("browse-cancel", "tab").await;
    assert!(cancel.cancel_requested);
    let pending = pending.await.expect("join cancel");
    assert!(matches!(pending, Err(TableBrowseError::Cancelled)));

    let admin = session_postgres::connect(&spec).await.expect("admin");
    let schema = format!("browse_live_{}", uuid::Uuid::new_v4().simple());
    admin
        .client
        .batch_execute(&format!(
            "CREATE SCHEMA {schema}; CREATE TABLE {schema}.wide (id int PRIMARY KEY, body text); INSERT INTO {schema}.wide VALUES (1, repeat('x', 1500000));"
        ))
        .await
        .expect("wide");
    let mut wide = browse_payload("browse-cancel", &schema, "wide");
    wide.tab_id = "wide".into();
    wide.request_id = 10;
    let truncated = manager.browse(spec.clone(), wide).await.expect("trunc");
    assert!(truncated.truncated_cells >= 1);

    admin
        .client
        .batch_execute(&format!(
            "CREATE TABLE {schema}.wide_pk (id text PRIMARY KEY, body text); INSERT INTO {schema}.wide_pk VALUES (repeat('k', 4000), 'row');"
        ))
        .await
        .expect("wide pk");
    let mut wide_pk = browse_payload("browse-cancel", &schema, "wide_pk");
    wide_pk.tab_id = "wide-pk".into();
    wide_pk.request_id = 11;
    let truncated_pk = manager
        .browse(spec.clone(), wide_pk)
        .await
        .expect("wide pk browse");
    let identity = truncated_pk.row_identity.expect("pk identity");
    assert_eq!(identity[0][0].len(), 4000);
    assert_eq!(
        identity[0][0],
        truncated_pk.rows[0][0].as_ref().expect("pk cell").as_str()
    );
    admin
        .client
        .batch_execute(&format!("DROP SCHEMA {schema} CASCADE"))
        .await
        .ok();

    let pid = {
        let executor = manager
            .inner
            .lock()
            .await
            .executors
            .get("browse-cancel")
            .cloned()
            .expect("executor");
        let client = executor
            .inner
            .lock()
            .await
            .connection
            .as_ref()
            .expect("socket")
            .inner
            .client
            .clone();
        client
            .query_one("SELECT pg_backend_pid()", &[])
            .await
            .expect("pid")
            .get::<_, i32>(0)
    };
    let _ = admin
        .client
        .execute("SELECT pg_terminate_backend($1)", &[&pid])
        .await;
    tokio::time::sleep(Duration::from_millis(100)).await;
    sleepy.request_id = 12;
    sleepy.tab_id = "reconnect".into();
    sleepy.filters.clear();
    let recovered = match manager.browse(spec.clone(), sleepy.clone()).await {
        Ok(result) => result,
        Err(TableBrowseError::ConnectionLost) => {
            sleepy.request_id = 13;
            manager
                .browse(spec.clone(), sleepy.clone())
                .await
                .expect("reconnect")
        }
        Err(error) => panic!("unexpected {error:?}"),
    };
    assert!(!recovered.rows.is_empty());

    sleepy.request_id = 14;
    sleepy.tab_id = "teardown".into();
    sleepy.filters = vec![BrowseFilter::RawSql {
        text: "(SELECT pg_sleep(20))::text IS NULL".into(),
    }];
    let inflight = {
        let manager = manager.clone();
        let spec = spec.clone();
        tokio::spawn(async move { manager.browse(spec, sleepy).await })
    };
    tokio::time::sleep(Duration::from_millis(200)).await;
    manager.begin_connection_teardown("browse-cancel").await;
    let inflight = inflight.await.expect("join teardown");
    assert!(matches!(
        inflight,
        Err(TableBrowseError::ConnectionClosing)
            | Err(TableBrowseError::Cancelled)
            | Err(TableBrowseError::Superseded)
            | Err(TableBrowseError::Database { .. })
    ));
}

#[tokio::test]
#[ignore = "requires pnpm db:postgres-tls"]
async fn table_browse_live_tls_connects() {
    let spec = live_spec(15433, true, "browse-tls");
    let manager = TableBrowseManager::new();
    let result = manager
        .browse(spec, browse_payload("browse-tls", "crm", "accounts"))
        .await
        .expect("tls browse");
    assert!(!result.rows.is_empty());
}
