use super::*;
use crate::schema_edit::fixtures::{existing, users};
use crate::schema_edit::model::*;
use crate::schema_edit::preview::PlanError;
use SchemaEngine::*;

fn draft(engine: SchemaEngine) -> (SchemaBaseline, SchemaDraft) {
    let baseline = users(engine);
    let draft = SchemaDraft::from_baseline(&baseline);
    (baseline, draft)
}

fn kinds(changes: &SchemaChangeSet) -> Vec<(SchemaObject, ChangeKind)> {
    changes
        .changes
        .iter()
        .map(|change| (change.object, change.kind.clone()))
        .collect()
}

#[test]
fn untouched_draft_has_no_changes_or_steps() {
    for engine in [Postgres, MySql, Sqlite] {
        let (baseline, draft) = draft(engine);
        assert!(diff(&baseline, &draft).is_empty());
        assert!(plan_steps(&baseline, &draft).is_empty());
        assert!(validate(&baseline, &draft).is_empty(), "{engine:?}");
    }
}

#[test]
fn rename_keeps_the_origin_and_is_never_drop_plus_add() {
    let (baseline, mut draft) = draft(Postgres);
    draft.table.column_mut(existing(3)).unwrap().name = "full_name".into();
    let changes = diff(&baseline, &draft);
    assert_eq!(
        kinds(&changes),
        vec![(
            SchemaObject::Column,
            ChangeKind::Renamed {
                from: "name".into(),
                to: "full_name".into()
            }
        )]
    );
    assert_eq!(changes.changes[0].destructive, None);
    // The index on the column follows it; nothing is re-created.
    assert_eq!(
        plan_steps(&baseline, &draft),
        vec![PlanStep::RenameColumn { id: BaselineId(3) }]
    );
}

#[test]
fn swapped_column_names_are_two_renames() {
    let (baseline, mut draft) = draft(Postgres);
    draft.table.column_mut(existing(2)).unwrap().name = "name".into();
    draft.table.column_mut(existing(3)).unwrap().name = "email".into();
    let changes = diff(&baseline, &draft);
    assert_eq!(changes.len(), 2);
    assert!(changes
        .changes
        .iter()
        .all(|c| matches!(c.kind, ChangeKind::Renamed { .. }) && c.destructive.is_none()));
    // Keys reference origins, so the unique key stays on the renamed column.
    assert!(changes
        .changes
        .iter()
        .all(|c| c.object == SchemaObject::Column));
    assert!(validate(&baseline, &draft).is_empty());
}

#[test]
fn untouched_expressions_compare_by_bytes() {
    let (baseline, mut draft) = draft(Postgres);
    // Re-assigning the same bytes is a no-op.
    draft.table.checks[0].expression = SqlFragment::new("length(email) > 3");
    assert!(diff(&baseline, &draft).is_empty());
    // Semantically equal but different bytes is a change: no guessing.
    draft.table.checks[0].expression = SqlFragment::new("length(email)>3");
    let changes = diff(&baseline, &draft);
    assert_eq!(changes.len(), 1);
    assert!(matches!(
        changes.changes[0].kind,
        ChangeKind::Changed {
            field: Field::Expression,
            ..
        }
    ));
}

#[test]
fn type_text_is_compared_normalized() {
    let (baseline, mut draft) = draft(Postgres);
    draft.table.column_mut(existing(2)).unwrap().data_type = "VARCHAR ( 100 )".into();
    assert!(diff(&baseline, &draft).is_empty());
}

#[test]
fn destructive_classification() {
    let (baseline, mut draft) = draft(Postgres);
    // Widening: varchar(100) → varchar(200).
    draft.table.column_mut(existing(2)).unwrap().data_type = "varchar(200)".into();
    // Narrowing: text → varchar(10).
    draft.table.column_mut(existing(3)).unwrap().data_type = "varchar(10)".into();
    // Set NOT NULL.
    draft.table.column_mut(existing(4)).unwrap().nullable = false;
    // New NOT NULL column without default.
    let added = draft.add_column("created_at", "timestamptz");
    draft.table.column_mut(added).unwrap().nullable = false;
    let changes = diff(&baseline, &draft);
    let destructive = |origin: Origin, field: Option<Field>| {
        changes
            .changes
            .iter()
            .find(|c| {
                c.origin == Some(origin)
                    && match (&c.kind, field) {
                        (ChangeKind::Changed { field: f, .. }, Some(want)) => *f == want,
                        (ChangeKind::Added, None) => true,
                        _ => false,
                    }
            })
            .and_then(|c| c.destructive)
    };
    assert_eq!(destructive(existing(2), Some(Field::DataType)), None);
    assert_eq!(
        destructive(existing(3), Some(Field::DataType)),
        Some(Destructive::NarrowingType)
    );
    assert_eq!(
        destructive(existing(4), Some(Field::Nullable)),
        Some(Destructive::SetNotNull)
    );
    assert_eq!(
        destructive(added, None),
        Some(Destructive::AddNotNullWithoutDefault)
    );
    // With a default it is safe.
    draft.table.column_mut(added).unwrap().default =
        DefaultValue::Expression(SqlFragment::new("now()"));
    assert_eq!(
        diff(&baseline, &draft)
            .changes
            .iter()
            .find(|c| c.origin == Some(added))
            .unwrap()
            .destructive,
        None
    );
}

#[test]
fn removing_a_column_drops_its_keys_and_indexes() {
    let (baseline, mut draft) = draft(Postgres);
    assert!(draft.remove_column(existing(3)));
    let changes = diff(&baseline, &draft);
    assert_eq!(
        kinds(&changes),
        vec![
            (SchemaObject::Column, ChangeKind::Removed),
            (SchemaObject::Index, ChangeKind::Removed),
        ]
    );
    assert_eq!(
        changes.changes[0].destructive,
        Some(Destructive::DropColumn)
    );
    assert_eq!(changes.changes[1].destructive, Some(Destructive::DropIndex));
    assert_eq!(
        plan_steps(&baseline, &draft),
        vec![
            PlanStep::DropIndex {
                id: BaselineId(500)
            },
            PlanStep::DropColumn { id: BaselineId(3) },
        ]
    );
}

#[test]
fn steps_follow_the_plan_order() {
    let (baseline, mut draft) = draft(Postgres);
    // Phase 8, then 7, 6, 5, 4, 3, 2, 1 in insertion order, to prove sorting.
    draft.table.comment = Some("people".into());
    draft.table.foreign_keys[0].on_delete = ReferentialAction::Restrict;
    let index = draft.new_origin();
    draft.table.indexes.push(IndexModel {
        origin: index,
        name: "users_email_idx".into(),
        unique: false,
        method: None,
        members: vec![IndexMember::column(existing(2))],
        include: Vec::new(),
        predicate: None,
    });
    draft.remove_column(existing(3));
    let added = draft.add_column("age", "integer");
    draft.table.column_mut(existing(2)).unwrap().default =
        DefaultValue::Literal("x@example.com".into());
    draft.table.name = "people".into();
    draft.table.checks.clear();
    let steps = plan_steps(&baseline, &draft);
    let phases: Vec<u8> = steps.iter().map(PlanStep::phase).collect();
    let mut sorted = phases.clone();
    sorted.sort();
    assert_eq!(phases, sorted);
    assert_eq!(
        steps,
        vec![
            PlanStep::DropForeignKey {
                id: BaselineId(300)
            },
            PlanStep::DropCheck {
                id: BaselineId(400)
            },
            PlanStep::DropIndex {
                id: BaselineId(500)
            },
            PlanStep::RenameTable,
            PlanStep::AlterColumn {
                id: BaselineId(2),
                fields: vec![ColumnField::Default]
            },
            PlanStep::AddColumn { origin: added },
            PlanStep::DropColumn { id: BaselineId(3) },
            PlanStep::CreateIndex {
                origin: draft.table.indexes[0].origin
            },
            PlanStep::AddForeignKey {
                origin: existing(300)
            },
            PlanStep::SetTableComment,
        ]
    );
}

#[test]
fn foreign_keys_are_recreated_around_a_type_change_of_their_columns() {
    let (baseline, mut draft) = draft(Postgres);
    draft.table.column_mut(existing(4)).unwrap().data_type = "integer".into();
    assert_eq!(
        plan_steps(&baseline, &draft),
        vec![
            PlanStep::DropForeignKey {
                id: BaselineId(300)
            },
            PlanStep::AlterColumn {
                id: BaselineId(4),
                fields: vec![ColumnField::DataType]
            },
            PlanStep::AddForeignKey {
                origin: existing(300)
            },
        ]
    );
}

#[test]
fn constraint_rename_alone_is_a_rename_step() {
    let (baseline, mut draft) = draft(Postgres);
    draft.table.uniques[0].name = Some("users_email_uq".into());
    draft.table.indexes[0].name = "users_name_ix".into();
    assert_eq!(
        plan_steps(&baseline, &draft),
        vec![
            PlanStep::RenameConstraint {
                object: SchemaObject::Unique,
                id: BaselineId(200)
            },
            PlanStep::RenameConstraint {
                object: SchemaObject::Index,
                id: BaselineId(500)
            },
        ]
    );
}

#[test]
fn reorder_is_detected_with_the_fewest_moved_columns() {
    let (baseline, mut draft) = draft(MySql);
    // Move org_id to the front: only org_id moved.
    draft.move_column(existing(4), 0);
    let changes = diff(&baseline, &draft);
    assert_eq!(changes.len(), 1);
    assert_eq!(changes.changes[0].origin, Some(existing(4)));
    assert!(matches!(
        changes.changes[0].kind,
        ChangeKind::Changed {
            field: Field::Position,
            ..
        }
    ));
    assert!(plan_steps(&baseline, &draft).contains(&PlanStep::ReorderColumns));
    // A new column placed before an existing one is also a reorder.
    let (baseline, mut draft) = super::tests::draft(MySql);
    let added = draft.add_column("first", "int");
    draft.move_column(added, 0);
    assert!(columns_reordered(&baseline.table, &draft.table));
    // Appended: no reorder.
    let (baseline, mut draft) = super::tests::draft(MySql);
    draft.add_column("last", "int");
    assert!(!columns_reordered(&baseline.table, &draft.table));
}

#[test]
fn changed_index_is_dropped_and_recreated() {
    let (baseline, mut draft) = draft(Sqlite);
    draft.table.indexes[0].unique = true;
    let changes = diff(&baseline, &draft);
    assert_eq!(changes.changes[0].destructive, Some(Destructive::Recreate));
    assert_eq!(
        plan_steps(&baseline, &draft),
        vec![
            PlanStep::DropIndex {
                id: BaselineId(500)
            },
            PlanStep::CreateIndex {
                origin: existing(500)
            },
        ]
    );
}

#[test]
fn widening_rules() {
    assert!(is_widening(Postgres, "varchar(10)", "varchar(20)"));
    assert!(is_widening(Postgres, "character varying(10)", "varchar"));
    assert!(is_widening(Postgres, "varchar(10)", "text"));
    assert!(is_widening(Postgres, "integer", "bigint"));
    assert!(is_widening(Postgres, "int4", "int8"));
    assert!(is_widening(Postgres, "smallint", "numeric"));
    assert!(is_widening(Postgres, "numeric(10,2)", "numeric(12,2)"));
    assert!(is_widening(Postgres, "real", "double precision"));
    assert!(!is_widening(Postgres, "varchar(20)", "varchar(10)"));
    assert!(!is_widening(Postgres, "bigint", "integer"));
    assert!(!is_widening(Postgres, "numeric(10,2)", "numeric(12,3)"));
    assert!(!is_widening(Postgres, "text", "varchar(10)"));
    assert!(!is_widening(Postgres, "timestamp", "timestamptz"));
    assert!(is_widening(MySql, "int", "bigint"));
    assert!(is_widening(MySql, "int(11)", "bigint(20)"));
    assert!(is_widening(MySql, "int unsigned", "bigint unsigned"));
    assert!(!is_widening(MySql, "int unsigned", "bigint"));
    assert!(!is_widening(MySql, "varchar(10)", "text"));
    assert!(is_widening(MySql, "varchar(10)", "mediumtext"));
    assert!(is_widening(MySql, "blob", "longblob"));
    assert!(!is_widening(MySql, "varchar(10)", "varchar"));
    assert!(is_widening(Sqlite, "varchar(10)", "text"));
}

#[test]
fn validation_reports_shape_problems() {
    let (baseline, mut draft) = draft(Postgres);
    draft.table.column_mut(existing(3)).unwrap().name = "email".into();
    let bad_type = draft.add_column("x", "int; drop table users");
    draft.table.checks[0].expression = SqlFragment::new("1); drop table users; --");
    let unique = draft.new_origin();
    draft.table.uniques.push(KeyModel {
        origin: unique,
        name: Some("users_pkey".into()),
        columns: vec![],
    });
    draft.table.indexes[0].name = "x".repeat(64);
    let issues = validate(&baseline, &draft);
    let messages: Vec<&str> = issues.iter().map(|i| i.message.as_str()).collect();
    assert!(
        messages
            .iter()
            .any(|m| m.contains("used twice") && m.contains("email")),
        "{messages:?}"
    );
    assert!(issues
        .iter()
        .any(|i| i.origin == Some(bad_type) && i.message.starts_with("Type")));
    assert!(messages.iter().any(|m| m.starts_with("Check")));
    assert!(messages.contains(&"has no columns"));
    assert!(messages
        .iter()
        .any(|m| m.contains("users_pkey") && m.contains("twice")));
    assert!(messages
        .iter()
        .any(|m| m.starts_with("Index name is longer")));
}

#[test]
fn validation_refuses_forged_origins() {
    let (baseline, mut draft) = draft(Postgres);
    draft
        .table
        .columns
        .push(ColumnModel::new(existing(99), "ghost", "int"));
    draft
        .table
        .columns
        .push(ColumnModel::new(Origin::New(DraftId(50)), "future", "int"));
    let issues = validate(&baseline, &draft);
    assert!(issues.iter().any(|i| i.origin == Some(existing(99))));
    assert!(issues
        .iter()
        .any(|i| i.origin == Some(Origin::New(DraftId(50)))));
}

#[test]
fn column_name_case_follows_the_engine() {
    let (baseline, mut draft) = draft(Postgres);
    draft.add_column("Email", "text");
    assert!(validate(&baseline, &draft).is_empty());
    let (baseline, mut draft) = super::tests::draft(MySql);
    draft.add_column("Email", "text");
    assert!(!validate(&baseline, &draft).is_empty());
}

#[test]
fn revert_column_restores_baseline_attributes() {
    let (baseline, mut draft) = draft(Postgres);
    let column = draft.table.column_mut(existing(3)).unwrap();
    column.name = "n".into();
    column.nullable = false;
    assert!(draft.revert_column(&baseline, existing(3)));
    assert!(diff(&baseline, &draft).is_empty());
    let added = draft.add_column("tmp", "int");
    assert!(draft.revert_column(&baseline, added));
    assert!(diff(&baseline, &draft).is_empty());
}

#[test]
fn object_count_counts_each_object_once() {
    let (baseline, mut draft) = draft(Postgres);
    let column = draft.table.column_mut(existing(3)).unwrap();
    column.name = "n".into();
    column.nullable = false;
    column.comment = Some("x".into());
    draft.table.comment = Some("t".into());
    let changes = diff(&baseline, &draft);
    assert_eq!(changes.len(), 4);
    assert_eq!(changes.object_count(), 2);
}

#[test]
fn plan_review_refuses_before_any_planner_runs() {
    let (mut baseline, draft) = draft(Postgres);
    assert_eq!(
        crate::schema_edit::plan_review(&baseline, &draft, &Default::default()),
        Err(PlanError::NoChanges)
    );
    baseline.editable = Editability::ReadOnly {
        reasons: vec!["old server".into()],
    };
    assert!(matches!(
        crate::schema_edit::plan_review(&baseline, &draft, &Default::default()),
        Err(PlanError::ReadOnly { .. })
    ));
    let (baseline, mut draft) = super::tests::draft(Postgres);
    draft.table.columns.clear();
    assert!(matches!(
        crate::schema_edit::plan_review(&baseline, &draft, &Default::default()),
        Err(PlanError::Invalid { .. })
    ));
}

#[test]
fn model_round_trips_through_serde() {
    let (baseline, mut draft) = draft(MySql);
    draft.add_column("c", "int");
    let text = serde_json::to_string(&draft).unwrap();
    assert_eq!(serde_json::from_str::<SchemaDraft>(&text).unwrap(), draft);
    let text = serde_json::to_string(&baseline).unwrap();
    assert_eq!(
        serde_json::from_str::<SchemaBaseline>(&text).unwrap(),
        baseline
    );
}

#[test]
fn naming_an_unnamed_key_is_a_rename_and_clearing_a_name_is_refused() {
    let (mut baseline, _) = draft(Sqlite);
    baseline.table.primary_key.as_mut().unwrap().name = None;
    let mut draft = SchemaDraft::from_baseline(&baseline);
    draft.table.primary_key.as_mut().unwrap().name = Some("users_pk".into());
    let changes = diff(&baseline, &draft);
    assert!(matches!(
        changes.changes[0].kind,
        ChangeKind::Renamed { .. }
    ));
    assert_eq!(
        plan_steps(&baseline, &draft),
        vec![PlanStep::RenameConstraint {
            object: SchemaObject::PrimaryKey,
            id: BaselineId(100)
        }]
    );

    let (baseline, mut draft) = super::tests::draft(Postgres);
    draft.table.checks[0].name = None;
    assert!(validate(&baseline, &draft)
        .iter()
        .any(|issue| issue.message.contains("keep a name")));
}
