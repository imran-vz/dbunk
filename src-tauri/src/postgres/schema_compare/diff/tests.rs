use super::*;
use crate::postgres::schema_compare::{
    capture::{
        test_support::{self, TestRelation},
        CaptureControl, CapturedEndpoint, CapturedValue,
    },
    values::ValueKind,
};
use serde_json::{json, Value};
use std::time::Duration;
use tokio::sync::watch;

fn control() -> CaptureControl {
    let (_tx, rx) = watch::channel(false);
    CaptureControl::new(tokio::time::Instant::now() + Duration::from_secs(60), rx)
}
fn id() -> ResultIdentity {
    ResultIdentity {
        job_id: "job".into(),
        result_id: "result".into(),
    }
}
fn relation(name: &str) -> RelationIdentity {
    RelationIdentity {
        kind: RelationKind::Table,
        name: name.into(),
    }
}
fn path(name: &str, field: ColumnField) -> FieldPath {
    FieldPath::Column {
        name: name.into(),
        field,
    }
}
fn comment() -> FieldPath {
    FieldPath::Table {
        field: TableField::Comment,
    }
}
fn text(value: &str) -> CapturedValue {
    CapturedValue::Text(value.into())
}
fn expression(value: &str) -> CapturedValue {
    CapturedValue::Expression {
        raw: value.into(),
        reason: None,
    }
}
fn table(oid: u32, name: &str, mut fields: Vec<(FieldPath, CapturedValue)>) -> TestRelation {
    fields.push((
        FieldPath::Table {
            field: TableField::Persistence,
        },
        text("permanent"),
    ));
    TestRelation {
        oid,
        entry: InventoryEntry {
            identity: relation(name),
            eligibility: Eligibility::Eligible,
        },
        fields,
    }
}
fn view(oid: u32, name: &str) -> TestRelation {
    TestRelation {
        oid,
        entry: InventoryEntry {
            identity: RelationIdentity {
                kind: RelationKind::View,
                name: name.into(),
            },
            eligibility: Eligibility::Excluded {
                reason: Exclusion::OtherKind,
            },
        },
        fields: vec![],
    }
}
fn excluded_table(oid: u32, name: &str, reason: Exclusion) -> TestRelation {
    TestRelation {
        oid,
        entry: InventoryEntry {
            identity: relation(name),
            eligibility: Eligibility::Excluded { reason },
        },
        fields: vec![],
    }
}
fn fixture(budget: &Budget, side: Side, version: u32, rows: Vec<TestRelation>) -> CapturedEndpoint {
    let (connection, schema) = match side {
        Side::Source => ("a", "Source"),
        Side::Target => ("b", "Target"),
    };
    test_support::fixture(budget, connection, schema, version, rows)
}
fn objects(result: &Comparison, now: Instant) -> Value {
    serde_json::from_str(
        result
            .object_page(&id(), "objects", 0, now)
            .unwrap()
            .as_str(),
    )
    .unwrap()
}
fn fields(result: &Comparison, table: &str, now: Instant) -> Value {
    serde_json::from_str(
        result
            .field_page(&id(), &relation(table), "fields", 0, now)
            .unwrap()
            .as_str(),
    )
    .unwrap()
}
fn raw(result: &Comparison, value: &Value, now: Instant) -> Value {
    let reference: ValueRef = serde_json::from_value(value.clone()).unwrap();
    let request = ValueRequest {
        identity: id(),
        value: reference,
        offset: 0,
    };
    serde_json::from_str(result.value_chunk(&request, "value", now).unwrap().as_str()).unwrap()
}

#[test]
fn stable_names_paths_and_value_ids_ignore_catalog_order_and_local_oids() {
    let budget = Budget::default().result_scope();
    let now = Instant::now();
    let make = |flipped| {
        let rows = |oid| {
            vec![
                table(oid, "z", vec![(comment(), text("same"))]),
                table(
                    oid + 9,
                    "A\" mixed",
                    vec![
                        (path("x", ColumnField::Position), CapturedValue::Integer(1)),
                        (
                            path("x", ColumnField::Default),
                            expression("'Source.literal'::text"),
                        ),
                    ],
                ),
            ]
        };
        let mut source = fixture(
            &budget,
            Side::Source,
            160015,
            rows(if flipped { 919 } else { 1 }),
        );
        let mut target = fixture(
            &budget,
            Side::Target,
            160015,
            rows(if flipped { 3 } else { 99 }),
        );
        if flipped {
            test_support::shuffled(&mut source);
            test_support::shuffled(&mut target);
        }
        compare(id(), source, target, &control(), now).unwrap()
    };
    let baseline = make(false);
    let shuffled = make(true);
    assert_eq!(baseline.kind(), DifferenceKind::Equal);
    assert_eq!(objects(&baseline, now), objects(&shuffled, now));
    assert_eq!(
        objects(&baseline, now)["items"][0]["source"]["name"],
        "A\" mixed"
    );
    for name in ["A\" mixed", "z"] {
        let left = fields(&baseline, name, now);
        let right = fields(&shuffled, name, now);
        assert_eq!(left, right);
        for field in left["items"].as_array().unwrap() {
            assert_eq!(
                raw(&baseline, &field["source"], now),
                raw(&shuffled, &field["source"], now)
            );
        }
    }
    drop(baseline);
    drop(shuffled);
    assert_eq!(budget.used(), 0);
}

#[test]
fn changes_incomparability_and_ordered_facts_coexist_without_rewriting_literals() {
    let budget = Budget::default().result_scope();
    let now = Instant::now();
    let make = |side| {
        let source = side == Side::Source;
        let selected = QualifiedName {
            namespace: Namespace::Selected,
            name: "custom_type".into(),
        };
        fixture(
            &budget,
            side,
            160015,
            vec![table(
                if source { 1 } else { 99 },
                "orders",
                vec![
                    (comment(), text(if source { "source" } else { "target" })),
                    (
                        path("id", ColumnField::Position),
                        CapturedValue::Integer(if source { 1 } else { 2 }),
                    ),
                    (
                        path("id", ColumnField::Type),
                        CapturedValue::Reference(selected),
                    ),
                    (
                        path("id", ColumnField::Default),
                        expression(if source {
                            "external.f('Source.literal')"
                        } else {
                            "external.f('Target.literal')"
                        }),
                    ),
                    (
                        path("label", ColumnField::Default),
                        expression(if source {
                            "'Source.literal'::text"
                        } else {
                            "'Target.literal'::text"
                        }),
                    ),
                    (
                        FieldPath::Constraint {
                            name: "fk".into(),
                            field: ConstraintField::DeleteColumns,
                        },
                        CapturedValue::Names(if source {
                            vec!["tenant".into(), "id".into()]
                        } else {
                            vec!["id".into(), "tenant".into()]
                        }),
                    ),
                    (
                        FieldPath::Index {
                            name: "uq".into(),
                            owner: Some("uq".into()),
                            field: IndexField::Valid,
                        },
                        CapturedValue::Boolean(source),
                    ),
                ],
            )],
        )
    };
    let result = compare(
        id(),
        make(Side::Source),
        make(Side::Target),
        &control(),
        now,
    )
    .unwrap();
    assert_eq!(result.kind(), DifferenceKind::Changed);
    let object = &objects(&result, now)["items"][0];
    assert_eq!(object["kind"], "changed");
    assert_eq!(object["changedFields"], 5);
    assert_eq!(object["incomparableFields"], 1);
    assert_eq!(result.metadata.coverage.incomparable_fields, 1);
    let page = fields(&result, "orders", now);
    let fields = page["items"].as_array().unwrap();
    let unknown = fields
        .iter()
        .find(|f| f["kind"] == "notComparable")
        .unwrap();
    assert_eq!(unknown["reason"], "expressionOutsideSubset");
    assert_eq!(
        raw(&result, &unknown["source"], now)["text"],
        "external.f('Source.literal')"
    );
    assert_eq!(
        raw(&result, &unknown["target"], now)["text"],
        "external.f('Target.literal')"
    );
    let ty = fields
        .iter()
        .find(|f| f["path"]["field"] == "type")
        .unwrap();
    assert_eq!(ty["kind"], "equal");
    let decoded: Value =
        serde_json::from_str(raw(&result, &ty["source"], now)["text"].as_str().unwrap()).unwrap();
    assert_eq!(
        decoded,
        json!({"namespace":{"kind":"selected"},"name":"custom_type"})
    );
}

#[test]
fn absence_rename_and_excluded_counterparts_are_directional() {
    let budget = Budget::default().result_scope();
    let now = Instant::now();
    let make = |reverse| {
        let left = fixture(
            &budget,
            Side::Source,
            160015,
            vec![
                table(1, "old", vec![]),
                table(2, "collision", vec![(comment(), text("known"))]),
            ],
        );
        let right = fixture(
            &budget,
            Side::Target,
            160015,
            vec![table(3, "new", vec![]), view(4, "collision")],
        );
        if reverse {
            compare(id(), right, left, &control(), now).unwrap()
        } else {
            compare(id(), left, right, &control(), now).unwrap()
        }
    };
    for (reverse, old, new) in [
        (false, "sourceOnly", "targetOnly"),
        (true, "targetOnly", "sourceOnly"),
    ] {
        let result = make(reverse);
        let page = objects(&result, now);
        let items = page["items"].as_array().unwrap();
        assert_eq!(items[0]["kind"], "notComparable");
        assert_eq!(items[0]["reason"], "excludedCounterpart");
        assert_eq!(items[0]["fieldCount"], 0);
        assert_eq!(items[1]["kind"], new);
        assert_eq!(items[2]["kind"], old);
        let collision = fields(&result, "collision", now);
        assert!(collision["items"].as_array().unwrap().is_empty());
        assert_eq!(result.metadata.coverage.excluded_relations, 1);
    }
}

#[test]
fn excluded_relation_eligibility_survives_capture_consumption_and_binds_side() {
    let budget = Budget::default().result_scope();
    let now = Instant::now();
    let result = compare(
        id(),
        fixture(
            &budget,
            Side::Source,
            160015,
            vec![excluded_table(1, "inherited", Exclusion::Inherited)],
        ),
        fixture(&budget, Side::Target, 160015, vec![]),
        &control(),
        now,
    )
    .unwrap();
    assert_eq!(
        result
            .relation_eligibility(&id(), &relation("inherited"), Side::Source, now)
            .unwrap(),
        &Eligibility::Excluded {
            reason: Exclusion::Inherited
        }
    );
    assert!(matches!(
        result.relation_eligibility(&id(), &relation("inherited"), Side::Target, now),
        Err(CompareError::Unavailable)
    ));
    let wrong_kind = RelationIdentity {
        kind: RelationKind::View,
        name: "inherited".into(),
    };
    assert!(matches!(
        result.relation_eligibility(&id(), &wrong_kind, Side::Source, now),
        Err(CompareError::Unavailable)
    ));
    let wrong_result = ResultIdentity {
        result_id: "other".into(),
        ..id()
    };
    assert!(matches!(
        result.relation_eligibility(&wrong_result, &relation("inherited"), Side::Source, now),
        Err(CompareError::Unavailable)
    ));
}

#[test]
fn counterpart_exclusions_retain_the_reason_for_each_requested_side() {
    let budget = Budget::default().result_scope();
    let now = Instant::now();
    for reverse in [false, true] {
        let inherited = fixture(
            &budget,
            Side::Source,
            160015,
            vec![excluded_table(1, "shared", Exclusion::Inherited)],
        );
        let extension_owned = fixture(
            &budget,
            Side::Target,
            160015,
            vec![excluded_table(2, "shared", Exclusion::ExtensionOwned)],
        );
        let result = if reverse {
            compare(id(), extension_owned, inherited, &control(), now).unwrap()
        } else {
            compare(id(), inherited, extension_owned, &control(), now).unwrap()
        };
        assert_eq!(objects(&result, now)["items"][0]["kind"], "notComparable");
        assert_eq!(
            result
                .relation_eligibility(&id(), &relation("shared"), Side::Source, now)
                .unwrap(),
            &Eligibility::Excluded {
                reason: if reverse {
                    Exclusion::ExtensionOwned
                } else {
                    Exclusion::Inherited
                }
            }
        );
        assert_eq!(
            result
                .relation_eligibility(&id(), &relation("shared"), Side::Target, now)
                .unwrap(),
            &Eligibility::Excluded {
                reason: if reverse {
                    Exclusion::Inherited
                } else {
                    Exclusion::ExtensionOwned
                }
            }
        );
    }
}

#[test]
fn empty_supported_scope_and_excluded_only_schemas_have_distinct_outcomes() {
    let budget = Budget::default().result_scope();
    let now = Instant::now();
    let result = compare(
        id(),
        fixture(&budget, Side::Source, 160015, vec![]),
        fixture(&budget, Side::Target, 160015, vec![]),
        &control(),
        now,
    )
    .unwrap();
    assert_eq!(result.kind(), DifferenceKind::Equal);
    assert_eq!(result.metadata.coverage.scope, SCOPE);
    assert_eq!(
        result.metadata.coverage.normalization_version,
        NORMALIZATION_VERSION
    );
    assert_eq!(result.metadata.coverage.excluded_categories.len(), 13);
    assert_eq!(result.object_count(), 0);
    drop(result);
    let result = compare(
        id(),
        fixture(&budget, Side::Source, 160015, vec![view(7, "v")]),
        fixture(&budget, Side::Target, 160015, vec![]),
        &control(),
        now,
    )
    .unwrap();
    assert_eq!(result.kind(), DifferenceKind::NotComparable);
    assert_eq!(
        objects(&result, now)["items"][0]["reason"],
        "excludedObject"
    );
}

#[test]
fn different_minor_versions_and_unshared_transactions_cannot_claim_expression_equality() {
    let budget = Budget::default().result_scope();
    let now = Instant::now();
    let make = |side, version| {
        fixture(
            &budget,
            side,
            version,
            vec![table(
                1,
                "t",
                vec![(path("id", ColumnField::Default), expression("7"))],
            )],
        )
    };
    let result = compare(
        id(),
        make(Side::Source, 160015),
        make(Side::Target, 160014),
        &control(),
        now,
    )
    .unwrap();
    assert_eq!(result.kind(), DifferenceKind::NotComparable);
    assert_eq!(
        fields(&result, "t", now)["items"][1]["reason"],
        "renderingVersionDifference"
    );
    assert_eq!(result.metadata.coverage.incomparable_fields, 1);
    drop(result);
    assert!(matches!(
        compare(
            id(),
            make(Side::Source, 150012),
            make(Side::Target, 160015),
            &control(),
            now
        ),
        Err(CompareError::UnsupportedVersion {
            side: Side::Source,
            ..
        })
    ));
    let source = make(Side::Source, 160015);
    let mut target = make(Side::Target, 160015);
    target.metadata.endpoint.connection_id = source.metadata.endpoint.connection_id.clone();
    assert!(matches!(
        compare(id(), source, target, &control(), now),
        Err(CompareError::InvalidRequest)
    ));
    let source = make(Side::Source, 160015);
    let mut target = make(Side::Target, 160015);
    target.metadata.endpoint.connection_id = source.metadata.endpoint.connection_id.clone();
    test_support::share_snapshot(&source, &mut target);
    let result = compare(id(), source, target, &control(), now).unwrap();
    assert_eq!(
        result.metadata.consistency,
        SnapshotConsistency::SharedTransaction
    );
}

#[test]
fn result_pages_and_chunks_bound_payloads_and_validate_identity_ttl_and_side() {
    let budget = Budget::default().result_scope();
    let now = Instant::now();
    let mut many: Vec<_> = (0..101)
        .map(|i| {
            (
                path(&format!("c{i:03}"), ColumnField::Position),
                CapturedValue::Integer(i),
            )
        })
        .collect();
    many.push((comment(), text(&"\u{1}".repeat(FIELD_BYTES))));
    let result = compare(
        id(),
        fixture(&budget, Side::Source, 160015, vec![table(1, "t", many)]),
        fixture(&budget, Side::Target, 160015, vec![]),
        &control(),
        now,
    )
    .unwrap();
    let first = result
        .field_page(&id(), &relation("t"), "one", 0, now)
        .unwrap();
    assert!(first.bytes() <= PAGE_BYTES);
    let decoded: Value = serde_json::from_str(first.as_str()).unwrap();
    assert_eq!(decoded["items"].as_array().unwrap().len(), 100);
    assert_eq!(decoded["nextOffset"], 100);
    let next = result
        .field_page(&id(), &relation("t"), "two", 100, now)
        .unwrap();
    assert_eq!(
        serde_json::from_str::<Value>(next.as_str()).unwrap()["items"]
            .as_array()
            .unwrap()
            .len(),
        3
    );
    assert!(matches!(
        result.object_page(&id(), "busy", 0, now),
        Err(CompareError::Busy)
    ));
    drop(first);
    drop(next);
    let comment = &decoded["items"][1];
    assert_eq!(comment["path"]["field"], "comment");
    let value: ValueRef = serde_json::from_value(comment["source"].clone()).unwrap();
    assert_eq!(value.value_kind, ValueKind::Text);
    assert_eq!(value.raw_bytes as usize, FIELD_BYTES);
    let mut request = ValueRequest {
        identity: id(),
        value,
        offset: 0,
    };
    let mut bytes = 0;
    loop {
        let chunk = result.value_chunk(&request, "chunk", now).unwrap();
        assert!(chunk.bytes() <= PAGE_BYTES);
        let decoded: Value = serde_json::from_str(chunk.as_str()).unwrap();
        let text = decoded["text"].as_str().unwrap();
        assert!(text.len() <= CHUNK_BYTES);
        assert!(text.bytes().all(|b| b == 1));
        bytes += text.len();
        if decoded["complete"] == true {
            break;
        }
        request.offset = decoded["nextOffset"].as_u64().unwrap() as u32;
    }
    assert_eq!(bytes, FIELD_BYTES);
    request.value.side = Side::Target;
    assert!(matches!(
        result.value_chunk(&request, "bad-side", now),
        Err(CompareError::Unavailable)
    ));
    let wrong = ResultIdentity {
        result_id: "other".into(),
        ..id()
    };
    assert!(matches!(
        result.object_page(&wrong, "wrong", 0, now),
        Err(CompareError::Unavailable)
    ));
    assert!(matches!(
        result.field_page(&id(), &relation("missing"), "wrong", 0, now),
        Err(CompareError::Unavailable)
    ));
    let wrong_kind = RelationIdentity {
        kind: RelationKind::View,
        name: "t".into(),
    };
    assert!(matches!(
        result.field_page(&id(), &wrong_kind, "wrong", 0, now),
        Err(CompareError::Unavailable)
    ));
    assert!(matches!(
        result.field_page(&id(), &relation("t"), "bad-offset", 104, now),
        Err(CompareError::InvalidRequest)
    ));
    assert!(result
        .field_page(&id(), &relation("t"), "end", 103, now)
        .is_ok());
    assert!(matches!(
        result.object_page(&id(), "expired", 0, now + RESULT_TTL),
        Err(CompareError::Unavailable)
    ));
    assert!(matches!(
        result.field_page(&id(), &relation("t"), "expired", 0, now + RESULT_TTL),
        Err(CompareError::Unavailable)
    ));
    request.value = value;
    assert!(matches!(
        result.value_chunk(&request, "expired", now + RESULT_TTL),
        Err(CompareError::Unavailable)
    ));
    drop(result);
    assert_eq!(budget.used(), 0);
}

#[test]
fn cancelled_expired_and_over_budget_builds_release_every_partial_allocation() {
    let budget = Budget::default().result_scope();
    let now = Instant::now();
    let pair = || {
        (
            fixture(&budget, Side::Source, 160015, vec![table(1, "t", vec![])]),
            fixture(&budget, Side::Target, 160015, vec![table(2, "t", vec![])]),
        )
    };
    let (tx, rx) = watch::channel(false);
    tx.send(true).unwrap();
    let cancelled = CaptureControl::new(tokio::time::Instant::now() + Duration::from_secs(60), rx);
    let (source, target) = pair();
    assert!(matches!(
        compare(id(), source, target, &cancelled, now),
        Err(CompareError::Cancelled)
    ));
    assert_eq!(budget.used(), 0);
    let (_tx, rx) = watch::channel(false);
    let expired = CaptureControl::new(tokio::time::Instant::now(), rx);
    let (source, target) = pair();
    assert!(matches!(
        compare(id(), source, target, &expired, now),
        Err(CompareError::DeadlineExceeded)
    ));
    assert_eq!(budget.used(), 0);
    let (source, target) = pair();
    let held_bytes = RESULT_BYTES - budget.used() - 64 * 1024;
    let held = budget.reserve(held_bytes).unwrap();
    assert!(matches!(
        compare(id(), source, target, &control(), now),
        Err(CompareError::LimitExceeded {
            limit: Limit::ResultBytes
        })
    ));
    assert_eq!(budget.used(), held_bytes);
    drop(held);
    assert_eq!(budget.used(), 0);
    let other = Budget::default().result_scope();
    let source = fixture(&budget, Side::Source, 160015, vec![]);
    let target = fixture(&other, Side::Target, 160015, vec![]);
    assert!(matches!(
        compare(id(), source, target, &control(), now),
        Err(CompareError::InvalidRequest)
    ));
    assert_eq!(budget.used(), 0);
    assert_eq!(other.used(), 0);
}

#[test]
fn field_merge_checks_cancellation_between_units() {
    let budget = Budget::default().result_scope();
    let source = fixture(
        &budget,
        Side::Source,
        160015,
        vec![table(1, "t", vec![(comment(), text("x"))])],
    );
    let source: Vec<_> = source.fields().iter().collect();
    let (tx, rx) = watch::channel(false);
    let control = CaptureControl::new(tokio::time::Instant::now() + Duration::from_secs(60), rx);
    let mut visited = 0;
    let result = build::visit_fields(&source, &[], (160015, 160015), &control, |_| {
        visited += 1;
        tx.send(true).unwrap();
        Ok(())
    });
    assert_eq!(result, Err(CompareError::Cancelled));
    assert_eq!(visited, 1);
}
