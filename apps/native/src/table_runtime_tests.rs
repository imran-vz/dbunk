use super::*;
use dbunk_lib::backend::data::*;
use std::sync::atomic::AtomicUsize;
use std::time::Duration;
use tokio::sync::Notify;

fn count_command() -> TableCommand {
    let mut model = crate::data_model::TableDocument::new(
        "connection".into(),
        "tab".into(),
        MutationTable {
            schema: "public".into(),
            table: "rows".into(),
        },
    )
    .unwrap();
    let (ticket, payload) = model.count().unwrap();
    TableCommand::Count(ticket, payload)
}

struct Fake {
    block_open: bool,
    open_started: Notify,
    release_open: Notify,
    request_started: Notify,
    release_request: Notify,
    cancelled: Notify,
    cancellations: AtomicUsize,
    closes: AtomicUsize,
    outcome: DataCloseOutcome,
}
impl Fake {
    fn new(block_open: bool, outcome: DataCloseOutcome) -> Arc<Self> {
        Arc::new(Self {
            block_open,
            open_started: Notify::new(),
            release_open: Notify::new(),
            request_started: Notify::new(),
            release_request: Notify::new(),
            cancelled: Notify::new(),
            cancellations: AtomicUsize::new(0),
            closes: AtomicUsize::new(0),
            outcome,
        })
    }
}
impl TableBackend for Arc<Fake> {
    type Document = ();
    fn open<'a>(&'a self, _: &'a str, _: &'a str, _: &'a str) -> BoxFuture<'a, DataResult<()>> {
        Box::pin(async move {
            self.open_started.notify_one();
            if self.block_open {
                self.release_open.notified().await;
            }
            Ok(())
        })
    }
    fn request<'a>(&'a self, _: &'a (), command: TableCommand) -> BoxFuture<'a, TableMessage> {
        Box::pin(async move {
            self.request_started.notify_one();
            self.release_request.notified().await;
            cancelled(command)
        })
    }
    fn cancel<'a>(&'a self, _: &'a ()) -> BoxFuture<'a, DataResult<()>> {
        Box::pin(async move {
            self.cancellations.fetch_add(1, Ordering::Relaxed);
            self.cancelled.notify_one();
            self.release_request.notify_one();
            Ok(())
        })
    }
    fn close<'a>(&'a self, _: &'a ()) -> BoxFuture<'a, TableCloseResult> {
        Box::pin(async move {
            self.closes.fetch_add(1, Ordering::Relaxed);
            Ok(self.outcome)
        })
    }
}
fn worker(fake: Arc<Fake>) -> (Worker, TableControls, TableReceiver) {
    let (wake, _) = async_channel::bounded(1);
    let (controls, receiver, delivery, commands, cancellation, stop) =
        channels(ByteBudget::new(WORKSPACE_QUEUE_BYTES), wake);
    let status = delivery.status.clone();
    let task = tokio::spawn(document_worker(
        fake,
        "window".into(),
        "tab".into(),
        "connection".into(),
        delivery,
        commands,
        cancellation,
        stop,
        Arc::new(tokio::sync::Mutex::new(())),
    ));
    (Worker::new(task, status), controls, receiver)
}

#[tokio::test]
async fn dropped_ui_and_close_waiter_cannot_lose_an_opened_document() {
    let fake = Fake::new(true, DataCloseOutcome::ConnectionDataClosed);
    let (worker, controls, receiver) = worker(fake.clone());
    fake.open_started.notified().await;
    drop(receiver);
    drop(controls);
    assert!(worker.join.clone().now_or_never().is_none());
    assert_eq!(fake.closes.load(Ordering::Relaxed), 0);
    fake.release_open.notify_one();
    let result = tokio::time::timeout(Duration::from_secs(1), worker.join)
        .await
        .unwrap();
    assert!(matches!(result, Ok(DataCloseOutcome::ConnectionDataClosed)));
    assert_eq!(fake.closes.load(Ordering::Relaxed), 1);
}

#[tokio::test]
async fn cancel_and_stop_bypass_a_full_command_queue() {
    let fake = Fake::new(false, DataCloseOutcome::Closed);
    let (worker, controls, receiver) = worker(fake.clone());
    controls.send(count_command()).unwrap();
    fake.request_started.notified().await;
    controls.send(count_command()).unwrap();
    assert!(controls.send(count_command()).is_err());
    controls.cancel();
    tokio::time::timeout(Duration::from_secs(1), fake.cancelled.notified())
        .await
        .unwrap();
    assert_eq!(fake.cancellations.load(Ordering::Relaxed), 1);
    controls.stop();
    assert!(matches!(
        tokio::time::timeout(Duration::from_secs(1), worker.join)
            .await
            .unwrap(),
        Ok(DataCloseOutcome::Closed)
    ));
    assert_eq!(fake.closes.load(Ordering::Relaxed), 1);
    assert!(matches!(
        receiver.close_result(),
        Some(Ok(DataCloseOutcome::Closed))
    ));
}

#[test]
fn shared_budget_is_held_by_envelope_and_released_by_consume_or_drop() {
    let bytes = message_bytes(&TableMessage::Opened);
    let budget = ByteBudget::new(2 * bytes);
    let (wake, _) = async_channel::bounded(1);
    let (_a, first, a, ..) = channels(budget.clone(), wake.clone());
    let (_b, second, b, ..) = channels(budget.clone(), wake);
    assert!(a.send(TableMessage::Opened));
    assert!(b.send(TableMessage::Opened));
    let envelope = first.try_recv().unwrap();
    assert_eq!(budget.used(), 2 * bytes);
    assert!(matches!(envelope.into_message(), TableMessage::Opened));
    assert_eq!(budget.used(), bytes);
    drop(second);
    assert_eq!(budget.used(), 0);
}

#[test]
fn oversized_delivery_and_terminal_outcome_remain_visible_without_queue_space() {
    let (wake, _) = async_channel::bounded(1);
    let budget = ByteBudget::new(WORKSPACE_QUEUE_BYTES);
    let (_controls, receiver, delivery, ..) = channels(budget.clone(), wake);
    let TableCommand::Count(ticket, _) = count_command() else {
        unreachable!()
    };
    let page = BrowseTableResult {
        request_id: 1,
        columns: vec![],
        rows: vec![vec![Some("x".repeat(WORKSPACE_QUEUE_BYTES))]],
        identity: BrowseIdentity {
            kind: BrowseIdentityKind::None,
            columns: vec![],
        },
        row_identity: None,
        page_info: BrowsePageInfo {
            mode: BrowsePageMode::Offset,
            page: Some(1),
            has_more: false,
            next_cursor: None,
        },
        count: BrowseCount {
            kind: BrowseCountKind::Unknown,
            value: None,
        },
        inspection: BrowseInspection {
            sql: "SELECT".into(),
            params: vec![],
        },
        omitted_rows: 0,
        truncated_cells: 0,
        runtime_ms: 0,
    };
    assert!(!delivery.send(TableMessage::Page(ticket, Ok(page))));
    assert_eq!(budget.used(), 0);
    delivery
        .status
        .finish(Ok(DataCloseOutcome::ConnectionDataClosed));
    assert!(matches!(
        receiver.try_recv().unwrap().into_message(),
        TableMessage::Error(TableError::Delivery(DeliveryFailure::Oversize))
    ));
    assert!(matches!(
        receiver.try_recv().unwrap().into_message(),
        TableMessage::Closed(Ok(DataCloseOutcome::ConnectionDataClosed))
    ));
    assert!(!receiver.has_pending());
}

#[tokio::test]
async fn local_close_timeout_preserves_owned_open_for_retry() {
    let fake = Fake::new(true, DataCloseOutcome::Closed);
    let (worker, controls, receiver) = worker(fake.clone());
    fake.open_started.notified().await;
    controls.stop();
    assert!(matches!(
        wait_close(&worker, Instant::now() + Duration::from_millis(10)).await,
        Err(TableError::Worker(message)) if message.contains("still pending")
    ));
    assert!(receiver.close_result().is_none());
    assert!(!worker.abort.is_finished());
    fake.release_open.notify_one();
    assert!(matches!(
        wait_close(&worker, Instant::now() + Duration::from_secs(1)).await,
        Ok(DataCloseOutcome::Closed)
    ));
    assert_eq!(fake.closes.load(Ordering::Relaxed), 1);
}

#[test]
fn only_explicit_global_stop_accepts_backend_closing_handoff() {
    let closing = || {
        Err(TableError::Backend(Arc::new(DataError::Unavailable(
            QuerySessionError::ConnectionClosing,
        ))))
    };
    assert!(global_close_result(closing(), false).is_err());
    assert!(global_close_result(closing(), true).is_ok());
    assert!(
        global_close_result(
            Err(TableError::Backend(Arc::new(DataError::Unavailable(
                QuerySessionError::Timeout {
                    operation: "closeDataDocument".into(),
                }
            )))),
            true,
        )
        .is_err()
    );
    assert!(global_close_result(Err(TableError::Worker("forced shutdown".into())), true).is_err());
}

#[tokio::test]
async fn queued_mutation_cancellation_preserves_family_and_request_identity() {
    let commands = [
        TableCommand::Analyze(
            71,
            AnalyzeResultSetPayload {
                connection_id: "connection".into(),
                tab_id: "tab".into(),
                request_id: 71,
                source: AnalyzeSource::Relation {
                    schema: "public".into(),
                    table: "rows".into(),
                },
                refresh_structure: false,
            },
        ),
        TableCommand::Review(72, 9, MutationPlan { operations: vec![] }),
    ];
    for command in commands {
        let fake = Fake::new(false, DataCloseOutcome::Closed);
        let (worker, controls, receiver) = worker(fake.clone());
        controls.send(count_command()).unwrap();
        fake.request_started.notified().await;
        controls.send(command).unwrap();
        controls.cancel();
        tokio::time::timeout(Duration::from_secs(1), fake.cancelled.notified())
            .await
            .unwrap();
        assert!(matches!(
            receiver.try_recv().unwrap().into_message(),
            TableMessage::Opened
        ));
        match receiver.try_recv().unwrap().into_message() {
            TableMessage::Analysis(71, Err(error)) | TableMessage::Reviewed(72, Err(error)) => {
                assert!(matches!(
                    error.as_ref(),
                    DataError::Mutation(ResultMutationError::Cancelled)
                ));
            }
            _ => panic!("queued mutation cancellation lost its family or ID"),
        }
        controls.stop();
        assert!(matches!(
            tokio::time::timeout(Duration::from_secs(1), worker.join)
                .await
                .unwrap(),
            Ok(DataCloseOutcome::Closed)
        ));
        assert_eq!(fake.closes.load(Ordering::Relaxed), 1);
    }
}

#[test]
fn mutation_delivery_preserves_unknown_outcomes_and_releases_shared_budget() {
    let (wake, _) = async_channel::bounded(1);
    let budget = ByteBudget::new(WORKSPACE_QUEUE_BYTES);
    let (_controls, receiver, delivery, ..) = channels(budget.clone(), wake);
    // Losing the socket after apply admission cannot be presented as a known
    // rollback. Keep the exact error for the UI's durable OutcomeUnknown state.
    let error = Arc::new(DataError::Mutation(ResultMutationError::ConnectionLost));
    assert!(delivery.send(TableMessage::Applied(81, Err(error.clone()))));
    assert!(budget.used() > 0);
    match receiver.try_recv().unwrap().into_message() {
        TableMessage::Applied(81, Err(received)) => assert!(Arc::ptr_eq(&received, &error)),
        _ => panic!("apply outcome was changed during delivery"),
    }
    assert_eq!(budget.used(), 0);
    assert!(delivery.send(TableMessage::Applied(
        82,
        Ok(MutationSubmission::Applied(ApplyResult {
            operations: vec![AppliedOperation {
                op_index: 0,
                rows_affected: 1
            }],
            runtime_ms: 2,
        }))
    )));
    assert!(budget.used() > 0);
    drop(receiver);
    assert_eq!(budget.used(), 0);
}

#[tokio::test]
async fn cancel_after_dequeue_before_first_poll_never_dispatches_the_command() {
    let fake = Fake::new(false, DataCloseOutcome::Closed);
    let (wake, _) = async_channel::bounded(1);
    let (controls, receiver, delivery, mut commands, mut cancellation, mut stop) =
        channels(ByteBudget::new(WORKSPACE_QUEUE_BYTES), wake);
    // Reproduce the interval after the worker takes a command but before it
    // polls that request. The independent watch is already ready to win select.
    let command = TableCommand::Review(91, 7, MutationPlan { operations: vec![] });
    controls.cancel();
    assert!(
        run_request(
            &fake,
            &(),
            command,
            &delivery,
            &mut commands,
            &mut cancellation,
            &mut stop,
            &tokio::sync::Mutex::new(()),
        )
        .await
    );
    assert!(fake.request_started.notified().now_or_never().is_none());
    assert_eq!(fake.cancellations.load(Ordering::Relaxed), 0);
    match receiver.try_recv().unwrap().into_message() {
        TableMessage::Reviewed(91, Err(error)) => assert!(matches!(
            error.as_ref(),
            DataError::Mutation(ResultMutationError::Cancelled)
        )),
        _ => panic!("never-dispatched review did not return its own cancellation"),
    }
}

#[tokio::test]
async fn preference_lane_fences_waiters_and_joins_dispatched_commit_before_close() {
    let first = Fake::new(false, DataCloseOutcome::Closed);
    let second = Fake::new(false, DataCloseOutcome::Closed);
    let lane = Arc::new(tokio::sync::Mutex::new(()));
    let budget = ByteBudget::new(WORKSPACE_QUEUE_BYTES);
    let (a, a_receive, a_delivery, mut a_commands, mut a_cancel, mut a_stop) =
        channels(budget.clone(), async_channel::bounded(1).0);
    let (b, b_receive, b_delivery, mut b_commands, mut b_cancel, mut b_stop) =
        channels(budget.clone(), async_channel::bounded(1).0);
    let pending = {
        let backend = first.clone();
        let lane = lane.clone();
        tokio::spawn(async move {
            run_request(
                &backend,
                &(),
                TableCommand::SavePreferences(
                    42,
                    "public".into(),
                    "rows".into(),
                    PreferencePatch::Mode(crate::browse_preferences::FilterMode::Raw),
                ),
                &a_delivery,
                &mut a_commands,
                &mut a_cancel,
                &mut a_stop,
                &lane,
            )
            .await
        })
    };
    first.request_started.notified().await;
    // A load on another document cannot read the profile during the commit.
    let waiting = {
        let backend = second.clone();
        let lane = lane.clone();
        tokio::spawn(async move {
            run_request(
                &backend,
                &(),
                TableCommand::LoadPreferences("public".into(), "rows".into()),
                &b_delivery,
                &mut b_commands,
                &mut b_cancel,
                &mut b_stop,
                &lane,
            )
            .await
        })
    };
    b.stop();
    assert!(!waiting.await.unwrap());
    assert!(second.request_started.notified().now_or_never().is_none());
    a.stop();
    assert!(!pending.is_finished());
    assert!(lane.try_lock().is_err());
    first.release_request.notify_one();
    assert!(!pending.await.unwrap());
    assert!(lane.try_lock().is_ok());
    drop(a_receive);
    drop(b_receive);
    assert_eq!(budget.used(), 0);
}

#[test]
fn cancelled_preferences_keep_the_exact_save_identity() {
    let message = cancelled(TableCommand::SavePreferences(
        73,
        "public".into(),
        "rows".into(),
        PreferencePatch::Mode(crate::browse_preferences::FilterMode::Raw),
    ));
    assert!(matches!(
        message,
        TableMessage::PreferencesSaved(73, Err(_))
    ));
}

#[tokio::test]
async fn virtual_key_commit_holds_profile_lane_until_settled_after_stop() {
    let backend = Fake::new(false, DataCloseOutcome::Closed);
    let lane = Arc::new(tokio::sync::Mutex::new(()));
    let (controls, receiver, delivery, mut commands, mut cancellation, mut stop) = channels(
        ByteBudget::new(WORKSPACE_QUEUE_BYTES),
        async_channel::bounded(1).0,
    );
    let task = {
        let backend = backend.clone();
        let lane = lane.clone();
        tokio::spawn(async move {
            run_request(
                &backend,
                &(),
                TableCommand::WriteVirtualKey(
                    81,
                    "public".into(),
                    "rows".into(),
                    Some(vec!["source_id".into()]),
                ),
                &delivery,
                &mut commands,
                &mut cancellation,
                &mut stop,
                &lane,
            )
            .await
        })
    };
    backend.request_started.notified().await;
    controls.stop();
    assert!(!task.is_finished());
    assert!(lane.try_lock().is_err());
    backend.release_request.notify_one();
    assert!(!task.await.unwrap());
    assert!(lane.try_lock().is_ok());
    assert!(receiver.try_recv().is_none());
    assert!(matches!(
        cancelled(TableCommand::WriteVirtualKey(
            82,
            "public".into(),
            "rows".into(),
            None
        )),
        TableMessage::VirtualKeySaved(82, Err(_))
    ));
    assert!(matches!(
        cancelled(TableCommand::LoadVirtualKey(
            83,
            "public".into(),
            "rows".into()
        )),
        TableMessage::VirtualKeyLoaded(83, Err(_))
    ));
}

#[test]
fn reserved_signal_reply_keeps_its_queue_allowance_until_consumed_or_dropped() {
    let budget = ByteBudget::new(16 * 1024);
    let (wake, _) = async_channel::bounded(1);
    let (_, receiver, delivery, _, _, _) = channels(budget.clone(), wake);
    let permit = budget.reserve(16 * 1024).unwrap();
    assert!(budget.reserve(1).is_none());
    assert!(delivery.send_reserved(
        TableMessage::AdminApplied(71, Err(Arc::new(AdminControlError::Cancelled))),
        permit
    ));
    assert_eq!(budget.used(), 16 * 1024);
    let reply = receiver.try_recv().unwrap();
    assert!(
        matches!(&*reply, TableMessage::AdminApplied(71, Err(error)) if matches!(error.as_ref(), AdminControlError::Cancelled))
    );
    drop(reply);
    assert_eq!(budget.used(), 0);
}
