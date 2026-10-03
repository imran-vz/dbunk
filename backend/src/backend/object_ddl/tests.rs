use super::*;
use crate::backend::profile;
use std::sync::atomic::{AtomicUsize, Ordering};

pub(crate) fn address(class_oid: u32, object_oid: u32, version: &str) -> ObjectAddress {
    ObjectAddress {
        class_oid,
        object_oid,
        row_version: version.into(),
    }
}
pub(crate) fn view_ref(schema: &str, name: &str) -> PgObjectRef {
    PgObjectRef {
        kind: PgObjectKind::View,
        schema: Some(schema.into()),
        name: name.into(),
        identity_args: None,
    }
}
fn schema_claim(name: &str) -> ObjectDdlClaim {
    ObjectDdlClaim::Schema {
        name: name.into(),
        address: address(2615, 2200, "5"),
    }
}
/// Drop of an observed view plus creation of a new view: one atomic group.
pub(crate) fn atomic_case() -> (ObjectDdlDescription, Vec<ObjectDdlOperation>) {
    let operations = vec![
        ObjectDdlOperation::DropObject {
            reference: view_ref("app\"s", "old view"),
            cascade: false,
        },
        ObjectDdlOperation::CreateView {
            schema: "app\"s".into(),
            name: "fresh".into(),
            or_replace: false,
            sql_body: "SELECT 1 AS one;".into(),
        },
    ];
    let description = ObjectDdlDescription {
        database_oid: 16384,
        claims: vec![
            vec![ObjectDdlClaim::Existing {
                reference: view_ref("app\"s", "old view"),
                address: address(1259, 500, "77"),
            }],
            vec![
                schema_claim("app\"s"),
                ObjectDdlClaim::Absent {
                    schema: "app\"s".into(),
                    name: "fresh".into(),
                },
            ],
        ],
    };
    (description, operations)
}
/// Atomic view, standalone concurrent index, standalone enum label, atomic drop.
pub(crate) fn mixed_case() -> (ObjectDdlDescription, Vec<ObjectDdlOperation>) {
    let table = PgObjectRef {
        kind: PgObjectKind::Table,
        schema: Some("app".into()),
        name: "orders".into(),
        identity_args: None,
    };
    let kind = PgObjectRef {
        kind: PgObjectKind::Type,
        schema: Some("app".into()),
        name: "state".into(),
        identity_args: None,
    };
    let operations = vec![
        ObjectDdlOperation::CreateView {
            schema: "app".into(),
            name: "summary".into(),
            or_replace: false,
            sql_body: "SELECT 1".into(),
        },
        ObjectDdlOperation::CreateIndex {
            schema: "app".into(),
            table: "orders".into(),
            name: "orders_state_idx".into(),
            unique: false,
            method: "btree".into(),
            columns: vec![ObjectDdlIndexColumn {
                expression: "state".into(),
                descending: false,
            }],
            concurrently: true,
        },
        ObjectDdlOperation::AddEnumValue {
            schema: "app".into(),
            name: "state".into(),
            value: "archived".into(),
            position: Some(ObjectDdlEnumPosition::After {
                neighbor: "done".into(),
            }),
        },
        ObjectDdlOperation::DropObject {
            reference: view_ref("app", "legacy"),
            cascade: true,
        },
    ];
    let description = ObjectDdlDescription {
        database_oid: 16384,
        claims: vec![
            vec![
                schema_claim("app"),
                ObjectDdlClaim::Absent {
                    schema: "app".into(),
                    name: "summary".into(),
                },
            ],
            vec![
                schema_claim("app"),
                ObjectDdlClaim::Existing {
                    reference: table,
                    address: address(1259, 600, "80"),
                },
                ObjectDdlClaim::Absent {
                    schema: "app".into(),
                    name: "orders_state_idx".into(),
                },
            ],
            vec![ObjectDdlClaim::Existing {
                reference: kind,
                address: address(1247, 700, "81"),
            }],
            vec![ObjectDdlClaim::Existing {
                reference: view_ref("app", "legacy"),
                address: address(1259, 800, "82"),
            }],
        ],
    };
    (description, operations)
}

#[test]
fn atomic_and_standalone_groups_follow_shared_classification() {
    let (description, operations) = atomic_case();
    let preview = preview::render(&description, &operations, Some(0), false).unwrap();
    assert_eq!(
        preview.groups,
        vec![ObjectDdlGroup::Atomic {
            statements: vec![0, 1]
        }]
    );
    assert_eq!(
        preview.statements[0].sql,
        "DROP VIEW \"app\"\"s\".\"old view\" RESTRICT;"
    );
    assert!(preview.statements[0].destructive);
    assert!(preview.destructive());
    assert_eq!(preview.effect_scope(), OBJECT_DDL_ATOMIC_SCOPE);
    let (description, operations) = mixed_case();
    let preview = preview::render(&description, &operations, None, true).unwrap();
    assert_eq!(
        preview.groups,
        vec![
            ObjectDdlGroup::Atomic {
                statements: vec![0]
            },
            ObjectDdlGroup::Standalone { statement: 1 },
            ObjectDdlGroup::Standalone { statement: 2 },
            ObjectDdlGroup::Atomic {
                statements: vec![3]
            },
        ]
    );
    assert!(preview.statements[1]
        .sql
        .starts_with("CREATE INDEX CONCURRENTLY"));
    assert!(!preview.statements[1].transactional && !preview.statements[2].transactional);
    assert!(preview.statements[3].sql.ends_with("CASCADE;"));
    assert_eq!(preview.effect_scope(), OBJECT_DDL_STANDALONE_SCOPE);
    assert!(preview.confirmation_required);
    // Each statement equals the shared renderer's output for the typed op.
    let typed = operations
        .iter()
        .map(ObjectDdlOperation::to_pg)
        .collect::<Vec<_>>();
    let shared = crate::postgres::object_ddl::generate_object_ddl(&typed).unwrap();
    for (ours, theirs) in preview.statements.iter().zip(&shared.statements) {
        assert_eq!(ours.sql, theirs.sql);
    }
}

#[test]
fn claims_must_answer_each_operation_exactly() {
    let (mut description, operations) = atomic_case();
    let original = description.clone();
    // A recreated view under the same name is a different reference only if
    // its identity differs; a claim for another object never satisfies.
    description.claims[0][0] = ObjectDdlClaim::Existing {
        reference: view_ref("app\"s", "other"),
        address: address(1259, 500, "77"),
    };
    assert_eq!(
        preview::render(&description, &operations, None, false),
        Err(ObjectDdlError::TargetMismatch)
    );
    let mut description = original.clone();
    description.claims[1][1] = ObjectDdlClaim::Existing {
        reference: view_ref("app\"s", "fresh"),
        address: address(1259, 900, "1"),
    };
    // Without OR REPLACE a create requires absence.
    assert_eq!(
        preview::render(&description, &operations, None, false),
        Err(ObjectDdlError::TargetMismatch)
    );
    let mut replace = operations.clone();
    if let ObjectDdlOperation::CreateView { or_replace, .. } = &mut replace[1] {
        *or_replace = true;
    }
    assert!(preview::render(&description, &replace, None, false).is_ok());
    let mut description = original;
    description.claims.pop();
    assert_eq!(
        preview::render(&description, &operations, None, false),
        Err(ObjectDdlError::TargetMismatch)
    );
}

#[test]
fn requests_are_bounded_and_refuse_unsupported_or_repeated_targets() {
    let (_, operations) = atomic_case();
    assert!(ObjectDdlRequest {
        operations: operations.clone()
    }
    .validate()
    .is_ok());
    let mut refused = vec![
        Vec::new(),
        vec![operations[0].clone(); MAX_OBJECT_DDL_OPERATIONS + 1],
        vec![operations[0].clone(), operations[0].clone()],
    ];
    let mut extension = operations[0].clone();
    if let ObjectDdlOperation::DropObject { reference, .. } = &mut extension {
        reference.kind = PgObjectKind::Extension;
    }
    refused.push(vec![extension]);
    let mut routine = operations[0].clone();
    if let ObjectDdlOperation::DropObject { reference, .. } = &mut routine {
        reference.kind = PgObjectKind::Function;
    }
    refused.push(vec![routine]);
    for body in ["", "SELECT '\0'"] {
        let mut view = operations[1].clone();
        if let ObjectDdlOperation::CreateView { sql_body, .. } = &mut view {
            *sql_body = body.into();
        }
        refused.push(vec![view]);
    }
    let mut huge = operations[1].clone();
    if let ObjectDdlOperation::CreateView { sql_body, .. } = &mut huge {
        *sql_body = "x".repeat(MAX_OBJECT_DDL_SQL_BODY_BYTES + 1);
    }
    refused.push(vec![huge]);
    let mut long = operations[1].clone();
    if let ObjectDdlOperation::CreateView { name, .. } = &mut long {
        *name = "n".repeat(64);
    }
    refused.push(vec![long]);
    for operations in refused {
        assert_eq!(
            ObjectDdlRequest { operations }.validate(),
            Err(ObjectDdlError::InvalidRequest)
        );
    }
    // A routine drop carries its exact overload identity.
    let overload = ObjectDdlOperation::DropObject {
        reference: PgObjectRef {
            kind: PgObjectKind::Function,
            schema: Some("app".into()),
            name: "f".into(),
            identity_args: Some("integer, text".into()),
        },
        cascade: false,
    };
    assert!(ObjectDdlRequest {
        operations: vec![overload]
    }
    .validate()
    .is_ok());
    // The shared renderer still refuses statement boundaries in bodies.
    let (description, mut operations) = atomic_case();
    if let ObjectDdlOperation::CreateView { sql_body, .. } = &mut operations[1] {
        *sql_body = "SELECT 1; DROP TABLE x".into();
    }
    assert_eq!(
        preview::render(&description, &operations, None, false),
        Err(ObjectDdlError::InvalidRequest)
    );
}

#[test]
fn restored_preview_must_equal_regenerated_review() {
    let (description, operations) = mixed_case();
    let preview = preview::render(&description, &operations, Some(250), true).unwrap();
    assert!(preview.matches_typed_description(&description, &operations));
    // The confirmation flag and timeout are self-describing recovery facts;
    // apply re-derives both from stored state and refuses any difference.
    for mutation in 0..5 {
        let mut changed = preview.clone();
        match mutation {
            0 => changed.statements[0].sql.push(' '),
            1 => changed.operation_digest = "fnv64:0".into(),
            2 => changed.statements[2].summary.push('!'),
            3 => changed.groups.swap(1, 2),
            _ => changed.statements[1].transactional = true,
        }
        assert!(
            !changed.matches_typed_description(&description, &operations),
            "mutation {mutation}"
        );
    }
    let mut other = operations.clone();
    other.swap(1, 2);
    assert!(!preview.matches_typed_description(&description, &other));
    assert!(!format!("{:?}", operations[0]).contains("SELECT"));
}

#[test]
fn readmission_refuses_after_cancellation_during_an_admitted_group() {
    let permit = WritePermit::test_permit();
    assert!(permit.admit_commit());
    assert!(permit.readmit());
    assert!(permit.check_preparing());
    assert!(permit.admit_dispatch());
    permit.test_cancel();
    assert!(!permit.readmit());
    assert!(!permit.check_preparing());
    let permit = WritePermit::test_permit();
    assert!(
        !permit.readmit(),
        "never readmit a group that was not admitted"
    );
}

async fn fixture() -> (tempfile::TempDir, Backend, DataDocument) {
    let directory = profile::directory();
    let backend = Backend::open_fixture(&directory.path().canonicalize().unwrap())
        .await
        .unwrap();
    let document = backend
        .open_data_document("objects", "object-ddl", &backend.fixture().id)
        .await
        .unwrap();
    (directory, backend, document)
}
fn target(document: &DataDocument) -> ObjectDdlTarget {
    let (description, operations) = atomic_case();
    ObjectDdlTarget {
        document: document.clone(),
        generation: *document.0.read_cancellation().borrow(),
        operations,
        description,
        impacts: vec![
            Some(PgDropImpact {
                dependents: Vec::new(),
                truncated: false,
            }),
            None,
        ],
        statement_timeout_ms: None,
    }
}
async fn policy(backend: &Backend, readonly: bool, mode: crate::SafeMode, timeout: Option<u32>) {
    let mut stored =
        crate::storage::read_connection_by_id(&backend.0.state.pool, &backend.fixture().id)
            .await
            .unwrap()
            .unwrap();
    let crate::StoredConnection::PostgreSQL(pg) = &mut stored else {
        panic!()
    };
    pg.read_only = readonly;
    pg.safe_mode = mode;
    pg.driver_options
        .get_or_insert_with(Default::default)
        .statement_timeout_ms = timeout;
    crate::storage::upsert_connection(&backend.0.state.pool, &stored)
        .await
        .unwrap();
}
fn never_dispatch(
    calls: &Arc<AtomicUsize>,
) -> impl FnOnce(
    Arc<crate::app::AppState>,
    String,
) -> BoxFuture<'static, Option<ResolvedPostgresConnectSpec>>
       + Send
       + 'static {
    let calls = calls.clone();
    move |_, _| {
        calls.fetch_add(1, Ordering::SeqCst);
        Box::pin(async { None })
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn stored_policy_is_bound_at_review_and_rechecked_before_credentials() {
    let (_directory, backend, document) = fixture().await;
    policy(&backend, true, crate::SafeMode::Strict, None).await;
    assert!(matches!(
        backend.review_object_ddl(target(&document)).await,
        Err(ObjectDdlError::PolicyBlocked)
    ));
    policy(&backend, false, crate::SafeMode::Strict, None).await;
    let review = backend.review_object_ddl(target(&document)).await.unwrap();
    assert!(review.preview().confirmation_required);
    assert_eq!(review.impacts().len(), 2);
    let attempt = review.attempt_id().clone();
    let calls = Arc::new(AtomicUsize::new(0));
    let ObjectDdlSubmission::NeedsConfirmation(confirmation) = backend
        .submit_object_ddl(
            review,
            false,
            native_object_ddl::execute,
            never_dispatch(&calls),
        )
        .await
        .unwrap()
    else {
        panic!("strict policy requires confirmation")
    };
    assert_eq!(confirmation.review().attempt_id(), &attempt);
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    // Read-only after review: the confirmation token is refused.
    policy(&backend, true, crate::SafeMode::Strict, None).await;
    assert!(matches!(
        backend.confirm_object_ddl(*confirmation).await,
        Err(ObjectDdlError::PolicyBlocked)
    ));
    // A loosened policy is a changed policy: review again.
    policy(&backend, false, crate::SafeMode::Strict, None).await;
    let review = backend.review_object_ddl(target(&document)).await.unwrap();
    policy(&backend, false, crate::SafeMode::Disabled, None).await;
    assert!(matches!(
        backend
            .submit_object_ddl(
                review,
                true,
                native_object_ddl::execute,
                never_dispatch(&calls)
            )
            .await,
        Err(ObjectDdlError::PolicyChanged)
    ));
    // A changed statement timeout no longer matches the reviewed preview.
    let review = backend.review_object_ddl(target(&document)).await.unwrap();
    assert!(!review.preview().confirmation_required);
    policy(&backend, false, crate::SafeMode::Disabled, Some(20)).await;
    assert!(matches!(
        backend
            .submit_object_ddl(
                review,
                false,
                native_object_ddl::execute,
                never_dispatch(&calls)
            )
            .await,
        Err(ObjectDdlError::InvalidRequest)
    ));
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    backend.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cancellation_before_dispatch_never_loads_credentials_or_sends() {
    let (_directory, backend, document) = fixture().await;
    policy(&backend, false, crate::SafeMode::Disabled, None).await;
    let review = backend.review_object_ddl(target(&document)).await.unwrap();
    backend.cancel_data(&document).await.unwrap();
    let calls = Arc::new(AtomicUsize::new(0));
    assert!(matches!(
        backend
            .submit_object_ddl(
                review,
                false,
                native_object_ddl::execute,
                never_dispatch(&calls)
            )
            .await,
        Err(ObjectDdlError::Unavailable)
    ));
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    // A stale observation cannot even be reviewed.
    let stale = target(&document);
    backend.cancel_data(&document).await.unwrap();
    assert!(matches!(
        backend.review_object_ddl(stale).await,
        Err(ObjectDdlError::Unavailable)
    ));
    backend.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn finished_receipt_is_exact_and_audits_only_possible_changes() {
    let (_directory, backend, document) = fixture().await;
    policy(&backend, false, crate::SafeMode::Disabled, None).await;
    let review = backend.review_object_ddl(target(&document)).await.unwrap();
    let attempt = review.attempt_id().clone();
    let expected_operations = review.operations().to_vec();
    let result = backend
        .submit_object_ddl(
            review,
            false,
            |_spec, drivers, permit, _cancel, _target, _operations, preview| async move {
                let _permit = permit;
                drivers.drain().await;
                assert_eq!(preview.groups.len(), 1);
                ObjectDdlOutcome::Stopped {
                    committed: 0,
                    stopped_at: 0,
                    stop: ObjectDdlStop::RolledBack,
                    reason: ObjectDdlFailure::TargetChanged,
                    residue: None,
                }
            },
            load,
        )
        .await
        .unwrap();
    let ObjectDdlSubmission::Finished(receipt) = result else {
        panic!("disabled policy dispatches")
    };
    assert_eq!(receipt.attempt_id, attempt);
    assert_eq!(receipt.operations, expected_operations);
    assert!(!receipt.outcome.may_have_changed());
    assert!(receipt.encoded_bytes() <= MAX_OBJECT_DDL_RECEIPT_BYTES);
    backend.shutdown().await.unwrap();
}
