use super::*;
use dbunk_lib::backend::table_ddl::{TableDdlColumn, TableIdentity};

fn target() -> TableDdlDescription {
    TableDdlDescription {
        identity: TableIdentity {
            database_oid: 1,
            relation_oid: 2,
        },
        schema_oid: 3,
        schema: "a.b".into(),
        table: "table\"name".into(),
        namespace_xmin: "7".into(),
        namespace_ctid: "(0,1)".into(),
        column: Some(TableDdlColumn {
            attnum: 4,
            name: "column.dot".into(),
        }),
        comment: None,
    }
}
#[test]
fn selected_column_identity_survives_names_but_refuses_recreated_attnum() {
    let mut observed = target();
    let request = observed.request();
    assert!(selected_matches(&request, Some(4), &observed));
    observed.column.as_mut().unwrap().attnum = 5;
    assert!(!selected_matches(&request, Some(4), &observed));
    assert!(selected_matches(&request, None, &observed));
    observed.identity.relation_oid = 8;
    assert!(!selected_matches(&request, None, &observed));
}
#[test]
fn qualified_target_and_table_versus_column_cannot_change_on_observe() {
    let observed = target();
    let mut request = observed.request();
    request.schema = "a".into();
    request.table = "b.table\"name".into();
    assert!(!selected_matches(&request, Some(4), &observed));
    let mut table = target();
    table.column = None;
    let request = table.request();
    assert!(selected_matches(&request, None, &table));
    assert!(!selected_matches(&request, Some(4), &table));
    assert!(!selected_matches(&request, None, &observed));
}

// Uses the same owned worker as production, without sockets or opaque tokens.
// Budget pressure must refuse even observation before the backend future runs.
#[tokio::test]
async fn ddl_delivery_reservation_precedes_backend_dispatch() {
    use std::sync::atomic::Ordering;
    let fake = Arc::new(Fake(std::sync::atomic::AtomicUsize::new(0)));
    let budget = ByteBudget::new(RESPONSE_BYTES - 1);
    let (wake, _) = async_channel::bounded(1);
    let (controls, receiver, delivery, commands, cancellation, stop) = channels(budget, wake);
    let status = delivery.status.clone();
    let task = tokio::spawn(document_worker(
        fake.clone(),
        "window".into(),
        "tab".into(),
        "connection".into(),
        delivery,
        commands,
        cancellation,
        stop,
        Arc::new(tokio::sync::Mutex::new(())),
    ));
    let worker = Worker::new(task, status);
    controls
        .send(TableCommand::TableDdlObserve(
            77,
            target().request(),
            Some(4),
        ))
        .unwrap();
    // Wait for the correlated refusal, not a wall-clock sleep.
    tokio::time::timeout(std::time::Duration::from_secs(1), async {
        loop {
            if let Some(message) = receiver.try_recv()
                && matches!(
                    message.into_message(),
                    TableMessage::TableDdlObserved(77, DdlObserved::Table(Err(_)))
                )
            {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert_eq!(fake.0.load(Ordering::Relaxed), 0);
    controls.stop();
    worker.join.await.unwrap();
}

struct Fake(std::sync::atomic::AtomicUsize);
impl TableBackend for Arc<Fake> {
    type Document = ();
    fn open<'a>(&'a self, _: &'a str, _: &'a str, _: &'a str) -> BoxFuture<'a, DataResult<()>> {
        Box::pin(async { Ok(()) })
    }
    fn request<'a>(&'a self, _: &'a (), command: TableCommand) -> BoxFuture<'a, TableMessage> {
        Box::pin(async move {
            self.0.fetch_add(1, Ordering::Relaxed);
            cancelled(command)
        })
    }
    fn cancel<'a>(&'a self, _: &'a ()) -> BoxFuture<'a, DataResult<()>> {
        Box::pin(async { Ok(()) })
    }
    fn close<'a>(&'a self, _: &'a ()) -> BoxFuture<'a, TableCloseResult> {
        Box::pin(async { Ok(DataCloseOutcome::Closed) })
    }
}

#[tokio::test]
async fn cancellation_before_first_poll_reuses_reserved_delivery_and_never_dispatches() {
    let fake = Arc::new(Fake(std::sync::atomic::AtomicUsize::new(0)));
    let budget = ByteBudget::new(RESPONSE_BYTES);
    let (wake, _) = async_channel::bounded(1);
    let (controls, receiver, delivery, mut commands, mut cancellation, mut stop) =
        channels(budget.clone(), wake);
    controls.cancel();
    let result = run_request(
        &fake,
        &(),
        TableCommand::TableDdlObserve(88, target().request(), Some(4)),
        &delivery,
        &mut commands,
        &mut cancellation,
        &mut stop,
        &tokio::sync::Mutex::new(()),
    )
    .await;
    assert!(result);
    assert_eq!(fake.0.load(Ordering::Relaxed), 0);
    assert_eq!(budget.used(), RESPONSE_BYTES);
    assert!(matches!(receiver.try_recv().unwrap().into_message(),
        TableMessage::TableDdlObserved(88, DdlObserved::Table(Err(error))) if *error == TableDdlError::Unavailable));
    assert_eq!(budget.used(), 0);
}

#[test]
fn schema_observation_keeps_exact_name_and_refuses_recreated_oid_once_pinned() {
    use dbunk_lib::backend::schema_alter::SchemaIdentity;
    let observed = SchemaAlterDescription {
        identity: SchemaIdentity {
            database_oid: 1,
            schema_oid: 2,
        },
        schema: " a.b\"".into(),
        namespace_xmin: "7".into(),
        namespace_ctid: "(0,1)".into(),
        comment: None,
    };
    let mut request = SchemaAlterRequest {
        schema: observed.schema.clone(),
        expected: None,
    };
    assert!(schema_matches(&request, &observed));
    request.expected = Some(observed.identity);
    assert!(schema_matches(&request, &observed));
    let mut recreated = observed.clone();
    recreated.identity.schema_oid = 3;
    assert!(!schema_matches(&request, &recreated));
    request.schema = "a.b\"".into();
    assert!(!schema_matches(&request, &observed));
}

// A schema command consumed by the queue but never polled settles as a typed
// pre-dispatch refusal on the schema lane, never as a table reply or unknown.
#[tokio::test]
async fn schema_cancellation_before_first_poll_never_dispatches_and_keeps_family() {
    let fake = Arc::new(Fake(std::sync::atomic::AtomicUsize::new(0)));
    let budget = ByteBudget::new(RESPONSE_BYTES);
    let (wake, _) = async_channel::bounded(1);
    let (controls, receiver, delivery, mut commands, mut cancellation, mut stop) =
        channels(budget.clone(), wake);
    controls.cancel();
    let result = run_request(
        &fake,
        &(),
        TableCommand::SchemaAlterObserve(
            89,
            SchemaAlterRequest {
                schema: "s".into(),
                expected: None,
            },
        ),
        &delivery,
        &mut commands,
        &mut cancellation,
        &mut stop,
        &tokio::sync::Mutex::new(()),
    )
    .await;
    assert!(result);
    assert_eq!(fake.0.load(Ordering::Relaxed), 0);
    assert!(matches!(receiver.try_recv().unwrap().into_message(),
        TableMessage::TableDdlObserved(89, DdlObserved::Schema(Err(error)))
            if *error == SchemaAlterError::Unavailable));
    assert_eq!(budget.used(), 0);
}
