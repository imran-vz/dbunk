use super::*;

#[test]
fn catalog_preserves_overloads_types_and_database_scoped_groups() {
    let mut builder = Builder::new();
    builder.schema("Mixed \" schema").unwrap();
    for identity in ["", "\"Arg\" numeric(32, 12)"] {
        builder
            .entry(
                Some("Mixed \" schema"),
                "function",
                "overload",
                Some(identity),
                Some("日本語\ncomment"),
                None,
            )
            .unwrap();
    }
    builder
        .entry(
            Some("Mixed \" schema"),
            "type",
            "range",
            None,
            None,
            Some(PgTypeClass::Range),
        )
        .unwrap();
    builder
        .entry(None, "role", "Owner", None, Some(""), None)
        .unwrap();
    let catalog = builder.finish().unwrap();
    assert_eq!(
        catalog.schemas[0].functions[1].identity_args.as_deref(),
        Some("\"Arg\" numeric(32, 12)")
    );
    assert_eq!(
        catalog.schemas[0].functions[0].identity_args.as_deref(),
        Some("")
    );
    assert_eq!(
        catalog.schemas[0].types[0].type_class,
        Some(PgTypeClass::Range)
    );
    assert_eq!(catalog.roles[0].comment.as_deref(), Some(""));
    assert!(catalog.truncated.is_empty());
}

#[test]
fn baseline_per_kind_cap_is_disclosed_without_merging_names_or_overloads() {
    let mut builder = Builder::new();
    builder.schema("public").unwrap();
    for i in 0..=CATALOG_KIND_CAP {
        builder
            .entry(
                Some("public"),
                "function",
                "same_name",
                Some(&format!("arg{i} integer")),
                None,
                None,
            )
            .unwrap();
    }
    let catalog = builder.finish().unwrap();
    assert_eq!(catalog.schemas[0].functions.len(), CATALOG_KIND_CAP);
    assert_eq!(
        catalog.truncated,
        [PgCatalogTruncation {
            schema: Some("public".into()),
            kind: "function".into()
        }]
    );
}

#[test]
fn native_total_node_limit_refuses_across_individually_small_groups() {
    let mut builder = Builder::new();
    for i in 0..10 {
        builder.schema(&format!("schema{i}")).unwrap();
    }
    for i in 10..MAX_CATALOG_NODES {
        builder
            .entry(
                Some(&format!("schema{}", i % 10)),
                "table",
                &format!("t{i}"),
                None,
                None,
                None,
            )
            .unwrap();
    }
    assert_eq!(
        builder.entry(None, "role", "one_too_many", None, None, None),
        Err(CatalogError::NodeLimit)
    );
}

#[test]
fn escaping_is_charged_before_retention_and_never_returns_partial_success() {
    let mut builder = Builder::new();
    builder.schema("public").unwrap();
    // Six JSON bytes per input NUL makes a raw-byte-only admission wrong.
    let comment = "\0".repeat(MAX_TEXT_BYTES);
    let mut accepted = 0;
    loop {
        let result = builder.entry(
            Some("public"),
            "table",
            "bounded",
            None,
            Some(&comment),
            None,
        );
        if result == Err(CatalogError::ByteLimit) {
            break;
        }
        result.unwrap();
        accepted += 1;
    }
    assert!(accepted < 200);
    assert_eq!(builder.schemas["public"].tables.len(), accepted);
    let bytes = serde_json::to_vec(&builder.finish().unwrap())
        .unwrap()
        .len();
    assert!(bytes <= MAX_CATALOG_BYTES);
    assert!(bytes > MAX_CATALOG_BYTES - 60_000);
}

#[test]
fn oversized_identity_and_comment_are_refused_not_truncated() {
    for (identity, comment) in [
        (Some("x".repeat(MAX_TEXT_BYTES + 1)), None),
        (None, Some("é".repeat(MAX_TEXT_BYTES / 2 + 1))),
    ] {
        let mut builder = Builder::new();
        builder.schema("public").unwrap();
        assert_eq!(
            builder.entry(
                Some("public"),
                "function",
                "f",
                identity.as_deref(),
                comment.as_deref(),
                None
            ),
            Err(CatalogError::TextLimit)
        );
        assert!(builder.schemas["public"].functions.is_empty());
    }
}

#[test]
fn unknown_group_or_schema_is_an_error_not_a_silently_missing_object() {
    let mut builder = Builder::new();
    builder.schema("public").unwrap();
    assert_eq!(
        builder.entry(Some("missing"), "table", "t", None, None, None),
        Err(CatalogError::InvalidResponse)
    );
    assert_eq!(
        builder.entry(None, "table", "t", None, None, None),
        Err(CatalogError::InvalidResponse)
    );
}

#[tokio::test]
async fn cleanup_aborts_and_joins_hung_driver_without_restarting_grace() {
    use std::sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    };
    struct Dropped(Arc<AtomicBool>);
    impl Drop for Dropped {
        fn drop(&mut self) {
            self.0.store(true, Ordering::SeqCst);
        }
    }
    let parent = DriverJoins::default();
    let drivers = parent.child();
    let dropped = Arc::new(AtomicBool::new(false));
    let flag = dropped.clone();
    let (ready, started) = tokio::sync::oneshot::channel();
    drivers.track_task(tokio::spawn(async move {
        let _guard = Dropped(flag);
        ready.send(()).unwrap();
        std::future::pending::<()>().await;
    }));
    started.await.unwrap();
    let deadline = tokio::time::Instant::now() + Duration::from_millis(20);
    // Simulate cancel/close consuming the shared grace before the driver join.
    tokio::time::sleep_until(deadline).await;
    tokio::time::timeout(Duration::from_millis(100), join_drivers(&drivers, deadline))
        .await
        .unwrap();
    assert!(
        dropped.load(Ordering::SeqCst),
        "cleanup must actually join, not detach or merely signal abort"
    );
    tokio::time::timeout(Duration::from_millis(100), parent.drain())
        .await
        .unwrap();
    assert!(
        cleanup_deadline(deadline) <= deadline,
        "expired operation deadlines never get fresh cleanup time"
    );
    assert!(
        cleanup_deadline(tokio::time::Instant::now() + Duration::from_secs(30))
            <= tokio::time::Instant::now() + Duration::from_secs(1)
    );
}
