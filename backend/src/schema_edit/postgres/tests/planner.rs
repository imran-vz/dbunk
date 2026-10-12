use super::super::planner::{
    binary_coercible, build, preserved_attributes, referenced_target, references_identifier,
    resolve_renames, server_major,
};
use super::{review, setup, sql};
use crate::schema_edit::fixtures::existing;
use crate::schema_edit::*;

fn plan_sql(baseline: &SchemaBaseline, draft: &SchemaDraft) -> Vec<String> {
    let preview = review(baseline, draft, &PlanOptions::default()).expect("plans");
    sql(&preview).into_iter().map(str::to_owned).collect()
}

fn unsupported(baseline: &SchemaBaseline, draft: &SchemaDraft) -> String {
    match review(baseline, draft, &PlanOptions::default()) {
        Err(PlanError::Unsupported { reason }) => reason,
        other => panic!("expected Unsupported, got {other:?}"),
    }
}

fn has_note(preview: &SchemaEditPreview, kind: NoteKind, text: &str) -> bool {
    preview
        .notes
        .iter()
        .any(|note| note.kind == kind && note.text.contains(text))
}

#[test]
fn adds_a_column_in_one_atomic_group_under_the_table_lock() {
    let (baseline, mut draft) = setup();
    draft.add_column("nick", "text");
    let preview = review(&baseline, &draft, &PlanOptions::default()).unwrap();
    assert_eq!(
        sql(&preview),
        vec![r#"ALTER TABLE "public"."users" ADD COLUMN "nick" text;"#]
    );
    assert_eq!(
        preview.statements[0].summary,
        "Add column public.users.nick"
    );
    assert!(!preview.statements[0].destructive);
    assert_eq!(
        preview.groups,
        vec![PreviewGroup::Atomic {
            statements: vec![0]
        }]
    );
    assert!(has_note(&preview, NoteKind::Lock, "ACCESS EXCLUSIVE"));
    assert!(has_note(&preview, NoteKind::Lock, "lock_timeout"));
}

#[test]
fn orders_drops_renames_adds_column_drops_and_comments() {
    let (baseline, mut draft) = setup();
    assert!(draft.remove_column(existing(4)));
    draft.table.checks.clear();
    draft.add_column("nick", "text");
    draft.table.name = "members".into();
    draft.table.comment = Some("People".into());
    assert_eq!(
        plan_sql(&baseline, &draft),
        vec![
            r#"ALTER TABLE "public"."users" DROP CONSTRAINT "users_org_id_fkey" RESTRICT;"#,
            r#"ALTER TABLE "public"."users" DROP CONSTRAINT "users_email_check" RESTRICT;"#,
            r#"ALTER TABLE "public"."users" RENAME TO "members";"#,
            r#"ALTER TABLE "public"."members" ADD COLUMN "nick" text;"#,
            r#"ALTER TABLE "public"."members" DROP COLUMN "org_id" RESTRICT;"#,
            r#"COMMENT ON TABLE "public"."members" IS E'People';"#,
        ]
    );
}

#[test]
fn renames_a_column_and_its_comment_follows_the_new_names() {
    let (baseline, mut draft) = setup();
    let column = draft.table.column_mut(existing(3)).unwrap();
    column.name = "full_name".into();
    column.comment = Some("Display name".into());
    draft.table.name = "people".into();
    assert_eq!(
        plan_sql(&baseline, &draft),
        vec![
            r#"ALTER TABLE "public"."users" RENAME COLUMN "name" TO "full_name";"#,
            r#"ALTER TABLE "public"."users" RENAME TO "people";"#,
            r#"COMMENT ON COLUMN "public"."people"."full_name" IS E'Display name';"#,
        ]
    );
}

#[test]
fn swapped_column_names_go_through_a_temporary_name() {
    let (baseline, mut draft) = setup();
    draft.table.column_mut(existing(2)).unwrap().name = "name".into();
    draft.table.column_mut(existing(3)).unwrap().name = "email".into();
    assert_eq!(
        plan_sql(&baseline, &draft),
        vec![
            r#"ALTER TABLE "public"."users" RENAME COLUMN "email" TO "email_dbunk_tmp";"#,
            r#"ALTER TABLE "public"."users" RENAME COLUMN "name" TO "email";"#,
            r#"ALTER TABLE "public"."users" RENAME COLUMN "email_dbunk_tmp" TO "name";"#,
        ]
    );
}

#[test]
fn renames_constraints_and_plain_indexes() {
    let (baseline, mut draft) = setup();
    draft.table.uniques[0].name = Some("users_email_uq".into());
    draft.table.indexes[0].name = "users_full_idx".into();
    assert_eq!(
        plan_sql(&baseline, &draft),
        vec![
            r#"ALTER TABLE "public"."users" RENAME CONSTRAINT "users_email_key" TO "users_email_uq";"#,
            r#"ALTER INDEX "public"."users_name_idx" RENAME TO "users_full_idx";"#,
        ]
    );
}

#[test]
fn a_reused_column_name_drops_the_old_column_first() {
    let (baseline, mut draft) = setup();
    assert!(draft.remove_column(existing(3)));
    draft.add_column("name", "integer");
    assert_eq!(
        plan_sql(&baseline, &draft),
        vec![
            r#"DROP INDEX "public"."users_name_idx" RESTRICT;"#,
            r#"ALTER TABLE "public"."users" DROP COLUMN "name" RESTRICT;"#,
            r#"ALTER TABLE "public"."users" ADD COLUMN "name" integer;"#,
        ]
    );
}

#[test]
fn type_changes_note_rewrites_unless_binary_coercible() {
    let (baseline, mut draft) = setup();
    draft.table.column_mut(existing(2)).unwrap().data_type = "text".into();
    let preview = review(&baseline, &draft, &PlanOptions::default()).unwrap();
    assert_eq!(
        sql(&preview),
        vec![r#"ALTER TABLE "public"."users" ALTER COLUMN "email" TYPE text;"#]
    );
    // varchar(100) → text widens: nothing is lost.
    assert!(!preview.statements[0].destructive);
    assert!(!preview.destructive());
    assert!(!preview
        .notes
        .iter()
        .any(|note| note.kind == NoteKind::Rewrite));
    assert!(has_note(&preview, NoteKind::Info, "needs no table rewrite"));

    let (baseline, mut draft) = setup();
    draft.table.column_mut(existing(3)).unwrap().data_type = "integer".into();
    let preview = review(&baseline, &draft, &PlanOptions::default()).unwrap();
    assert!(has_note(&preview, NoteKind::Rewrite, "rewrites the table"));
    // text → integer narrows.
    assert!(preview.statements[0].destructive);
}

#[test]
fn retyping_a_foreign_key_column_recreates_the_foreign_key_around_it() {
    let (baseline, mut draft) = setup();
    draft.table.column_mut(existing(4)).unwrap().data_type = "integer".into();
    assert_eq!(
        plan_sql(&baseline, &draft),
        vec![
            r#"ALTER TABLE "public"."users" DROP CONSTRAINT "users_org_id_fkey" RESTRICT;"#,
            r#"ALTER TABLE "public"."users" ALTER COLUMN "org_id" TYPE integer;"#,
            r#"ALTER TABLE "public"."users" ADD CONSTRAINT "users_org_id_fkey" FOREIGN KEY ("org_id") REFERENCES "public"."orgs" ("id") ON UPDATE NO ACTION ON DELETE CASCADE NOT DEFERRABLE;"#,
        ]
    );
}

#[test]
fn nullability_and_default_changes_are_ordered() {
    let (baseline, mut draft) = setup();
    let column = draft.table.column_mut(existing(3)).unwrap();
    column.nullable = false;
    column.default = DefaultValue::Literal("anon".into());
    assert_eq!(
        plan_sql(&baseline, &draft),
        vec![
            r#"ALTER TABLE "public"."users" ALTER COLUMN "name" SET NOT NULL;"#,
            r#"ALTER TABLE "public"."users" ALTER COLUMN "name" SET DEFAULT E'anon';"#,
        ]
    );
}

#[test]
fn a_replaced_default_is_dropped_before_a_type_change() {
    let (mut baseline, _) = setup();
    baseline.table.column_mut(existing(3)).unwrap().default =
        DefaultValue::Expression(SqlFragment::new("'x'::text"));
    let mut draft = SchemaDraft::from_baseline(&baseline);
    let column = draft.table.column_mut(existing(3)).unwrap();
    column.data_type = "varchar(10)".into();
    column.default = DefaultValue::Literal("y".into());
    assert_eq!(
        plan_sql(&baseline, &draft),
        vec![
            r#"ALTER TABLE "public"."users" ALTER COLUMN "name" DROP DEFAULT;"#,
            r#"ALTER TABLE "public"."users" ALTER COLUMN "name" TYPE varchar(10);"#,
            r#"ALTER TABLE "public"."users" ALTER COLUMN "name" SET DEFAULT E'y';"#,
        ]
    );
}

#[test]
fn identity_changes() {
    let (baseline, mut draft) = setup();
    draft.table.column_mut(existing(1)).unwrap().identity = Identity::Always;
    assert_eq!(
        plan_sql(&baseline, &draft),
        vec![r#"ALTER TABLE "public"."users" ALTER COLUMN "id" ADD GENERATED ALWAYS AS IDENTITY;"#]
    );

    let (mut baseline, _) = setup();
    baseline.table.column_mut(existing(1)).unwrap().identity = Identity::Always;
    let mut draft = SchemaDraft::from_baseline(&baseline);
    draft.table.column_mut(existing(1)).unwrap().identity = Identity::ByDefault;
    assert_eq!(
        plan_sql(&baseline, &draft),
        vec![r#"ALTER TABLE "public"."users" ALTER COLUMN "id" SET GENERATED BY DEFAULT;"#]
    );
    draft.table.column_mut(existing(1)).unwrap().identity = Identity::None;
    assert_eq!(
        plan_sql(&baseline, &draft),
        vec![r#"ALTER TABLE "public"."users" ALTER COLUMN "id" DROP IDENTITY;"#]
    );
}

fn generated_baseline(version: &str) -> SchemaBaseline {
    let (mut baseline, _) = setup();
    baseline.server.version = version.into();
    baseline.table.column_mut(existing(3)).unwrap().generated = Some(Generated {
        expression: SqlFragment::new("upper((email)::text)"),
        stored: true,
    });
    baseline
}

#[test]
fn generated_expression_changes_are_gated_by_server_version() {
    let baseline = generated_baseline("16.4");
    let mut draft = SchemaDraft::from_baseline(&baseline);
    draft
        .table
        .column_mut(existing(3))
        .unwrap()
        .generated
        .as_mut()
        .unwrap()
        .expression = SqlFragment::new("lower(email)");
    assert_eq!(
        unsupported(&baseline, &draft),
        "Changing the generated expression of name needs PostgreSQL 17 or later"
    );

    let baseline = generated_baseline("17.2");
    let mut later = SchemaDraft::from_baseline(&baseline);
    later.table = draft.table.clone();
    let preview = review(&baseline, &later, &PlanOptions::default()).unwrap();
    assert_eq!(
        sql(&preview),
        vec![
            r#"ALTER TABLE "public"."users" ALTER COLUMN "name" SET EXPRESSION AS (lower(email));"#
        ]
    );
    assert!(preview.statements[0].destructive);
    assert!(has_note(&preview, NoteKind::Rewrite, "Recomputes name"));
}

#[test]
fn a_generated_column_can_become_plain_but_not_the_reverse() {
    let baseline = generated_baseline("16.4");
    let mut draft = SchemaDraft::from_baseline(&baseline);
    draft.table.column_mut(existing(3)).unwrap().generated = None;
    assert_eq!(
        plan_sql(&baseline, &draft),
        vec![r#"ALTER TABLE "public"."users" ALTER COLUMN "name" DROP EXPRESSION;"#]
    );

    let (baseline, mut draft) = setup();
    draft.table.column_mut(existing(3)).unwrap().generated = Some(Generated {
        expression: SqlFragment::new("upper(email)"),
        stored: true,
    });
    assert!(unsupported(&baseline, &draft).contains("cannot turn the existing column name"));
}

#[test]
fn virtual_generated_columns_need_postgres_18() {
    for (version, supported) in [("17.0", false), ("18.0", true)] {
        let (mut baseline, _) = setup();
        baseline.server.version = version.into();
        let mut draft = SchemaDraft::from_baseline(&baseline);
        let origin = draft.add_column("shout", "text");
        draft.table.column_mut(origin).unwrap().generated = Some(Generated {
            expression: SqlFragment::new("upper(email)"),
            stored: false,
        });
        if supported {
            assert_eq!(
                plan_sql(&baseline, &draft),
                vec![
                    r#"ALTER TABLE "public"."users" ADD COLUMN "shout" text GENERATED ALWAYS AS (upper(email)) VIRTUAL;"#
                ]
            );
        } else {
            assert_eq!(
                unsupported(&baseline, &draft),
                "Virtual generated columns need PostgreSQL 18 or later"
            );
        }
    }
}

#[test]
fn collation_rides_on_the_type_change() {
    let (baseline, mut draft) = setup();
    draft.table.column_mut(existing(2)).unwrap().collation = Some("C".into());
    assert_eq!(
        plan_sql(&baseline, &draft),
        vec![r#"ALTER TABLE "public"."users" ALTER COLUMN "email" TYPE varchar(100) COLLATE "C";"#]
    );

    // A type change restates a kept collation, which it would otherwise reset.
    let (mut baseline, _) = setup();
    baseline.table.column_mut(existing(2)).unwrap().collation = Some("C".into());
    let mut draft = SchemaDraft::from_baseline(&baseline);
    draft.table.column_mut(existing(2)).unwrap().data_type = "text".into();
    assert_eq!(
        plan_sql(&baseline, &draft),
        vec![r#"ALTER TABLE "public"."users" ALTER COLUMN "email" TYPE text COLLATE "C";"#]
    );

    draft.table.column_mut(existing(2)).unwrap().data_type = "varchar(100)".into();
    draft.table.column_mut(existing(2)).unwrap().collation = None;
    assert_eq!(
        plan_sql(&baseline, &draft),
        vec![r#"ALTER TABLE "public"."users" ALTER COLUMN "email" TYPE varchar(100);"#]
    );
}

#[test]
fn concurrent_indexes_and_separate_validation_run_after_the_atomic_group() {
    let (baseline, mut draft) = setup();
    let check = draft.new_origin();
    draft.table.checks.push(CheckModel {
        origin: check,
        name: None,
        expression: SqlFragment::new("id > 0"),
        enforced: true,
    });
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
    let options = PlanOptions {
        concurrent_indexes: vec![index],
        validate_separately: true,
        ..PlanOptions::default()
    };
    let preview = review(&baseline, &draft, &options).unwrap();
    assert_eq!(
        sql(&preview),
        vec![
            r#"ALTER TABLE "public"."users" ADD CONSTRAINT "users_check" CHECK (id > 0) NOT VALID;"#,
            r#"ALTER TABLE "public"."users" VALIDATE CONSTRAINT "users_check";"#,
            r#"CREATE INDEX CONCURRENTLY "users_email_idx" ON "public"."users" USING "btree" ("email");"#,
        ]
    );
    assert_eq!(
        preview.groups,
        vec![
            PreviewGroup::Atomic {
                statements: vec![0]
            },
            PreviewGroup::Standalone { statement: 1 },
            PreviewGroup::Standalone { statement: 2 },
        ]
    );
    assert!(has_note(&preview, NoteKind::NonAtomic, "stay applied"));
    assert!(has_note(&preview, NoteKind::Info, "NOT VALID"));
    assert_eq!(preview.options, options);
}

#[test]
fn a_recreated_index_cannot_be_built_concurrently() {
    let (baseline, mut draft) = setup();
    draft.table.indexes[0].members[0].descending = true;
    let options = PlanOptions {
        concurrent_indexes: vec![existing(500)],
        ..PlanOptions::default()
    };
    match review(&baseline, &draft, &options) {
        Err(PlanError::Unsupported { reason }) => {
            assert!(
                reason.starts_with("Index users_name_idx is re-created"),
                "{reason}"
            );
            assert!(reason.contains("only to new indexes"), "{reason}");
        }
        other => panic!("expected Unsupported, got {other:?}"),
    }
    // Without the option the rebuild stays inside the atomic group.
    assert_eq!(
        plan_sql(&baseline, &draft),
        vec![
            r#"DROP INDEX "public"."users_name_idx" RESTRICT;"#,
            r#"CREATE INDEX "users_name_idx" ON "public"."users" USING "btree" ("name" DESC);"#,
        ]
    );
}

fn flagged(id: u64, bits: u64) -> Origin {
    Origin::Existing(BaselineId(id | bits))
}

#[test]
fn objects_with_attributes_the_model_lacks_are_never_recreated() {
    const COMMENT: u64 = 1 << 34;
    const TABLESPACE: u64 = 1 << 33;
    let (mut baseline, _) = setup();
    baseline.table.indexes[0].origin = flagged(500, COMMENT | TABLESPACE);
    baseline.table.foreign_keys[0].origin = flagged(300, COMMENT);
    assert_eq!(
        preserved_attributes(baseline.table.indexes[0].origin),
        vec!["a tablespace", "a comment"]
    );
    assert!(preserved_attributes(existing(500)).is_empty());

    let mut draft = SchemaDraft::from_baseline(&baseline);
    draft.table.indexes[0].members[0].descending = true;
    assert_eq!(
        unsupported(&baseline, &draft),
        "Index users_name_idx has a tablespace and a comment, which re-creating it would lose; undo the change to it or make the change outside the editor"
    );

    // Retyping the column re-creates its foreign key, which has a comment.
    let mut draft = SchemaDraft::from_baseline(&baseline);
    draft.table.column_mut(existing(4)).unwrap().data_type = "integer".into();
    assert!(
        unsupported(&baseline, &draft).starts_with("Foreign key users_org_id_fkey has a comment")
    );

    // Renames, drops and unrelated edits are still planned.
    let mut draft = SchemaDraft::from_baseline(&baseline);
    draft.table.indexes[0].name = "users_name_idx2".into();
    draft.table.foreign_keys.clear();
    draft.add_column("nick", "text");
    assert_eq!(
        plan_sql(&baseline, &draft),
        vec![
            r#"ALTER TABLE "public"."users" DROP CONSTRAINT "users_org_id_fkey" RESTRICT;"#,
            r#"ALTER INDEX "public"."users_name_idx" RENAME TO "users_name_idx2";"#,
            r#"ALTER TABLE "public"."users" ADD COLUMN "nick" text;"#,
        ]
    );
}

fn self_referencing() -> SchemaBaseline {
    let (mut baseline, _) = setup();
    let key = &mut baseline.table.foreign_keys[0];
    key.name = Some("users_parent_fkey".into());
    key.referenced_table = "users".into();
    key.referenced_columns = vec!["id".into()];
    baseline
}

#[test]
fn a_recreated_self_reference_follows_table_and_column_renames() {
    let baseline = self_referencing();
    let mut draft = SchemaDraft::from_baseline(&baseline);
    draft.table.name = "people".into();
    draft.table.column_mut(existing(1)).unwrap().name = "ident".into();
    draft.table.column_mut(existing(4)).unwrap().data_type = "integer".into();
    let statements = plan_sql(&baseline, &draft);
    let added = statements.last().unwrap();
    assert!(
        added.contains(r#"FOREIGN KEY ("org_id") REFERENCES "public"."people" ("ident")"#),
        "{added}"
    );
}

#[test]
fn referenced_targets_map_only_unedited_self_references() {
    let baseline = self_referencing();
    let mut draft = SchemaDraft::from_baseline(&baseline);
    draft.table.name = "people".into();
    draft.table.column_mut(existing(1)).unwrap().name = "ident".into();
    let (old, new) = (&baseline.table, &draft.table);
    assert_eq!(
        referenced_target(old, new, &new.foreign_keys[0]),
        ("people".to_owned(), vec!["ident".to_owned()])
    );

    // An edited reference is already written against the draft.
    let mut edited = new.foreign_keys[0].clone();
    edited.referenced_columns = vec!["email".into()];
    assert_eq!(
        referenced_target(old, new, &edited),
        ("people".to_owned(), vec!["email".to_owned()])
    );

    // Another table, or this name in another schema, is left alone.
    let mut other = new.foreign_keys[0].clone();
    other.referenced_table = "orgs".into();
    assert_eq!(
        referenced_target(old, new, &other),
        ("orgs".to_owned(), vec!["id".to_owned()])
    );
    other.referenced_table = "users".into();
    other.referenced_schema = Some("archive".into());
    assert_eq!(
        referenced_target(old, new, &other),
        ("users".to_owned(), vec!["id".to_owned()])
    );
}

#[test]
fn an_unnamed_foreign_key_validated_separately_gets_a_derived_name() {
    let (baseline, mut draft) = setup();
    let origin = draft.new_origin();
    draft.table.foreign_keys.push(ForeignKeyModel {
        origin,
        name: None,
        columns: vec![existing(4)],
        referenced_schema: Some("billing".into()),
        referenced_table: "teams".into(),
        referenced_columns: vec!["id".into()],
        on_update: ReferentialAction::NoAction,
        on_delete: ReferentialAction::SetNull,
        deferrable: Deferrable::InitiallyDeferred,
    });
    let options = PlanOptions {
        validate_separately: true,
        ..PlanOptions::default()
    };
    let preview = review(&baseline, &draft, &options).unwrap();
    assert_eq!(
        sql(&preview),
        vec![
            r#"ALTER TABLE "public"."users" ADD CONSTRAINT "users_org_id_fkey1" FOREIGN KEY ("org_id") REFERENCES "billing"."teams" ("id") ON UPDATE NO ACTION ON DELETE SET NULL DEFERRABLE INITIALLY DEFERRED NOT VALID;"#,
            r#"ALTER TABLE "public"."users" VALIDATE CONSTRAINT "users_org_id_fkey1";"#,
        ]
    );
}

#[test]
fn refuses_what_postgres_cannot_do() {
    let (baseline, mut draft) = setup();
    assert!(draft.move_column(existing(4), 0));
    assert_eq!(
        unsupported(&baseline, &draft),
        "Column reorder is not supported on PostgreSQL"
    );

    let (baseline, mut draft) = setup();
    draft.table.column_mut(existing(1)).unwrap().nullable = true;
    assert_eq!(
        unsupported(&baseline, &draft),
        "Column id is in the primary key and must stay NOT NULL"
    );

    let (baseline, mut draft) = setup();
    draft.add_column("seq", "serial");
    assert!(unsupported(&baseline, &draft).contains("identity column"));

    let (baseline, mut draft) = setup();
    draft.add_column("x", "text collate \"C\"");
    assert_eq!(
        unsupported(&baseline, &draft),
        "Column data type contains a column option"
    );

    let (baseline, mut draft) = setup();
    draft.table.column_mut(existing(3)).unwrap().extra.invisible = true;
    assert!(unsupported(&baseline, &draft).contains("MySQL-only"));
}

#[test]
fn a_check_on_a_dropped_column_is_refused_before_anything_runs() {
    let (baseline, mut draft) = setup();
    assert!(draft.remove_column(existing(2)));
    assert_eq!(
        unsupported(&baseline, &draft),
        "Check users_email_check uses column email, which is being dropped; remove or edit the check first"
    );
}

#[test]
fn an_index_expression_or_generated_column_on_a_dropped_column_is_refused() {
    let (baseline, mut draft) = setup();
    let origin = draft.new_origin();
    draft.table.indexes.push(IndexModel {
        origin,
        name: "users_lower_name_idx".into(),
        unique: false,
        method: None,
        members: vec![IndexMember {
            target: IndexTarget::Expression(SqlFragment::new("lower(name)")),
            descending: false,
            prefix_length: None,
            collation: None,
        }],
        include: Vec::new(),
        predicate: None,
    });
    assert!(draft.remove_column(existing(3)));
    assert_eq!(
        unsupported(&baseline, &draft),
        "Index users_lower_name_idx uses column name in an expression or predicate, which is being dropped; remove or edit the index first"
    );

    let (baseline, mut draft) = setup();
    let origin = draft.new_origin();
    draft.table.indexes.push(IndexModel {
        origin,
        name: "users_named_idx".into(),
        unique: false,
        method: None,
        members: vec![IndexMember::column(existing(1))],
        include: Vec::new(),
        predicate: Some(SqlFragment::new("name IS NOT NULL")),
    });
    assert!(draft.remove_column(existing(3)));
    assert!(unsupported(&baseline, &draft).starts_with("Index users_named_idx uses column name"));

    let (baseline, mut draft) = setup();
    let upper = draft.add_column("upper_name", "text");
    draft.table.column_mut(upper).unwrap().generated = Some(Generated {
        expression: SqlFragment::new("upper(name)"),
        stored: true,
    });
    assert!(draft.remove_column(existing(3)));
    assert_eq!(
        unsupported(&baseline, &draft),
        "Generated column upper_name uses column name, which is being dropped; remove or edit the generated column first"
    );
}

#[test]
fn notes_cover_dependents_and_large_tables() {
    let (mut baseline, _) = setup();
    baseline.estimated_rows = Some(5_000_000);
    baseline.inbound.push(InboundReference {
        schema: "public".into(),
        table: "posts".into(),
        name: "posts_author_fkey".into(),
        columns: vec!["id".into()],
    });
    let mut draft = SchemaDraft::from_baseline(&baseline);
    draft.table.column_mut(existing(3)).unwrap().data_type = "integer".into();
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
    let check = draft.new_origin();
    draft.table.checks.push(CheckModel {
        origin: check,
        name: Some("users_id_positive".into()),
        expression: SqlFragment::new("id > 0"),
        enforced: true,
    });
    let preview = review(&baseline, &draft, &PlanOptions::default()).unwrap();
    assert!(has_note(
        &preview,
        NoteKind::Dependents,
        "public.posts.posts_author_fkey"
    ));
    assert!(has_note(&preview, NoteKind::LargeTable, "5000000 rows"));
    assert!(has_note(&preview, NoteKind::LargeTable, "NOT VALID"));
    assert!(has_note(&preview, NoteKind::LargeTable, "CONCURRENTLY"));
    assert!(has_note(&preview, NoteKind::LargeTable, "rewrite"));
}

#[test]
fn planning_is_deterministic() {
    let (baseline, mut draft) = setup();
    draft.table.column_mut(existing(2)).unwrap().name = "name".into();
    draft.table.column_mut(existing(3)).unwrap().name = "email".into();
    draft.add_column("nick", "text");
    draft.table.uniques[0].name = Some("users_email_uq".into());
    draft.table.comment = Some("People".into());
    let index = draft.new_origin();
    draft.table.indexes.push(IndexModel {
        origin: index,
        name: "users_nick_idx".into(),
        unique: true,
        method: None,
        members: vec![IndexMember::column(Origin::New(DraftId(1)))],
        include: Vec::new(),
        predicate: Some(SqlFragment::new("nick IS NOT NULL")),
    });
    let options = PlanOptions {
        concurrent_indexes: vec![index],
        validate_separately: true,
        ..PlanOptions::default()
    };
    let first = review(&baseline, &draft, &options).unwrap();
    let second = review(&baseline, &draft, &options).unwrap();
    assert_eq!(first, second);
    let built = build(&baseline, &draft, &options).unwrap();
    assert_eq!(built.statements, first.statements);
    assert_eq!(built.groups(), first.groups);
    assert_eq!(built.deferred_indexes, vec![index]);
}

#[test]
fn binary_coercible_type_changes() {
    for (before, after) in [
        ("character varying(100)", "varchar(200)"),
        ("character varying(100)", "character varying"),
        ("character varying(100)", "text"),
        ("text", "varchar"),
        ("numeric(10,2)", "numeric(12, 2)"),
        ("numeric(10,2)", "numeric"),
        ("NUMERIC(8)", "decimal(9)"),
    ] {
        assert!(binary_coercible(before, after), "{before} → {after}");
    }
    for (before, after) in [
        ("character varying(100)", "varchar(50)"),
        ("text", "varchar(10)"),
        ("numeric(10,2)", "numeric(12,3)"),
        ("integer", "bigint"),
        ("text", "integer"),
    ] {
        assert!(!binary_coercible(before, after), "{before} → {after}");
    }
}

#[test]
fn resolves_rename_chains_and_cycles() {
    let names = |list: &[&str]| list.iter().map(|s| s.to_string()).collect::<Vec<_>>();
    let rename = |key: u8, from: &str, to: &str| (key, from.to_string(), to.to_string());
    assert_eq!(
        resolve_renames(
            names(&["a", "b", "c"]),
            vec![rename(1, "a", "b"), rename(2, "b", "d")]
        ),
        vec![rename(2, "b", "d"), rename(1, "a", "b")]
    );
    assert_eq!(
        resolve_renames(
            names(&["a", "b"]),
            vec![rename(1, "a", "b"), rename(2, "b", "a")]
        ),
        vec![
            rename(1, "a", "a_dbunk_tmp"),
            rename(2, "b", "a"),
            rename(1, "a_dbunk_tmp", "b"),
        ]
    );
    // A target held by an object that is not renamed is left to the server.
    assert_eq!(
        resolve_renames(names(&["a", "x"]), vec![rename(1, "a", "x")]),
        vec![rename(1, "a", "x")]
    );
}

#[test]
fn helpers() {
    assert_eq!(server_major("16.4"), 16);
    assert_eq!(server_major("13.2 (Debian 13.2-1)"), 13);
    assert_eq!(server_major("17beta1"), 17);
    assert_eq!(server_major(""), 0);
    assert!(references_identifier("length(email) > 3", "email"));
    assert!(references_identifier("\"Email\" <> ''", "Email"));
    assert!(!references_identifier("\"Email\" <> ''", "email"));
    assert!(!references_identifier("'email' <> ''", "email"));
}

#[test]
fn a_reviewed_using_expression_converts_a_retyped_column() {
    let (baseline, mut draft) = setup();
    // name: text → integer has no implicit cast.
    draft.table.column_mut(existing(3)).unwrap().data_type = "integer".into();
    let using = |column, text: &str| PlanOptions {
        type_conversions: vec![TypeConversion {
            column,
            using: SqlFragment::new(text),
        }],
        ..PlanOptions::default()
    };
    let preview = review(&baseline, &draft, &using(existing(3), "length(name)")).unwrap();
    assert!(
        sql(&preview).iter().any(|statement| statement
            == &r#"ALTER TABLE "public"."users" ALTER COLUMN "name" TYPE integer USING length(name);"#),
        "{:?}",
        sql(&preview)
    );
    assert!(preview.statements.iter().any(|s| s.destructive));

    // Only for an existing column whose type changes, once, as one
    // expression.
    for (options, expected) in [
        (using(existing(2), "email"), "whose type changes"),
        (using(draft_origin(&draft), "1"), "whose type changes"),
        (
            using(existing(3), "1; DROP TABLE users"),
            "USING for column name",
        ),
    ] {
        let error = review(&baseline, &draft, &options).unwrap_err();
        assert!(error.to_string().contains(expected), "{error}");
    }
    let mut twice = using(existing(3), "1");
    twice
        .type_conversions
        .push(twice.type_conversions[0].clone());
    let error = review(&baseline, &draft, &twice).unwrap_err();
    assert!(error.to_string().contains("more than one"), "{error}");
}

fn draft_origin(draft: &SchemaDraft) -> Origin {
    let mut draft = draft.clone();
    draft.add_column("fresh", "integer")
}

#[test]
fn other_engines_refuse_using() {
    for engine in [SchemaEngine::MySql, SchemaEngine::Sqlite] {
        let baseline = crate::schema_edit::fixtures::users(engine);
        let mut draft = SchemaDraft::from_baseline(&baseline);
        draft.table.column_mut(existing(3)).unwrap().data_type = "integer".into();
        let options = PlanOptions {
            type_conversions: vec![TypeConversion {
                column: existing(3),
                using: SqlFragment::new("0"),
            }],
            ..PlanOptions::default()
        };
        let error = plan_review(&baseline, &draft, &options).unwrap_err();
        assert!(error.to_string().contains("USING"), "{engine:?}: {error}");
    }
}
