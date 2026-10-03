use super::*;
fn budget() -> Rc<Cell<usize>> {
    Rc::new(Cell::new(0))
}
fn request() -> ResultRequest {
    ResultRequest {
        identity: ResultIdentity {
            job_id: "job".into(),
            result_id: "result".into(),
        },
        source: Endpoint {
            connection_id: "source".into(),
            schema: "Case.Source".into(),
        },
        target: Endpoint {
            connection_id: "target".into(),
            schema: "Case.Target".into(),
        },
    }
}
fn relation(name: &str) -> RelationIdentity {
    RelationIdentity {
        kind: RelationKind::Table,
        name: name.into(),
    }
}
fn object(name: &str) -> ObjectSummary {
    ObjectSummary {
        difference: SummaryDifference::Equal {
            source: relation(name),
            target: relation(name),
        },
        field_count: 2,
        changed_fields: 0,
        incomparable_fields: 0,
    }
}
fn page(dispatch: &Dispatch, reply: CompareReply) -> SchemaComparisonPage {
    SchemaComparisonPage {
        response_id: uuid::Uuid::new_v4().to_string(),
        request: dispatch.request.clone(),
        read: dispatch.read.clone(),
        reply,
    }
}
fn metadata(request: &ResultRequest) -> CompareReply {
    CompareReply::Metadata {
        metadata: ComparisonMetadata {
            identity: request.identity.clone(),
            source: CaptureMetadata {
                endpoint: request.source.clone(),
                server_version: "16.15".into(),
                server_version_num: 160015,
                captured_at: "2026-10-03T00:00:00Z".into(),
            },
            target: CaptureMetadata {
                endpoint: request.target.clone(),
                server_version: "16.15".into(),
                server_version_num: 160015,
                captured_at: "2026-10-03T00:00:01Z".into(),
            },
            consistency: SnapshotConsistency::IndependentTransactions,
            coverage: Coverage {
                scope: "postgres16OrdinaryTableProjectionV1".into(),
                normalization_version: 1,
                excluded_relations: 0,
                incomparable_fields: 0,
                excluded_categories: vec![ExcludedCategory::Routines],
            },
        },
        kind: DifferenceKind::Equal,
        object_count: 2,
        source_excluded_counts: vec![],
        target_excluded_counts: vec![],
    }
}
fn deliver(
    state: &mut ReaderState,
    dispatch: Dispatch,
    reply: CompareReply,
    shared: Rc<Cell<usize>>,
) -> Option<Dispatch> {
    let page = page(&dispatch, reply);
    let lease = PageLease::new(&page, shared).unwrap();
    state.accept(dispatch.token, page, lease).unwrap()
}
fn opened(shared: Rc<Cell<usize>>) -> ReaderState {
    let mut state = ReaderState::new(shared.clone()).unwrap();
    let dispatch = state.enqueue(Intent::Open(request())).unwrap().unwrap();
    let reply = metadata(&dispatch.request);
    let objects = deliver(&mut state, dispatch, reply, shared.clone()).unwrap();
    assert!(
        deliver(
            &mut state,
            objects,
            CompareReply::Objects {
                offset: 0,
                next_offset: Some(1),
                items: vec![object("a")]
            },
            shared
        )
        .is_none()
    );
    state
}
#[test]
fn page_history_commits_only_after_exact_successful_reply() {
    let shared = budget();
    let mut state = opened(shared.clone());
    let next = state
        .enqueue(Intent::ObjectPage(Turn::Next))
        .unwrap()
        .unwrap();
    assert!(matches!(next.read, ReadRequest::Objects { offset: 1 }));
    state.fail(next.token, false).unwrap();
    assert!(!state.can_turn_objects(Turn::Previous));
    assert!(state.can_turn_objects(Turn::Next));
    let next = state.enqueue(Intent::Retry).unwrap().unwrap();
    let mut wrong = page(
        &next,
        CompareReply::Objects {
            offset: 1,
            next_offset: None,
            items: vec![object("b")],
        },
    );
    wrong.request.target.schema = "different".into();
    let lease = PageLease::new(&wrong, shared.clone()).unwrap();
    assert!(state.accept(next.token, wrong, lease).is_err());
    assert!(!state.can_turn_objects(Turn::Previous));
    let next = state
        .enqueue(Intent::ObjectPage(Turn::Next))
        .unwrap()
        .unwrap();
    deliver(
        &mut state,
        next,
        CompareReply::Objects {
            offset: 1,
            next_offset: None,
            items: vec![object("b")],
        },
        shared.clone(),
    );
    let previous = state
        .enqueue(Intent::ObjectPage(Turn::Previous))
        .unwrap()
        .unwrap();
    assert!(matches!(previous.read, ReadRequest::Objects { offset: 0 }));
    deliver(
        &mut state,
        previous,
        CompareReply::Objects {
            offset: 0,
            next_offset: Some(1),
            items: vec![object("a")],
        },
        shared.clone(),
    );
    assert!(!state.can_turn_objects(Turn::Previous));
    drop(state);
    assert_eq!(shared.get(), 0);
}
#[test]
fn stale_reply_settles_without_capture_and_only_latest_intent_dispatches() {
    let shared = budget();
    let mut state = ReaderState::new(shared.clone()).unwrap();
    let first = state.enqueue(Intent::Open(request())).unwrap().unwrap();
    let mut second = request();
    second.identity.result_id = "new-result".into();
    state.enqueue(Intent::Open(second.clone())).unwrap();
    assert!(!state.accepts(&first.token));
    let latest = state.fail(first.token, false).unwrap().unwrap();
    assert_eq!(latest.request, second);
    assert!(state.metadata().is_none());
    assert_eq!(shared.get(), 1024 * 1024);
    let stale_page = page(&first, metadata(&first.request));
    let lease = PageLease::new(&stale_page, shared.clone()).unwrap();
    assert!(
        state
            .accept(first.token, stale_page, lease)
            .unwrap()
            .is_none()
    );
    assert!(state.accepts(&latest.token));
    assert_eq!(shared.get(), 1024 * 1024);
    state.close();
    assert!(!state.accepts(&latest.token));
    assert!(state.fail(latest.token, false).unwrap().is_none());
    drop(state);
    assert_eq!(shared.get(), 0);
}
#[test]
fn value_previous_uses_observed_utf8_cut_and_sides_remain_independent() {
    let shared = budget();
    let mut state = opened(shared.clone());
    let fields = state
        .enqueue(Intent::SelectObject(relation("a")))
        .unwrap()
        .unwrap();
    let path = FieldPath::Column {
        name: "text".into(),
        field: ColumnField::Comment,
    };
    let source = ValueRef {
        side: Side::Source,
        value_id: 1,
        raw_bytes: 65536,
        value_kind: ValueKind::Text,
    };
    let target = ValueRef {
        side: Side::Target,
        value_id: 2,
        raw_bytes: 0,
        value_kind: ValueKind::Text,
    };
    deliver(
        &mut state,
        fields,
        CompareReply::Fields {
            object: relation("a"),
            offset: 0,
            next_offset: None,
            items: vec![FieldSummary {
                path: path.clone(),
                difference: SummaryDifference::Changed { source, target },
            }],
        },
        shared.clone(),
    );
    let first = state.enqueue(Intent::SelectField(path)).unwrap().unwrap();
    let text = format!("{}€", "a".repeat(65532));
    assert_eq!(text.len(), 65535);
    deliver(
        &mut state,
        first,
        CompareReply::Value {
            value: source,
            offset: 0,
            text,
            next_offset: 65535,
            complete: false,
        },
        shared.clone(),
    );
    assert!(matches!(
        value_state(&state, Side::Target),
        ValueState::Empty
    ));
    let next = state
        .enqueue(Intent::ValuePage(Side::Source, Turn::Next))
        .unwrap()
        .unwrap();
    assert!(matches!(
        next.read,
        ReadRequest::Value { offset: 65535, .. }
    ));
    deliver(
        &mut state,
        next,
        CompareReply::Value {
            value: source,
            offset: 65535,
            text: "b".into(),
            next_offset: 65536,
            complete: true,
        },
        shared.clone(),
    );
    let previous = state
        .enqueue(Intent::ValuePage(Side::Source, Turn::Previous))
        .unwrap()
        .unwrap();
    assert!(matches!(
        previous.read,
        ReadRequest::Value { offset: 0, .. }
    ));
    assert!(!state.can_turn_value(Side::Target, Turn::Previous));
    drop(state);
    assert_eq!(shared.get(), 0);
}
#[test]
fn field_identity_does_not_use_colliding_display_paths() {
    let shared = budget();
    let mut state = opened(shared.clone());
    let fields = state
        .enqueue(Intent::SelectObject(relation("a")))
        .unwrap()
        .unwrap();
    let first = FieldPath::Index {
        name: "x (constraint y)".into(),
        owner: None,
        field: IndexField::Unique,
    };
    let second = FieldPath::Index {
        name: "x".into(),
        owner: Some("y".into()),
        field: IndexField::Unique,
    };
    assert_eq!(field_label(&first), field_label(&second));
    let value = ValueRef {
        side: Side::Source,
        value_id: 1,
        raw_bytes: 0,
        value_kind: ValueKind::Text,
    };
    deliver(
        &mut state,
        fields,
        CompareReply::Fields {
            object: relation("a"),
            offset: 0,
            next_offset: None,
            items: vec![
                FieldSummary {
                    path: first.clone(),
                    difference: SummaryDifference::SourceOnly { source: value },
                },
                FieldSummary {
                    path: second.clone(),
                    difference: SummaryDifference::SourceOnly { source: value },
                },
            ],
        },
        shared.clone(),
    );
    assert!(
        state
            .enqueue(Intent::SelectField(second.clone()))
            .unwrap()
            .is_none()
    );
    assert_eq!(state.selected_field(), Some(&second));
    assert!(matches!(
        value_state(&state, Side::Target),
        ValueState::Absent
    ));
    assert!(
        state
            .enqueue(Intent::SelectField(first.clone()))
            .unwrap()
            .is_none()
    );
    assert_eq!(state.selected_field(), Some(&first));
}
#[test]
fn replacement_admission_refusal_retains_old_capture_and_releases_overlap() {
    let shared = budget();
    let mut state = opened(shared.clone());
    let used = shared.get();
    let next = state
        .enqueue(Intent::ObjectPage(Turn::Next))
        .unwrap()
        .unwrap();
    let next_page = page(
        &next,
        CompareReply::Objects {
            offset: 1,
            next_offset: None,
            items: vec![object("b")],
        },
    );
    shared.set(WORKSPACE_BYTES);
    assert!(PageLease::new(&next_page, shared.clone()).is_err());
    assert!(matches!(
        state.objects(),
        Some(CompareReply::Objects { offset: 0, .. })
    ));
    shared.set(used);
    let lease = PageLease::new(&next_page, shared.clone()).unwrap();
    assert!(shared.get() > used);
    state.accept(next.token, next_page, lease).unwrap();
    assert!(matches!(
        state.objects(),
        Some(CompareReply::Objects { offset: 1, .. })
    ));
    state.close();
    assert_eq!(shared.get(), 1024 * 1024);
    drop(state);
    assert_eq!(shared.get(), 0);
}
#[test]
fn unavailable_never_becomes_empty_and_excluded_counterpart_is_not_absence() {
    let shared = budget();
    let mut state = ReaderState::new(shared.clone()).unwrap();
    let start = state.enqueue(Intent::Open(request())).unwrap().unwrap();
    let reply = metadata(&start.request);
    let objects = deliver(&mut state, start, reply, shared.clone()).unwrap();
    let foreign = RelationIdentity {
        kind: RelationKind::ForeignTable,
        name: "a".into(),
    };
    deliver(
        &mut state,
        objects,
        CompareReply::Objects {
            offset: 0,
            next_offset: None,
            items: vec![ObjectSummary {
                difference: SummaryDifference::NotComparable {
                    reason: IncomparableReason::ExcludedCounterpart,
                    observed: ObservedSides::Both {
                        source: relation("a"),
                        target: foreign.clone(),
                    },
                },
                field_count: 0,
                changed_fields: 0,
                incomparable_fields: 0,
            }],
        },
        shared.clone(),
    );
    let source = state
        .enqueue(Intent::SelectObject(relation("a")))
        .unwrap()
        .unwrap();
    assert!(matches!(
        value_state(&state, Side::Source),
        ValueState::Unread
    ));
    assert!(matches!(
        value_state(&state, Side::Target),
        ValueState::Excluded(None)
    ));
    let target = deliver(
        &mut state,
        source,
        CompareReply::Eligibility {
            object: relation("a"),
            side: Side::Source,
            eligibility: Eligibility::Eligible,
        },
        shared.clone(),
    )
    .unwrap();
    state.fail(target.token, true).unwrap();
    assert!(state.objects().is_none());
    assert!(state.metadata().is_none());
    assert!(state.request().is_some());
}

#[test]
fn connection_choices_refuse_duplicate_or_large_retention_without_replacing_old() {
    fn choice(id: &str) -> ConnectionChoice {
        ConnectionChoice {
            id: id.into(),
            name: "Exact name".into(),
            database: "Case.Database".into(),
            environment: "development".into(),
            schemas: vec!["Case.Schema".into()],
        }
    }
    let shared = budget();
    let current =
        Connections::new(vec![choice("source"), choice("target")], shared.clone()).unwrap();
    let retained = shared.get();
    assert!(Connections::new(vec![choice("source"), choice("source")], shared.clone()).is_err());
    assert_eq!(shared.get(), retained);
    assert_eq!(current.get("source").unwrap().database, "Case.Database");
    let mut oversized = choice("source");
    oversized.name.reserve(1024 * 1024);
    assert!(Connections::new(vec![oversized], shared.clone()).is_err());
    assert_eq!(shared.get(), retained);
    shared.set(WORKSPACE_BYTES - 1);
    assert!(Connections::new(vec![choice("replacement")], shared.clone()).is_err());
    assert!(current.get("source").is_some());
    shared.set(retained);
    drop(current);
    assert_eq!(shared.get(), 0);
}

#[test]
fn job_selection_is_exact_and_release_never_substitutes_another_job() {
    fn job(id: &str) -> Status {
        let request = request();
        Status {
            job_id: id.into(),
            request_id: format!("request-{id}"),
            source: request.source,
            target: request.target,
            source_objects: 2,
            target_objects: 2,
            state: StatusState::Completed {
                result_id: format!("result-{id}"),
            },
        }
    }
    let shared = budget();
    let old = JobCapture::new(
        SchemaComparisonList {
            jobs: vec![job("a"), job("b")],
        },
        shared.clone(),
    )
    .unwrap();
    let new = JobCapture::new(
        SchemaComparisonList {
            jobs: vec![job("b")],
        },
        shared.clone(),
    )
    .unwrap();
    assert_eq!(shared.get(), 128 * 1024);
    assert!(old.row("a").is_some());
    assert!(new.row("a").is_none());
    assert_eq!(new.row("b").unwrap().job_id, "b");
    assert!(!new.has_active());
    assert!(
        JobCapture::new(
            SchemaComparisonList {
                jobs: vec![job("b"), job("b")]
            },
            shared.clone()
        )
        .is_err()
    );
    assert_eq!(shared.get(), 128 * 1024);
    drop(old);
    drop(new);
    assert_eq!(shared.get(), 0);
}

#[test]
fn cached_schema_names_preserve_exact_text_and_refuse_oversized_snapshots() {
    fn choice(schemas: Vec<String>) -> ConnectionChoice {
        ConnectionChoice {
            id: "source".into(),
            name: "Source".into(),
            database: "db".into(),
            environment: "development".into(),
            schemas,
        }
    }
    let shared = budget();
    let names = vec![" Case.Schema ".into(), "日本語".repeat(7)];
    let original = Connections::new(vec![choice(names.clone())], shared.clone()).unwrap();
    let retained = shared.get();
    assert_eq!(original.get("source").unwrap().schemas, names);
    for invalid in [
        vec!["n".into(); 513],
        vec!["日本語".repeat(8)],
        vec!["bad\0schema".into()],
    ] {
        assert!(Connections::new(vec![choice(invalid)], shared.clone()).is_err());
        assert_eq!(shared.get(), retained);
    }
    let mut inflated = String::with_capacity(1024 * 1024);
    inflated.push('x');
    assert!(Connections::new(vec![choice(vec![inflated])], shared.clone()).is_err());
    assert_eq!(original.get("source").unwrap().schemas, names);
    drop(original);
    assert_eq!(shared.get(), 0);
}
#[test]
fn unavailable_failure_distinguishes_failed_capture_from_expired_result() {
    let result = failure_text(&CompareError::Unavailable);
    assert!(result.contains("expired"));
    let job = job_failure_text(&CompareError::Unavailable);
    assert!(job.contains("schema may not exist"));
    assert!(job.contains("No result was produced"));
    assert!(!job.contains("expired"));
    let version = failure_text(&CompareError::UnsupportedVersion {
        side: Side::Target,
        version: "17.2".into(),
    });
    assert!(version.starts_with("Target"));
    assert!(version.contains("17.2"));
    assert!(version.contains("PostgreSQL 16"));
}
