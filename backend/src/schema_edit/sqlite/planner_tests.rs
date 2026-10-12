use super::*;
use crate::schema_edit::fixtures::{existing, users};
use crate::schema_edit::{
    plan_review, CheckModel, IndexMember, PlanOptions, SchemaEditPreview, SchemaEngine,
};

type DraftEdit = Box<dyn Fn(&mut SchemaDraft)>;
type ColumnEdit = Box<dyn Fn(&mut ColumnModel)>;

fn draft_edit(change: impl Fn(&mut SchemaDraft) + 'static) -> DraftEdit {
    Box::new(change)
}

fn column_edit(change: impl Fn(&mut ColumnModel) + 'static) -> ColumnEdit {
    Box::new(change)
}

fn baseline() -> SchemaBaseline {
    users(SchemaEngine::Sqlite)
}

fn review(baseline: &SchemaBaseline, draft: &SchemaDraft) -> Result<SchemaEditPreview, PlanError> {
    plan_review(baseline, draft, &PlanOptions::default())
}

fn sql(preview: &SchemaEditPreview) -> Vec<&str> {
    preview.statements.iter().map(|s| s.sql.as_str()).collect()
}

fn rebuilt(preview: &SchemaEditPreview) -> bool {
    is_rebuild(&preview.notes)
}

fn one_atomic_group(preview: &SchemaEditPreview) {
    assert_eq!(
        preview.groups,
        vec![PreviewGroup::Atomic {
            statements: (0..preview.statements.len()).collect()
        }]
    );
}

fn unsupported_reason(result: Result<SchemaEditPreview, PlanError>) -> String {
    match result {
        Err(PlanError::Unsupported { reason }) => reason,
        other => panic!("expected Unsupported, got {other:?}"),
    }
}

#[test]
fn add_column_with_a_constant_default_is_one_alter() {
    let base = baseline();
    let mut draft = SchemaDraft::from_baseline(&base);
    let origin = draft.add_column("nickname", "TEXT");
    draft.table.column_mut(origin).unwrap().default = DefaultValue::Literal("it's".into());
    let preview = review(&base, &draft).unwrap();
    assert_eq!(
        sql(&preview),
        vec!["ALTER TABLE \"main\".\"users\" ADD COLUMN \"nickname\" TEXT DEFAULT 'it''s'"]
    );
    assert!(!preview.statements[0].destructive);
    assert!(!rebuilt(&preview));
    one_atomic_group(&preview);

    let mut draft = SchemaDraft::from_baseline(&base);
    let origin = draft.add_column("visits", "INTEGER");
    let column = draft.table.column_mut(origin).unwrap();
    column.nullable = false;
    column.default = DefaultValue::Expression(SqlFragment::new("-1"));
    assert_eq!(
        sql(&review(&base, &draft).unwrap()),
        vec!["ALTER TABLE \"main\".\"users\" ADD COLUMN \"visits\" INTEGER NOT NULL DEFAULT -1"]
    );
}

#[test]
fn add_column_outside_alter_restrictions_rebuilds() {
    let base = baseline();
    let cases: Vec<ColumnEdit> = vec![
        // NOT NULL without a default.
        column_edit(|column: &mut ColumnModel| column.nullable = false),
        // A non-constant default.
        column_edit(|column: &mut ColumnModel| {
            column.default = DefaultValue::Expression(SqlFragment::new("CURRENT_TIMESTAMP"))
        }),
        column_edit(|column: &mut ColumnModel| {
            column.default = DefaultValue::Expression(SqlFragment::new("abs(-1)"))
        }),
        // NOT NULL with a NULL default.
        column_edit(|column: &mut ColumnModel| {
            column.nullable = false;
            column.default = DefaultValue::Expression(SqlFragment::new("NULL"));
        }),
        // A STORED generated column.
        column_edit(|column: &mut ColumnModel| {
            column.generated = Some(crate::schema_edit::Generated {
                expression: SqlFragment::new("upper(name)"),
                stored: true,
            })
        }),
    ];
    for (index, edit) in cases.iter().enumerate() {
        let mut draft = SchemaDraft::from_baseline(&base);
        let origin = draft.add_column("extra", "TEXT");
        edit(draft.table.column_mut(origin).unwrap());
        let preview = review(&base, &draft).unwrap();
        assert!(rebuilt(&preview), "case {index}");
        one_atomic_group(&preview);
        assert!(preview.statements.iter().all(|s| s.destructive));
    }
}

#[test]
fn renames_use_alter_and_track_the_table_name() {
    let base = baseline();
    let mut draft = SchemaDraft::from_baseline(&base);
    draft.table.column_mut(existing(3)).unwrap().name = "full_name".into();
    draft.table.name = "people".into();
    let preview = review(&base, &draft).unwrap();
    assert_eq!(
        sql(&preview),
        vec![
            "ALTER TABLE \"main\".\"users\" RENAME COLUMN \"name\" TO \"full_name\"",
            "ALTER TABLE \"main\".\"users\" RENAME TO \"people\"",
        ]
    );
    assert!(!rebuilt(&preview));
}

#[test]
fn a_case_only_table_rename_goes_through_a_temporary_name() {
    let base = baseline();
    let mut draft = SchemaDraft::from_baseline(&base);
    draft.table.name = "Users".into();
    assert_eq!(
        sql(&review(&base, &draft).unwrap()),
        vec![
            "ALTER TABLE \"main\".\"users\" RENAME TO \"_dbunk_rename_Users\"",
            "ALTER TABLE \"main\".\"_dbunk_rename_Users\" RENAME TO \"Users\"",
        ]
    );
}

#[test]
fn drop_column_drops_its_index_first() {
    let base = baseline();
    let mut draft = SchemaDraft::from_baseline(&base);
    assert!(draft.remove_column(existing(3)));
    let preview = review(&base, &draft).unwrap();
    assert_eq!(
        sql(&preview),
        vec![
            "DROP INDEX \"main\".\"users_name_idx\"",
            "ALTER TABLE \"main\".\"users\" DROP COLUMN \"name\"",
        ]
    );
    assert!(preview.statements.iter().all(|s| s.destructive));
    assert!(preview.notes.iter().any(|n| n.kind == NoteKind::Rewrite));
}

#[test]
fn drop_column_mentioned_anywhere_rebuilds() {
    let mut base = baseline();
    base.table.checks.push(CheckModel {
        origin: existing(401),
        name: Some("name_set".into()),
        expression: SqlFragment::new("\"Name\" <> ''"),
        enforced: true,
    });
    // The check stays; only the column goes (the server would refuse the
    // CREATE, but planning must already avoid DROP COLUMN).
    let mut draft = SchemaDraft::from_baseline(&base);
    draft.remove_column(existing(3));
    assert!(rebuilt(&review(&base, &draft).unwrap()));

    // A view naming the column is enough.
    let mut base = baseline();
    base.dependents.push(Dependent {
        kind: VIEW.into(),
        schema: "main".into(),
        name: "named".into(),
        definition: Some("CREATE VIEW named AS SELECT name FROM users".into()),
    });
    let mut draft = SchemaDraft::from_baseline(&base);
    draft.remove_column(existing(3));
    assert!(rebuilt(&review(&base, &draft).unwrap()));
}

#[test]
fn old_servers_rebuild_instead_of_rename_or_drop_column() {
    let mut base = baseline();
    base.server.features.rename_column = false;
    base.server.features.drop_column = false;
    let mut draft = SchemaDraft::from_baseline(&base);
    draft.table.column_mut(existing(3)).unwrap().name = "full_name".into();
    assert!(rebuilt(&review(&base, &draft).unwrap()));
    let mut draft = SchemaDraft::from_baseline(&base);
    draft.remove_column(existing(3));
    assert!(rebuilt(&review(&base, &draft).unwrap()));
}

#[test]
fn index_changes_stay_on_the_fast_path() {
    let base = baseline();
    let mut draft = SchemaDraft::from_baseline(&base);
    draft.table.indexes[0].name = "users_by_name".into();
    assert_eq!(
        sql(&review(&base, &draft).unwrap()),
        vec![
            "DROP INDEX \"main\".\"users_name_idx\"",
            "CREATE INDEX \"main\".\"users_by_name\" ON \"users\" (\"name\")",
        ]
    );

    let mut draft = SchemaDraft::from_baseline(&base);
    let column = draft.add_column("nick", "TEXT");
    let origin = draft.new_origin();
    draft.table.indexes.push(IndexModel {
        origin,
        name: "users_nick".into(),
        unique: true,
        method: None,
        members: vec![
            IndexMember::column(column),
            IndexMember {
                target: IndexTarget::Expression(SqlFragment::new("lower(email)")),
                descending: true,
                prefix_length: None,
                collation: Some("NOCASE".into()),
            },
        ],
        include: Vec::new(),
        predicate: Some(SqlFragment::new("nick IS NOT NULL")),
    });
    assert_eq!(
        sql(&review(&base, &draft).unwrap()),
        vec![
            "ALTER TABLE \"main\".\"users\" ADD COLUMN \"nick\" TEXT",
            "CREATE UNIQUE INDEX \"main\".\"users_nick\" ON \"users\" (\"nick\", lower(email) \
             COLLATE NOCASE DESC) WHERE nick IS NOT NULL",
        ]
    );
}

#[test]
fn a_type_change_rebuilds_the_whole_table() {
    let base = baseline();
    let mut draft = SchemaDraft::from_baseline(&base);
    draft.table.column_mut(existing(3)).unwrap().data_type = "varchar(50)".into();
    let preview = review(&base, &draft).unwrap();
    assert_eq!(
        sql(&preview),
        vec![
            "CREATE TABLE \"main\".\"_dbunk_rebuild_users\" (\n  \
             \"id\" bigint CONSTRAINT \"users_pkey\" PRIMARY KEY NOT NULL,\n  \
             \"email\" varchar(100) NOT NULL,\n  \
             \"name\" varchar(50),\n  \
             \"org_id\" bigint,\n  \
             CONSTRAINT \"users_email_key\" UNIQUE (\"email\"),\n  \
             CONSTRAINT \"users_email_check\" CHECK (length(email) > 3),\n  \
             CONSTRAINT \"users_org_id_fkey\" FOREIGN KEY (\"org_id\") REFERENCES \"orgs\" \
             (\"id\") ON DELETE CASCADE\n)",
            "INSERT INTO \"main\".\"_dbunk_rebuild_users\" (\"id\", \"email\", \"name\", \
             \"org_id\", rowid) SELECT \"id\", \"email\", \"name\", \"org_id\", rowid FROM \
             \"main\".\"users\"",
            "DROP TABLE \"main\".\"users\"",
            "ALTER TABLE \"main\".\"_dbunk_rebuild_users\" RENAME TO \"users\"",
            "CREATE INDEX \"main\".\"users_name_idx\" ON \"users\" (\"name\")",
        ]
    );
    assert!(preview.statements.iter().all(|s| s.destructive));
    one_atomic_group(&preview);
    assert_eq!(preview.notes.len(), 1);
    assert_eq!(preview.notes[0].kind, NoteKind::Rebuild);
    assert!(preview.notes[0].text.contains("Rowids are preserved"));
    // Pure and deterministic.
    assert_eq!(review(&base, &draft).unwrap(), preview);
}

#[test]
fn every_structural_change_rebuilds() {
    let base = baseline();
    let edits: Vec<DraftEdit> = vec![
        draft_edit(|d: &mut SchemaDraft| d.table.column_mut(existing(3)).unwrap().nullable = false),
        draft_edit(|d: &mut SchemaDraft| {
            d.table.column_mut(existing(3)).unwrap().default = DefaultValue::Literal("anon".into())
        }),
        draft_edit(|d: &mut SchemaDraft| {
            d.table.column_mut(existing(3)).unwrap().collation = Some("NOCASE".into())
        }),
        draft_edit(|d: &mut SchemaDraft| d.table.checks.clear()),
        draft_edit(|d: &mut SchemaDraft| d.table.uniques.clear()),
        draft_edit(|d: &mut SchemaDraft| {
            d.table.foreign_keys[0].on_delete = ReferentialAction::SetNull
        }),
        draft_edit(|d: &mut SchemaDraft| {
            d.table.primary_key.as_mut().unwrap().name = Some("pk".into())
        }),
        draft_edit(|d: &mut SchemaDraft| d.table.options.strict = true),
        draft_edit(|d: &mut SchemaDraft| d.table.options.without_rowid = true),
        draft_edit(|d: &mut SchemaDraft| {
            d.move_column(existing(4), 0);
        }),
    ];
    for (index, edit) in edits.iter().enumerate() {
        let mut draft = SchemaDraft::from_baseline(&base);
        edit(&mut draft);
        let preview = review(&base, &draft).unwrap_or_else(|e| panic!("case {index}: {e:?}"));
        assert!(rebuilt(&preview), "case {index}");
        one_atomic_group(&preview);
    }
}

#[test]
fn reorder_renders_the_draft_order() {
    let base = baseline();
    let mut draft = SchemaDraft::from_baseline(&base);
    draft.move_column(existing(4), 0);
    let preview = review(&base, &draft).unwrap();
    assert!(preview.statements[0]
        .sql
        .starts_with("CREATE TABLE \"main\".\"_dbunk_rebuild_users\" (\n  \"org_id\" bigint,\n"));
}

#[test]
fn a_rebuild_follows_renames_in_untouched_expressions() {
    let base = baseline();
    let mut draft = SchemaDraft::from_baseline(&base);
    draft.table.column_mut(existing(2)).unwrap().name = "mail".into();
    draft.table.column_mut(existing(3)).unwrap().data_type = "varchar(50)".into();
    let preview = review(&base, &draft).unwrap();
    let create = &preview.statements[0].sql;
    assert!(create.contains("CONSTRAINT \"users_email_check\" CHECK (length(\"mail\") > 3)"));
    assert!(create.contains("UNIQUE (\"mail\")"));
    assert!(preview.statements[1]
        .sql
        .contains("(\"id\", \"mail\", \"name\", \"org_id\", rowid) SELECT \"id\", \"email\""));

    // An edited expression is taken as written.
    draft.table.checks[0].expression = SqlFragment::new("length(mail) > 5");
    let preview = review(&base, &draft).unwrap();
    assert!(preview.statements[0]
        .sql
        .contains("CHECK (length(mail) > 5)"));
}

#[test]
fn expected_table_follows_self_references_through_renames() {
    let mut base = baseline();
    base.table.foreign_keys.push(ForeignKeyModel {
        origin: existing(301),
        name: None,
        columns: vec![existing(4)],
        referenced_schema: None,
        referenced_table: "users".into(),
        referenced_columns: vec!["id".into()],
        on_update: ReferentialAction::NoAction,
        on_delete: ReferentialAction::NoAction,
        deferrable: Deferrable::Never,
    });
    let mut draft = SchemaDraft::from_baseline(&base);
    draft.table.name = "people".into();
    draft.table.column_mut(existing(1)).unwrap().name = "uid".into();
    let expected = expected_table(&base, &draft);
    let key = expected.foreign_key(existing(301)).unwrap();
    assert_eq!(key.referenced_table, "people");
    assert_eq!(key.referenced_columns, vec!["uid".to_owned()]);
    let other = expected.foreign_key(existing(300)).unwrap();
    assert_eq!(other.referenced_table, "orgs");
}

#[test]
fn autoincrement_keeps_its_counter_and_rowids() {
    let mut base = baseline();
    let id = base.table.column_mut(existing(1)).unwrap();
    id.data_type = "INTEGER".into();
    id.identity = Identity::AutoIncrement;
    let mut draft = SchemaDraft::from_baseline(&base);
    draft.table.column_mut(existing(3)).unwrap().data_type = "varchar(50)".into();
    let preview = review(&base, &draft).unwrap();
    let statements = sql(&preview);
    assert!(statements[0]
        .contains("\"id\" INTEGER CONSTRAINT \"users_pkey\" PRIMARY KEY AUTOINCREMENT NOT NULL"));
    // The alias column carries the rowid; no separate rowid copy.
    assert!(!statements[1].contains("rowid"));
    assert_eq!(
        statements[2],
        "INSERT INTO \"main\".sqlite_sequence (name, seq) SELECT '_dbunk_rebuild_users', 0 \
         WHERE NOT EXISTS (SELECT 1 FROM \"main\".sqlite_sequence WHERE name = \
         '_dbunk_rebuild_users')"
    );
    assert_eq!(
        statements[3],
        "UPDATE \"main\".sqlite_sequence SET seq = max(seq, coalesce((SELECT seq FROM \
         \"main\".sqlite_sequence WHERE name = 'users'), 0)) WHERE name = '_dbunk_rebuild_users'"
    );
    assert_eq!(statements[4], "DROP TABLE \"main\".\"users\"");
    assert!(preview.notes[0].text.contains("Rowids are preserved"));
}

fn with_trigger(definition: &str) -> SchemaBaseline {
    let mut base = baseline();
    base.dependents.push(Dependent {
        kind: TRIGGER.into(),
        schema: "main".into(),
        name: "users_touch".into(),
        definition: Some(definition.into()),
    });
    base
}

#[test]
fn a_rebuild_recreates_triggers_qualified() {
    let base = with_trigger("CREATE TRIGGER users_touch AFTER UPDATE ON users BEGIN SELECT 1; END");
    let mut draft = SchemaDraft::from_baseline(&base);
    draft.table.column_mut(existing(3)).unwrap().data_type = "varchar(50)".into();
    let preview = review(&base, &draft).unwrap();
    assert_eq!(
        preview.statements.last().unwrap().sql,
        "CREATE TRIGGER \"main\".users_touch AFTER UPDATE ON users BEGIN SELECT 1; END"
    );
    assert!(preview.notes.iter().any(|n| n.kind == NoteKind::Dependents));
}

#[test]
fn a_rebuild_refuses_trigger_text_it_cannot_follow() {
    let base = with_trigger(
        "CREATE TRIGGER users_touch AFTER UPDATE ON users BEGIN UPDATE users SET name = \
         upper(name); END",
    );
    // A rename plus a rebuild would leave the trigger naming old names.
    let mut draft = SchemaDraft::from_baseline(&base);
    draft.table.column_mut(existing(2)).unwrap().name = "mail".into();
    draft.table.column_mut(existing(3)).unwrap().data_type = "varchar(50)".into();
    assert!(unsupported_reason(review(&base, &draft)).contains("triggers"));
    // A rename alone stays on the fast path, where SQLite rewrites triggers.
    let mut draft = SchemaDraft::from_baseline(&base);
    draft.table.column_mut(existing(2)).unwrap().name = "mail".into();
    assert!(!rebuilt(&review(&base, &draft).unwrap()));
    // Dropping a column the trigger uses.
    let mut draft = SchemaDraft::from_baseline(&base);
    draft.remove_column(existing(3));
    assert!(unsupported_reason(review(&base, &draft)).contains("users_touch"));
}

#[test]
fn attributes_sqlite_lacks_are_unsupported() {
    let base = baseline();
    let edits: Vec<DraftEdit> = vec![
        draft_edit(|d: &mut SchemaDraft| d.table.comment = Some("people".into())),
        draft_edit(|d: &mut SchemaDraft| {
            d.table.column_mut(existing(3)).unwrap().comment = Some("x".into())
        }),
        draft_edit(|d: &mut SchemaDraft| d.table.options.engine = Some("InnoDB".into())),
        draft_edit(|d: &mut SchemaDraft| {
            d.table.column_mut(existing(3)).unwrap().extra.invisible = true
        }),
        draft_edit(|d: &mut SchemaDraft| d.table.checks[0].enforced = false),
        draft_edit(|d: &mut SchemaDraft| d.table.indexes[0].method = Some("hash".into())),
        draft_edit(|d: &mut SchemaDraft| {
            d.table.foreign_keys[0].referenced_schema = Some("aux".into())
        }),
        draft_edit(|d: &mut SchemaDraft| {
            d.table.column_mut(existing(3)).unwrap().identity = Identity::AutoIncrement
        }),
        draft_edit(|d: &mut SchemaDraft| {
            d.table.primary_key = None;
            d.table.options.without_rowid = true;
        }),
        draft_edit(|d: &mut SchemaDraft| d.table.name = "sqlite_mine".into()),
    ];
    for (index, edit) in edits.iter().enumerate() {
        let mut draft = SchemaDraft::from_baseline(&base);
        edit(&mut draft);
        match review(&base, &draft) {
            Err(PlanError::Unsupported { .. }) => {}
            other => panic!("case {index}: expected Unsupported, got {other:?}"),
        }
    }
}

#[test]
fn defaults_render_bare_only_when_sqlite_accepts_them() {
    let render = |value: DefaultValue| render_default(&value);
    assert_eq!(render(DefaultValue::None), None);
    assert_eq!(
        render(DefaultValue::Literal("a'b".into())).as_deref(),
        Some("'a''b'")
    );
    for bare in [
        "42",
        "-1.5",
        "'x'",
        "x'00'",
        "NULL",
        "CURRENT_TIMESTAMP",
        "TRUE",
    ] {
        assert_eq!(
            render(DefaultValue::Expression(SqlFragment::new(bare))).as_deref(),
            Some(bare)
        );
    }
    assert_eq!(
        render(DefaultValue::Expression(SqlFragment::new("1 + 1"))).as_deref(),
        Some("(1 + 1)")
    );
    assert_eq!(
        render(DefaultValue::Expression(SqlFragment::new("1 -- one"))).as_deref(),
        Some("(1 -- one\n)")
    );
}

#[test]
fn a_rebuild_with_a_rename_is_refused_when_other_tables_refer_to_it() {
    let mut baseline = users(SchemaEngine::Sqlite);
    baseline.inbound.push(crate::schema_edit::InboundReference {
        schema: "main".into(),
        table: "sessions".into(),
        name: "fk0".into(),
        columns: vec!["id".into()],
    });
    let mut draft = SchemaDraft::from_baseline(&baseline);
    draft.table.name = "people".into();
    draft.table.column_mut(existing(3)).unwrap().nullable = false;
    let error = plan_review(&baseline, &draft, &PlanOptions::default()).unwrap_err();
    assert!(error.to_string().contains("refer to this table"), "{error}");
    // The rename alone takes the fast path, where SQLite rewrites references.
    let mut draft = SchemaDraft::from_baseline(&baseline);
    draft.table.name = "people".into();
    assert!(plan_review(&baseline, &draft, &PlanOptions::default()).is_ok());
}

fn referenced_by_sessions(columns: &[&str]) -> SchemaBaseline {
    let mut baseline = users(SchemaEngine::Sqlite);
    baseline.inbound.push(crate::schema_edit::InboundReference {
        schema: "main".into(),
        table: "sessions".into(),
        name: "fk0".into(),
        columns: columns.iter().map(|c| (*c).into()).collect(),
    });
    baseline
}

#[test]
fn the_inbound_note_names_each_referencing_table_once() {
    let mut baseline = referenced_by_sessions(&["id"]);
    for (table, name) in [("audit", "fk0"), ("sessions", "fk1")] {
        baseline.inbound.push(crate::schema_edit::InboundReference {
            schema: "main".into(),
            table: table.into(),
            name: name.into(),
            columns: vec!["id".into()],
        });
    }
    let mut draft = SchemaDraft::from_baseline(&baseline);
    draft.add_column("extra", "TEXT");
    let preview = plan_review(&baseline, &draft, &PlanOptions::default()).unwrap();
    assert!(
        preview
            .notes
            .iter()
            .any(|note| note.text.contains("Foreign keys in sessions, audit reference")),
        "{:?}",
        preview.notes
    );
}

#[test]
fn dropping_a_column_another_table_references_is_refused() {
    let baseline = referenced_by_sessions(&["email"]);
    let mut draft = SchemaDraft::from_baseline(&baseline);
    draft.table.checks.clear();
    draft.remove_column(existing(2));
    let reason = unsupported_reason(review(&baseline, &draft));
    assert!(
        reason.contains("email") && reason.contains("sessions"),
        "{reason}"
    );
    // An unreferenced column still drops.
    let mut draft = SchemaDraft::from_baseline(&baseline);
    draft.remove_column(existing(4));
    assert!(review(&baseline, &draft).is_ok());
}

#[test]
fn dropping_the_key_a_reference_needs_is_refused() {
    let baseline = referenced_by_sessions(&["email"]);
    let mut draft = SchemaDraft::from_baseline(&baseline);
    draft.table.uniques.clear();
    let reason = unsupported_reason(review(&baseline, &draft));
    assert!(reason.contains("unique key"), "{reason}");
    // A unique index on the same column keeps the reference matched.
    let origin = draft.new_origin();
    draft.table.indexes.push(IndexModel {
        origin,
        name: "users_email_idx".into(),
        unique: true,
        method: None,
        members: vec![IndexMember::column(existing(2))],
        include: Vec::new(),
        predicate: None,
    });
    assert!(review(&baseline, &draft).is_ok());
}

#[test]
fn a_reference_to_the_implicit_primary_key_keeps_it() {
    let baseline = referenced_by_sessions(&["(primary key, from user_id)"]);
    let mut draft = SchemaDraft::from_baseline(&baseline);
    draft.table.primary_key = None;
    let reason = unsupported_reason(review(&baseline, &draft));
    assert!(reason.contains("primary key"), "{reason}");
}
