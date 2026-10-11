use super::*;
use crate::apply_flow::ApplyFlow;
use crate::data_model::{ConfirmStyle, TablePolicy};
use dbunk_lib::backend::schema_edit::{
    BaselineId, DriftToken, EngineNote, IndexMember, IndexTarget, NoteKind, PlanOptions,
    PreviewGroup, PreviewStatement, SchemaEditAttemptId, SchemaEditError, SchemaEditFailure,
    SchemaEditOutcome, SchemaEditPreview, SchemaEditReceipt, ServerFacts, ServerFeatures,
    ServerFlavor, TableModel, TableOptions, WorkspaceSchemaEdit, WorkspaceSchemaEditAttempt,
};
use dbunk_lib::backend::{
    DevelopmentConnection, DevelopmentConnectionOrganization, DevelopmentEngineConnection,
    DevelopmentEnvironment, DevelopmentMySqlConnection, DevelopmentSafeMode,
    DevelopmentSqliteConnection, WorkspaceApplyState,
};

// Origins follow the backend fixtures: columns 1.., the primary key 100,
// uniques 200+, foreign keys 300+, checks 400+, indexes 500+.
fn existing(id: u64) -> Origin {
    Origin::Existing(BaselineId(id))
}

fn col(id: u64, name: &str, data_type: &str) -> ColumnModel {
    ColumnModel::new(existing(id), name, data_type)
}

fn features(engine: SchemaEngine) -> ServerFeatures {
    ServerFeatures {
        rename_column: true,
        drop_column: true,
        enforced_check: true,
        column_reorder: engine == SchemaEngine::MySql,
        transactional_ddl: engine != SchemaEngine::MySql,
        ..ServerFeatures::default()
    }
}

/// `users(id bigint PK, email varchar(100) NOT NULL UNIQUE, name text,
/// org_id bigint FK → orgs(id), CHECK (length(email) > 3), INDEX on name)`.
fn users(engine: SchemaEngine) -> SchemaBaseline {
    let schema = match engine {
        SchemaEngine::Postgres => "public",
        SchemaEngine::MySql => "app",
        SchemaEngine::Sqlite => "main",
    };
    let mut id = col(1, "id", "bigint");
    id.nullable = false;
    let mut email = col(2, "email", "varchar(100)");
    email.nullable = false;
    let flavor = match engine {
        SchemaEngine::Postgres => ServerFlavor::Postgres,
        SchemaEngine::MySql => ServerFlavor::MySql,
        SchemaEngine::Sqlite => ServerFlavor::Sqlite,
    };
    SchemaBaseline {
        engine,
        server: ServerFacts {
            flavor,
            version: "1".into(),
            features: features(engine),
        },
        table: TableModel {
            schema: schema.into(),
            name: "users".into(),
            comment: None,
            columns: vec![
                id,
                email,
                col(3, "name", "text"),
                col(4, "org_id", "bigint"),
            ],
            primary_key: Some(KeyModel {
                origin: existing(100),
                name: Some("users_pkey".into()),
                columns: vec![existing(1)],
            }),
            uniques: vec![KeyModel {
                origin: existing(200),
                name: Some("users_email_key".into()),
                columns: vec![existing(2)],
            }],
            foreign_keys: vec![ForeignKeyModel {
                origin: existing(300),
                name: Some("users_org_id_fkey".into()),
                columns: vec![existing(4)],
                referenced_schema: None,
                referenced_table: "orgs".into(),
                referenced_columns: vec!["id".into()],
                on_update: ReferentialAction::NoAction,
                on_delete: ReferentialAction::Cascade,
                deferrable: Deferrable::Never,
            }],
            checks: vec![CheckModel {
                origin: existing(400),
                name: Some("users_email_check".into()),
                expression: SqlFragment::new("length(email) > 3"),
                enforced: true,
            }],
            indexes: vec![IndexModel {
                origin: existing(500),
                name: "users_name_idx".into(),
                unique: false,
                method: None,
                members: vec![IndexMember::column(existing(3))],
                include: Vec::new(),
                predicate: None,
            }],
            options: TableOptions::default(),
        },
        drift: DriftToken("fixture".into()),
        editable: Editability::Editable,
        inbound: Vec::new(),
        dependents: Vec::new(),
        estimated_rows: Some(10),
    }
}

fn model(engine: SchemaEngine) -> SchemaEditModel {
    let baseline = users(engine);
    let request = SchemaEditRequest::new(baseline.table.schema.clone(), "users");
    SchemaEditModel::new(request, baseline)
}

const PG: SchemaEngine = SchemaEngine::Postgres;

#[test]
fn a_fresh_draft_has_nothing_to_review() {
    let model = model(PG);
    assert!(!model.has_changes());
    assert_eq!(model.review_count(), 0);
    assert_eq!(
        model.review_block().as_deref(),
        Some("No changes to review")
    );
    assert!(model.issues().is_empty());
    for group in ChangeGroup::ALL {
        assert!(
            model
                .rows(group)
                .iter()
                .all(|row| row.state == RowState::Unchanged)
        );
    }
}

#[test]
fn renaming_a_column_is_one_changed_row_that_undo_reverts() {
    let mut model = model(PG);
    let target = Target::Column(existing(3));
    assert_eq!(model.set_input(target, Input::Name, "full_name"), Ok(true));
    let row = model.row(target);
    assert_eq!(row.state, RowState::Changed);
    assert_eq!(row.label, "full_name");
    let delta = row.delta(Attr::Name).unwrap();
    assert_eq!(
        (delta.before.as_str(), delta.after.as_str()),
        ("name", "full_name")
    );
    assert_eq!(model.review_count(), 1);
    assert_eq!(model.review_block(), None);
    // The index on the renamed column still lists it by origin.
    assert_eq!(
        model.input_value(Target::Index(existing(500)), Input::Members),
        "full_name"
    );
    assert!(model.changed(target, Attr::Name));
    assert_eq!(model.before(target, Attr::Name).as_deref(), Some("name"));
    assert!(model.revert(target, Attr::Name));
    assert!(!model.has_changes());
}

#[test]
fn per_field_undo_reverts_only_that_field() {
    let mut model = model(PG);
    let target = Target::Column(existing(3));
    model.set_input(target, Input::DataType, "integer").unwrap();
    model.set_choice(target, Choice::Nullable(false));
    model
        .set_input(target, Input::Comment, "display name")
        .unwrap();
    let row = model.row(target);
    assert_eq!(row.deltas.len(), 3);
    // Three field changes on one column still count once.
    assert_eq!(model.review_count(), 1);
    assert_eq!(
        row.delta(Attr::Field(Field::DataType)).unwrap().destructive,
        Some(Destructive::NarrowingType)
    );
    assert!(model.revert(target, Attr::Field(Field::DataType)));
    let row = model.row(target);
    assert!(row.delta(Attr::Field(Field::DataType)).is_none());
    assert!(row.delta(Attr::Field(Field::Nullable)).is_some());
    assert!(row.delta(Attr::Field(Field::Comment)).is_some());
    assert_eq!(model.input_value(target, Input::DataType), "text");
}

#[test]
fn removed_objects_stay_visible_and_can_be_restored() {
    let mut model = model(PG);
    let column = Target::Column(existing(3));
    assert!(model.remove(column));
    // DROP COLUMN takes the index on it along.
    let index = Target::Index(existing(500));
    assert!(!model.in_draft(index));
    let columns = model.rows(ChangeGroup::Columns);
    let removed = columns.iter().find(|row| row.target == column).unwrap();
    assert_eq!(removed.state, RowState::Removed);
    assert_eq!(removed.label, "name");
    assert_eq!(removed.destructive, Some(Destructive::DropColumn));
    assert_eq!(
        columns.last().unwrap().target,
        column,
        "removed rows come last"
    );
    assert_eq!(model.rows(ChangeGroup::Indexes)[0].state, RowState::Removed);
    assert!(
        model.inputs(column).is_empty(),
        "removed rows are not editable"
    );
    assert!(model.set_input(column, Input::Name, "x").is_err());

    model.restore_object(column).unwrap();
    assert_eq!(model.position(existing(3)), Some(3), "restored in place");
    model.restore_object(index).unwrap();
    assert!(!model.has_changes());
}

#[test]
fn new_objects_are_added_rows_and_undo_removes_them() {
    let mut model = model(PG);
    let target = model.add_column();
    let row = model.row(target);
    assert_eq!(row.state, RowState::Added);
    assert_eq!(row.label, "column_1");
    assert_eq!(model.input_value(target, Input::DataType), "text");
    let second = model.add_column();
    assert_eq!(model.label(second), "column_2");
    assert_eq!(model.rows(ChangeGroup::Columns)[5].target, second);
    assert_eq!(model.revert_object(target), Ok(true));
    assert!(!model.in_draft(target));
    assert_eq!(model.rows(ChangeGroup::Columns).len(), 5);
}

#[test]
fn key_columns_parse_names_and_refuse_unknown_ones() {
    let mut model = model(PG);
    let key = Target::Unique(existing(200));
    assert_eq!(
        model.set_input(key, Input::Columns, "email, \"name\""),
        Ok(true)
    );
    assert_eq!(model.input_value(key, Input::Columns), "email, name");
    assert_eq!(
        model
            .row(key)
            .delta(Attr::Field(Field::Columns))
            .unwrap()
            .destructive,
        Some(Destructive::Recreate)
    );
    let before = model.draft().clone();
    assert_eq!(
        model.set_input(key, Input::Columns, "email, missing"),
        Err("No column named missing".into())
    );
    assert_eq!(
        model.draft(),
        &before,
        "a refused edit leaves the draft as it was"
    );
    // PostgreSQL names are case sensitive; MySQL and SQLite are not.
    assert!(model.set_input(key, Input::Columns, "EMAIL").is_err());
    let mut mysql = super::tests::model(SchemaEngine::MySql);
    assert_eq!(mysql.set_input(key, Input::Columns, "EMAIL"), Ok(false));
}

#[test]
fn index_members_round_trip_and_keep_collation() {
    let mut model = model(PG);
    let index = Target::Index(existing(500));
    model
        .set_input(index, Input::Members, "name DESC, (lower(email)), id")
        .unwrap();
    let members = &model.index(existing(500)).unwrap().members;
    assert_eq!(members.len(), 3);
    assert!(members[0].descending);
    assert_eq!(
        members[1].target,
        IndexTarget::Expression(SqlFragment::new("lower(email)"))
    );
    assert_eq!(
        model.input_value(index, Input::Members),
        "name DESC, (lower(email)), id"
    );
    assert!(model.set_input(index, Input::Members, "()").is_err());

    let mut mysql = super::tests::model(SchemaEngine::MySql);
    mysql.set_input(index, Input::Members, "email(10)").unwrap();
    assert_eq!(
        mysql.index(existing(500)).unwrap().members[0].prefix_length,
        Some(10)
    );
    assert!(mysql.set_input(index, Input::Members, "email(x)").is_err());

    // A member that stays the same keeps a collation the text does not show.
    let mut baseline = users(PG);
    baseline.table.indexes[0].members[0].collation = Some("C".into());
    let mut model = SchemaEditModel::new(SchemaEditRequest::new("public", "users"), baseline);
    model.set_input(index, Input::Members, "name, id").unwrap();
    assert_eq!(
        model.index(existing(500)).unwrap().members[0]
            .collation
            .as_deref(),
        Some("C")
    );
}

#[test]
fn lists_split_on_top_level_commas_only() {
    assert_eq!(
        super::members::split_list("a, (f(b, c)), 'x,y', \"q,r\" , ,"),
        vec!["a", "(f(b, c))", "'x,y'", "\"q,r\""]
    );
    assert_eq!(super::members::unquote(" `a b` "), "a b");
    assert_eq!(super::members::unquote("\"x\""), "x");
    assert_eq!(super::members::unquote("plain"), "plain");
}

#[test]
fn capabilities_follow_the_engine() {
    let pg = model(PG);
    let sqlite = model(SchemaEngine::Sqlite);
    let mysql = model(SchemaEngine::MySql);
    assert!(!pg.caps().reorder && mysql.caps().reorder && !sqlite.caps().reorder);
    let column = Target::Column(existing(3));
    assert!(pg.inputs(column).contains(&Input::Comment));
    assert!(
        !sqlite.inputs(column).contains(&Input::Comment),
        "SQLite has no comments"
    );
    assert!(!sqlite.inputs(Target::Table).contains(&Input::Comment));
    assert!(mysql.inputs(column).contains(&Input::Charset));
    assert!(mysql.inputs(Target::Table).contains(&Input::Engine));
    assert!(
        pg.inputs(Target::Index(existing(500)))
            .contains(&Input::Include)
    );
    assert!(
        !mysql
            .inputs(Target::Index(existing(500)))
            .contains(&Input::Predicate)
    );
    assert_eq!(
        pg.caps().identities(),
        &[Identity::None, Identity::Always, Identity::ByDefault]
    );
    assert_eq!(
        sqlite.caps().identities(),
        &[Identity::None, Identity::AutoIncrement]
    );
}

#[test]
fn reorder_is_mysql_only_and_undoable_per_field() {
    let mut pg = model(PG);
    assert!(!pg.move_column(existing(3), true));
    let mut mysql = model(SchemaEngine::MySql);
    assert!(mysql.move_column(existing(3), true));
    assert_eq!(mysql.position(existing(3)), Some(2));
    assert!(!mysql.move_column(existing(1), true), "already first");
    // The diff reports the fewest moved columns; either neighbour may be it.
    let moved = [existing(2), existing(3)]
        .into_iter()
        .map(Target::Column)
        .find(|target| mysql.changed(*target, Attr::Field(Field::Position)))
        .expect("a column moved");
    assert!(mysql.revert(moved, Attr::Field(Field::Position)));
    assert!(!mysql.has_changes());
}

#[test]
fn choices_keep_typed_text_and_identity_implies_not_null() {
    let mut model = model(PG);
    let target = Target::Column(existing(3));
    assert!(!model.input_enabled(target, Input::Default));
    assert!(model.set_choice(target, Choice::DefaultKind(DefaultKind::Literal)));
    model.set_input(target, Input::Default, "anon").unwrap();
    assert!(model.set_choice(target, Choice::DefaultKind(DefaultKind::Expression)));
    assert_eq!(
        model.column(existing(3)).unwrap().default,
        DefaultValue::Expression(SqlFragment::new("anon"))
    );
    assert_eq!(model.default_kind(existing(3)), DefaultKind::Expression);
    assert!(model.set_choice(target, Choice::DefaultKind(DefaultKind::None)));
    assert!(!model.input_enabled(target, Input::Default));

    assert!(model.set_input(target, Input::Generated, "x").is_err());
    assert!(model.set_choice(target, Choice::GeneratedKind(GeneratedKind::Stored)));
    model
        .set_input(target, Input::Generated, "upper(email)")
        .unwrap();
    assert_eq!(model.generated_kind(existing(3)), GeneratedKind::Stored);

    let org = Target::Column(existing(4));
    assert!(model.set_choice(org, Choice::Identity(Identity::ByDefault)));
    assert!(!model.column(existing(4)).unwrap().nullable);
}

#[test]
fn validation_issues_block_review_and_locate_the_row() {
    let mut model = model(PG);
    let target = Target::Column(existing(3));
    model.set_input(target, Input::Name, "").unwrap();
    assert_eq!(model.row(target).issues, 1);
    assert_eq!(
        model.review_block().as_deref(),
        Some("Column name is empty")
    );
    model.set_input(target, Input::Name, "email").unwrap();
    assert!(model.review_block().unwrap().contains("used twice"));
    let check = model.add_check();
    assert!(
        model.issues_for(check).count() > 0,
        "an empty check is an issue"
    );
    assert!(model.review_block().unwrap().starts_with("Fix the"));
}

#[test]
fn discard_and_rebase() {
    let mut model = model(PG);
    model
        .set_input(Target::Table, Input::Comment, "people")
        .unwrap();
    model
        .set_input(Target::Column(existing(3)), Input::DataType, "integer")
        .unwrap();
    assert!(model.has_changes());
    // Same table, new drift token: the draft carries over as is.
    let mut fresh = users(PG);
    fresh.drift = DriftToken("moved".into());
    assert!(model.rebase(fresh));
    assert!(model.has_changes());
    assert!(model.conflicts().is_empty());
    // The server changed the comment too and added a column: the draft is
    // re-based, the server's column shows through and the comment both
    // sides changed keeps the draft's value as a conflict.
    let mut changed = users(PG);
    changed.table.comment = Some("server".into());
    changed
        .table
        .columns
        .push(col(5, "created_at", "timestamptz"));
    assert!(!model.rebase(changed));
    assert_eq!(model.conflicts().len(), 1);
    assert_eq!(model.input_value(Target::Table, Input::Comment), "people");
    assert_eq!(
        model.input_value(Target::Column(existing(3)), Input::DataType),
        "integer"
    );
    let added = Target::Column(existing(5));
    assert!(model.in_draft(added));
    assert_eq!(model.row(added).state, RowState::Unchanged);
    model.add_column();
    model.discard();
    assert!(!model.has_changes());
    assert!(model.conflicts().is_empty());
    assert_eq!(model.draft().table, model.baseline().table);
}

#[test]
fn a_saved_draft_is_rebased_onto_a_changed_table() {
    let mut model = model(PG);
    model
        .set_input(Target::Column(existing(3)), Input::Name, "full_name")
        .unwrap();
    let mut changed = users(PG);
    changed.table.comment = Some("server".into());
    let rebased = SchemaEditModel::rebased(
        model.request().clone(),
        model.baseline(),
        changed,
        model.draft(),
    );
    assert!(rebased.conflicts().is_empty());
    assert_eq!(rebased.label(Target::Column(existing(3))), "full_name");
    assert_eq!(rebased.input_value(Target::Table, Input::Comment), "server");
    assert_eq!(rebased.review_count(), 1);
}

#[test]
fn primary_keys_are_single_and_named_per_engine() {
    let mut model = model(PG);
    assert!(model.add_primary_key().is_err());
    assert!(model.remove(Target::PrimaryKey(existing(100))));
    let added = model.add_primary_key().unwrap();
    assert_eq!(model.input_value(added, Input::Name), "users_pkey");
    assert!(
        model
            .restore_object(Target::PrimaryKey(existing(100)))
            .is_err()
    );
    let keys = model.rows(ChangeGroup::Keys);
    assert_eq!(keys[0].state, RowState::Added);
    assert_eq!(keys[1].state, RowState::Removed);

    let mut sqlite = super::tests::model(SchemaEngine::Sqlite);
    sqlite.remove(Target::PrimaryKey(existing(100)));
    let added = sqlite.add_primary_key().unwrap();
    assert_eq!(sqlite.input_value(added, Input::Name), "");
}

#[test]
fn foreign_key_inputs_and_actions() {
    let mut model = model(PG);
    let key = Target::ForeignKey(existing(300));
    model
        .set_input(key, Input::ReferencedTable, "teams")
        .unwrap();
    model
        .set_input(key, Input::ReferencedColumns, "id")
        .unwrap();
    assert!(model.set_choice(key, Choice::OnDelete(ReferentialAction::SetNull)));
    let row = model.row(key);
    assert!(row.delta(Attr::Field(Field::Reference)).is_some());
    assert!(row.delta(Attr::Field(Field::OnDeleteAction)).is_some());
    assert!(model.revert(key, Attr::Field(Field::Reference)));
    assert_eq!(model.input_value(key, Input::ReferencedTable), "orgs");
    assert!(model.revert_object(key).unwrap());
    assert!(!model.has_changes());
    let added = model.add_foreign_key();
    assert_eq!(model.input_value(added, Input::Name), "users_fkey1");
    assert!(model.issues_for(added).count() > 0);
}

#[test]
fn oversized_input_is_refused() {
    let mut model = model(PG);
    let text = "x".repeat(INPUT_BYTES + 1);
    assert!(
        model
            .set_input(Target::Table, Input::Comment, &text)
            .is_err()
    );
    assert!(!model.has_changes());
}

#[test]
fn read_only_baselines_explain_why() {
    let mut baseline = users(SchemaEngine::MySql);
    baseline.editable = Editability::ReadOnly {
        reasons: vec!["MySQL 5.7 is not editable".into()],
    };
    let model = SchemaEditModel::new(SchemaEditRequest::new("app", "users"), baseline);
    assert!(!model.editable());
    assert_eq!(
        model.read_only_reasons(),
        &["MySQL 5.7 is not editable".to_owned()]
    );
    assert_eq!(
        model.review_block().as_deref(),
        Some("MySQL 5.7 is not editable")
    );
}

// -- Gate ------------------------------------------------------------------

const PHASES: [ReviewPhase; 7] = [
    ReviewPhase::Ready,
    ReviewPhase::Saving,
    ReviewPhase::AutoConfirming,
    ReviewPhase::Applying,
    ReviewPhase::NeedsTyped,
    ReviewPhase::Failed,
    ReviewPhase::Settled,
];

#[test]
fn apply_gate_truth_table() {
    let styles = [None, Some(ConfirmStyle::Plain), Some(ConfirmStyle::Typed)];
    for phase in PHASES {
        for style in styles {
            for typed in [false, true] {
                for read_only in [false, true] {
                    let expected = !read_only
                        && match (phase, style) {
                            (_, None) => false,
                            (ReviewPhase::Ready, Some(ConfirmStyle::Plain)) => true,
                            (ReviewPhase::Ready, Some(ConfirmStyle::Typed)) => typed,
                            (ReviewPhase::NeedsTyped, Some(_)) => typed,
                            _ => false,
                        };
                    assert_eq!(
                        apply_enabled(style, typed, phase, read_only),
                        expected,
                        "{phase:?} {style:?} typed={typed} read_only={read_only}"
                    );
                }
            }
        }
    }
}

#[test]
fn phase_and_escape_follow_the_apply_flow() {
    let none: Option<&ApplyFlow<()>> = None;
    assert_eq!(review_phase(none, 0, true, false), ReviewPhase::Ready);
    assert_eq!(review_phase(none, 0, false, false), ReviewPhase::Failed);
    assert_eq!(review_phase(none, 0, true, true), ReviewPhase::Settled);

    let mut flow = ApplyFlow::new(1, "review");
    assert_eq!(
        review_phase(Some(&flow), 0, false, false),
        ReviewPhase::Saving
    );
    assert_eq!(
        review_escape(ReviewPhase::Saving),
        ReviewEscape::CancelBeforeDispatch
    );
    assert_eq!(flow.saved(1), Some("review"));
    let phase = review_phase(Some(&flow), 0, false, false);
    assert_eq!(phase, ReviewPhase::Applying);
    assert_eq!(review_escape(phase), ReviewEscape::Ignore);
    flow.needs_confirmation("confirmation");
    assert_eq!(
        review_phase(Some(&flow), 0, false, false),
        ReviewPhase::NeedsTyped
    );
    assert!(flow.confirm(2));
    assert_eq!(
        review_phase(Some(&flow), 1, false, false),
        ReviewPhase::AutoConfirming
    );
    for phase in [
        ReviewPhase::Ready,
        ReviewPhase::Failed,
        ReviewPhase::Settled,
    ] {
        assert_eq!(review_escape(phase), ReviewEscape::Close);
    }
}

#[test]
fn apply_label_names_confirmation_and_marks_typed_destructive_as_danger() {
    let staging = TablePolicy::resolve(
        DevelopmentEnvironment::Staging,
        DevelopmentSafeMode::Inherit,
        false,
    );
    let development = TablePolicy::resolve(
        DevelopmentEnvironment::Development,
        DevelopmentSafeMode::Inherit,
        false,
    );
    let production = TablePolicy::resolve(
        DevelopmentEnvironment::Production,
        DevelopmentSafeMode::Inherit,
        false,
    );
    assert_eq!(
        apply_label(&staging, ReviewPhase::Ready, 2, true),
        ("Confirm and apply".to_owned(), false)
    );
    assert_eq!(
        apply_label(&development, ReviewPhase::Ready, 1, false),
        ("Apply 1 change".to_owned(), false)
    );
    assert_eq!(
        apply_label(&production, ReviewPhase::Ready, 3, true),
        ("Apply 3 changes".to_owned(), true)
    );
    assert!(!apply_label(&production, ReviewPhase::Ready, 3, false).1);
    assert_eq!(
        apply_label(&development, ReviewPhase::Applying, 1, false).0,
        "Applying…"
    );
}

fn record(id: &str, settings: Option<DevelopmentEngineConnection>) -> DevelopmentConnection {
    DevelopmentConnection {
        id: id.into(),
        name: id.into(),
        engine: "mysql".into(),
        organization: DevelopmentConnectionOrganization::default(),
        unsupported_reason: None,
        postgres: None,
        environment: DevelopmentEnvironment::Development,
        settings,
        last_activity_at: None,
    }
}

#[test]
fn policy_resolves_mysql_and_sqlite_records() {
    let mysql = DevelopmentEngineConnection::MySQL(DevelopmentMySqlConnection {
        name: "my".into(),
        host: "localhost".into(),
        port: 3306,
        database: "app".into(),
        user: "app".into(),
        environment: DevelopmentEnvironment::Production,
        safe_mode: DevelopmentSafeMode::Inherit,
        read_only: false,
        ssl: false,
        ssh_tunnel: None,
    });
    let sqlite = DevelopmentEngineConnection::SQLite(DevelopmentSqliteConnection {
        name: "lite".into(),
        path: "/tmp/app.db".into(),
        environment: DevelopmentEnvironment::Development,
        safe_mode: DevelopmentSafeMode::Inherit,
        read_only: true,
    });
    let records = [record("my", Some(mysql)), record("lite", Some(sqlite))];
    let production = policy_for(&records, "my");
    assert_eq!(production.confirm_style(), Some(ConfirmStyle::Typed));
    assert!(policy_for(&records, "lite").read_only);
    assert_eq!(policy_for(&records, "gone"), TablePolicy::UNKNOWN);
    assert_eq!(
        policy_for(&[record("bare", None)], "bare"),
        TablePolicy::UNKNOWN
    );
}

#[test]
fn plan_toggles_match_engine_and_changes() {
    let mut pg = model(PG);
    assert!(plan_toggles(PG, pg.changes()).is_empty());
    let index = pg.add_index();
    assert_eq!(
        plan_toggles(PG, pg.changes()),
        vec![PlanToggle::ConcurrentIndexes]
    );
    let options = toggle(
        &PlanOptions::default(),
        PlanToggle::ConcurrentIndexes,
        pg.changes(),
    );
    assert_eq!(options.concurrent_indexes, vec![index.origin().unwrap()]);
    assert!(toggle_on(&options, PlanToggle::ConcurrentIndexes));
    let off = toggle(&options, PlanToggle::ConcurrentIndexes, pg.changes());
    assert!(off.concurrent_indexes.is_empty());
    pg.add_check();
    assert!(plan_toggles(PG, pg.changes()).contains(&PlanToggle::ValidateSeparately));
    let mysql = model(SchemaEngine::MySql);
    assert_eq!(
        plan_toggles(SchemaEngine::MySql, mysql.changes()),
        vec![PlanToggle::OnlineOnly]
    );
    assert!(plan_toggles(SchemaEngine::Sqlite, mysql.changes()).is_empty());

    // Re-creating an existing index (unique toggled) is never offered
    // CONCURRENTLY: the planner refuses it for existing indexes.
    let mut recreated = model(PG);
    assert!(recreated.set_choice(Target::Index(existing(500)), Choice::Unique(true)));
    assert!(recreated.changes().changes.iter().any(|change| {
        change.object == SchemaObject::Index && change.destructive == Some(Destructive::Recreate)
    }));
    assert!(concurrent_candidates(recreated.changes()).is_empty());
    assert!(!plan_toggles(PG, recreated.changes()).contains(&PlanToggle::ConcurrentIndexes));
    // A new index next to it is the only candidate.
    let added = recreated.add_index();
    assert_eq!(
        concurrent_candidates(recreated.changes()),
        vec![added.origin().unwrap()]
    );
}

fn preview(statements: &[&str], groups: Vec<PreviewGroup>) -> SchemaEditPreview {
    SchemaEditPreview {
        engine: PG,
        changes: Default::default(),
        statements: statements
            .iter()
            .map(|sql| PreviewStatement {
                sql: (*sql).into(),
                summary: String::new(),
                destructive: false,
            })
            .collect(),
        groups,
        notes: vec![EngineNote::new(NoteKind::Lock, "ACCESS EXCLUSIVE")],
        options: PlanOptions::default(),
    }
}

#[test]
fn sql_document_heads_each_group() {
    let single = preview(
        &["ALTER TABLE a ADD b int", "COMMENT ON TABLE a IS 'x';"],
        vec![PreviewGroup::Atomic {
            statements: vec![0, 1],
        }],
    );
    assert_eq!(
        sql_document(&single),
        "ALTER TABLE a ADD b int;\nCOMMENT ON TABLE a IS 'x';\n"
    );
    let mixed = preview(
        &[
            "ALTER TABLE a ADD b int",
            "CREATE INDEX CONCURRENTLY i ON a (b)",
        ],
        vec![
            PreviewGroup::Atomic {
                statements: vec![0],
            },
            PreviewGroup::Standalone { statement: 1 },
        ],
    );
    assert_eq!(
        sql_document(&mixed),
        "-- Group 1 of 2: runs in one transaction\nALTER TABLE a ADD b int;\n\n-- Group 2 of 2: commits on its own\nCREATE INDEX CONCURRENTLY i ON a (b);\n"
    );
    assert_eq!(note_label(NoteKind::Rebuild), "Rebuild");
    assert!(note_warns(NoteKind::NonAtomic));
    assert!(!note_warns(NoteKind::Info));
}

#[test]
fn outcomes_say_what_happened_to_the_table() {
    let applied = outcome_text(&SchemaEditOutcome::Applied { runtime_ms: 12 }, 1);
    assert!(!applied.error && applied.text.starts_with("Applied in 12 ms"));
    let stopped = outcome_text(
        &SchemaEditOutcome::Stopped {
            committed: 2,
            failed_statement: 2,
            reason: SchemaEditFailure::LockTimeout,
        },
        3,
    );
    assert!(stopped.error);
    assert!(stopped.text.starts_with("Stopped at statement 3 of 3"));
    assert!(stopped.text.contains("Statements 1 to 2 committed."));
    assert!(stopped.text.contains("partly changed"));
    let unknown = outcome_text(
        &SchemaEditOutcome::OutcomeUnknown {
            reason: SchemaEditFailure::Connection,
        },
        1,
    );
    assert!(unknown.text.contains("never replayed"));
    let rolled = outcome_text(
        &SchemaEditOutcome::RolledBack {
            reason: SchemaEditFailure::ForeignKeyViolation,
        },
        1,
    );
    assert!(rolled.text.ends_with("The table is unchanged."));
}

// -- Journal -----------------------------------------------------------------

fn attempt_record(
    model: &SchemaEditModel,
    state: WorkspaceApplyState,
) -> (WorkspaceSchemaEdit, SchemaEditAttemptId) {
    let id = SchemaEditAttemptId::new();
    let record = WorkspaceSchemaEdit {
        engine: PG,
        request: model.request().clone(),
        baseline: model.baseline().clone(),
        draft: model.draft().clone(),
        attempt: Some(WorkspaceSchemaEditAttempt {
            attempt_id: id.clone(),
            preview: preview(
                &["SELECT 1"],
                vec![PreviewGroup::Standalone { statement: 0 }],
            ),
            apply_state: state,
        }),
    };
    (record, id)
}

fn receipt(
    model: &SchemaEditModel,
    id: SchemaEditAttemptId,
    outcome: SchemaEditOutcome,
) -> SchemaEditReceipt {
    SchemaEditReceipt {
        attempt_id: id,
        engine: PG,
        connection_id: "pg".into(),
        request: model.request().clone(),
        outcome,
    }
}

#[test]
fn journal_keeps_only_changed_drafts_and_never_overwrites_unknown() {
    let mut model = model(PG);
    let mut recovery = Recovery::default();
    assert_eq!(
        recovery.save_draft(model.request(), model.baseline(), model.draft()),
        DraftSave::Cleared
    );
    assert!(recovery.record().is_none());
    model
        .set_input(Target::Table, Input::Comment, "people")
        .unwrap();
    assert_eq!(
        recovery.save_draft(model.request(), model.baseline(), model.draft()),
        DraftSave::Saved
    );
    assert_eq!(
        recovery.restorable(model.request(), PG),
        Some((model.baseline(), model.draft()))
    );
    // Another table or engine never restores this draft.
    let other = SchemaEditRequest::new("public", "orgs");
    assert_eq!(recovery.restorable(&other, PG), None);
    assert_eq!(
        recovery.restorable(model.request(), SchemaEngine::MySql),
        None
    );

    let (record, _) = attempt_record(&model, WorkspaceApplyState::OutcomeUnknown);
    let mut recovery = Recovery::new(Some(record), true);
    assert!(recovery.unknown());
    assert!(recovery.attempt_id().is_some());
    assert_eq!(
        recovery.save_draft(model.request(), model.baseline(), model.draft()),
        DraftSave::Blocked
    );
    assert!(!recovery.discard(false), "unknown needs reconciliation");
    assert!(recovery.discard(true));
    assert!(recovery.record().is_none());
}

#[test]
fn receipts_settle_only_their_own_attempt() {
    let mut model = model(PG);
    model
        .set_input(Target::Table, Input::Comment, "people")
        .unwrap();

    let (record, id) = attempt_record(&model, WorkspaceApplyState::OutcomeUnknown);
    let mut recovery = Recovery::new(Some(record), true);
    let applied = receipt(&model, id, SchemaEditOutcome::Applied { runtime_ms: 1 });
    assert_eq!(
        recovery.settle(&applied, model.request()),
        Settlement::Applied
    );
    assert!(recovery.record().is_none());

    let (record, id) = attempt_record(&model, WorkspaceApplyState::OutcomeUnknown);
    let mut recovery = Recovery::new(Some(record), true);
    let rolled = receipt(
        &model,
        id,
        SchemaEditOutcome::RolledBack {
            reason: SchemaEditFailure::Timeout,
        },
    );
    assert_eq!(
        recovery.settle(&rolled, model.request()),
        Settlement::Unchanged
    );
    let kept = recovery.record().unwrap();
    assert!(kept.attempt.is_none(), "the draft is kept for editing");
    assert_eq!(kept.draft, *model.draft());

    let (record, id) = attempt_record(&model, WorkspaceApplyState::OutcomeUnknown);
    let mut recovery = Recovery::new(Some(record), true);
    let stopped = receipt(
        &model,
        id,
        SchemaEditOutcome::Stopped {
            committed: 1,
            failed_statement: 1,
            reason: SchemaEditFailure::LockTimeout,
        },
    );
    assert_eq!(
        recovery.settle(&stopped, model.request()),
        Settlement::Partial
    );
    assert!(recovery.record().is_none());

    // Another attempt's receipt leaves the journal unknown.
    let (record, _) = attempt_record(&model, WorkspaceApplyState::OutcomeUnknown);
    let mut recovery = Recovery::new(Some(record), true);
    let foreign = receipt(
        &model,
        SchemaEditAttemptId::new(),
        SchemaEditOutcome::Applied { runtime_ms: 1 },
    );
    assert_eq!(
        recovery.settle(&foreign, model.request()),
        Settlement::Unknown
    );
    assert!(recovery.unknown());

    // A receipt for a staged (not yet journaled as unknown) attempt is refused.
    let (record, id) = attempt_record(&model, WorkspaceApplyState::Staged);
    let mut recovery = Recovery::new(Some(record), true);
    let early = receipt(&model, id, SchemaEditOutcome::Applied { runtime_ms: 1 });
    assert_eq!(
        recovery.settle(&early, model.request()),
        Settlement::Unknown
    );
    assert!(recovery.unknown());
}

#[test]
fn refusals_before_dispatch_keep_the_draft_but_lost_replies_stay_unknown() {
    let mut model = model(PG);
    model
        .set_input(Target::Table, Input::Comment, "people")
        .unwrap();
    let (record, _) = attempt_record(&model, WorkspaceApplyState::OutcomeUnknown);
    let mut recovery = Recovery::new(Some(record.clone()), true);
    recovery.submission_error(&SchemaEditError::Busy);
    assert!(!recovery.unknown());
    assert!(recovery.record().unwrap().attempt.is_none());

    let mut recovery = Recovery::new(Some(record), true);
    recovery.submission_error(&SchemaEditError::OutcomeUnavailable);
    assert!(recovery.unknown());
}

fn staged(baseline: SchemaBaseline) -> WorkspaceSchemaEdit {
    let draft = SchemaDraft::from_baseline(&baseline);
    WorkspaceSchemaEdit {
        engine: PG,
        request: SchemaEditRequest::new("public", "users"),
        baseline,
        draft,
        attempt: Some(WorkspaceSchemaEditAttempt {
            attempt_id: SchemaEditAttemptId::new(),
            preview: preview(
                &["SELECT 1"],
                vec![PreviewGroup::Standalone { statement: 0 }],
            ),
            apply_state: WorkspaceApplyState::Staged,
        }),
    }
}

/// A table whose journal record exceeds the 192 KiB workspace allowance.
fn wide() -> SchemaBaseline {
    let mut baseline = users(PG);
    for id in 10..50 {
        let mut column = col(id, &format!("wide_{id}"), "text");
        column.comment = Some("x".repeat(8 * 1024));
        baseline.table.columns.push(column);
    }
    baseline
}

#[test]
fn only_a_persisted_journal_refuses_an_attempt_it_cannot_hold() {
    // PostgreSQL: the attempt must be journaled before dispatch.
    let mut persisted = Recovery::new(None, true);
    assert_eq!(persisted.stage_record(staged(wide())), Err(JOURNAL_REFUSAL));
    assert!(persisted.record().is_none(), "nothing was staged");
    assert!(persisted.stage_record(staged(users(PG))).is_ok());
    assert!(
        persisted.unknown(),
        "a staged attempt is unknown until settled"
    );

    // MySQL and SQLite keep the record in memory only: a wide table applies.
    let mut memory = Recovery::new(None, false);
    assert!(memory.stage_record(staged(wide())).is_ok());
    assert!(memory.unknown());
    assert_eq!(
        memory.stage_record(staged(users(PG))),
        Err("Reconcile the previous unknown outcome first")
    );
}

#[test]
fn an_in_memory_journal_forgets_drafts_but_keeps_attempts() {
    let mut model = model(PG);
    model
        .set_input(Target::Table, Input::Comment, "people")
        .unwrap();
    let mut recovery = Recovery::new(None, false);
    recovery.save_draft(model.request(), model.baseline(), model.draft());
    assert!(recovery.record().is_some());
    recovery.forget_draft();
    assert!(recovery.record().is_none());

    let (record, _) = attempt_record(&model, WorkspaceApplyState::OutcomeUnknown);
    let mut recovery = Recovery::new(Some(record), false);
    recovery.forget_draft();
    assert!(recovery.unknown(), "an unknown attempt is never forgotten");
}

#[test]
fn row_view_data_is_precomputed_per_edit() {
    let mut model = model(PG);
    // Columns title and heads, four column rows, then the next section.
    assert_eq!(
        &model.lines()[..7],
        &[
            Line::Title(ChangeGroup::Columns),
            Line::Head(ChangeGroup::Columns),
            Line::Row(ChangeGroup::Columns, 0),
            Line::Row(ChangeGroup::Columns, 1),
            Line::Row(ChangeGroup::Columns, 2),
            Line::Row(ChangeGroup::Columns, 3),
            Line::Title(ChangeGroup::Keys),
        ]
    );
    let name = Target::Column(existing(3));
    assert_eq!(model.line_of(name), Some(4));
    assert_eq!(model.position(existing(4)), Some(4));

    model.set_input(name, Input::Name, "full_name").unwrap();
    let row = &model.rows(ChangeGroup::Columns)[2];
    assert_eq!(row.label, "full_name");
    assert_eq!(row.state, RowState::Changed);
    assert_eq!(row.description, "full_name, changed: name to full_name");
    assert_eq!(model.row(name), row.clone());
    assert_eq!(model.review_count(), model.changes().object_count());

    // A removed column moves to the end of its section; later positions shift.
    assert!(model.remove(name));
    assert_eq!(model.position(existing(4)), Some(3));
    assert_eq!(model.position(existing(3)), None);
    let columns = model.rows(ChangeGroup::Columns);
    assert_eq!(columns.last().map(|row| row.target), Some(name));
    assert_eq!(model.line_of(name), Some(5));
    assert_eq!(model.review_count(), model.changes().object_count());

    // Issues are counted on their row.
    let check = model.add_check();
    assert_eq!(model.row(check).issues, model.issues_for(check).count());
    assert!(
        model.row(check).issues > 0,
        "an empty check expression is a problem"
    );
}
