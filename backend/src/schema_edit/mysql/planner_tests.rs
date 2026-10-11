//! Golden SQL for the MySQL planner, through `plan_review` on the shared
//! `users` fixture (`app`.`users`: id, email, name, org_id; PK, unique
//! email, FK org_id → orgs, check on email, index on name).
use crate::schema_edit::fixtures::{col, existing, users};
use crate::schema_edit::*;

fn setup() -> (SchemaBaseline, SchemaDraft) {
    let baseline = users(SchemaEngine::MySql);
    let draft = SchemaDraft::from_baseline(&baseline);
    (baseline, draft)
}

fn sql(baseline: &SchemaBaseline, draft: &SchemaDraft, options: &PlanOptions) -> Vec<String> {
    plan_review(baseline, draft, options)
        .unwrap_or_else(|error| panic!("plan failed: {error:?}"))
        .statements
        .into_iter()
        .map(|statement| statement.sql)
        .collect()
}

fn plan(baseline: &SchemaBaseline, draft: &SchemaDraft) -> SchemaEditPreview {
    plan_review(baseline, draft, &PlanOptions::default()).unwrap()
}

fn kinds(preview: &SchemaEditPreview) -> Vec<NoteKind> {
    preview.notes.iter().map(|note| note.kind).collect()
}

fn online() -> PlanOptions {
    PlanOptions {
        online_only: true,
        ..PlanOptions::default()
    }
}

#[test]
fn add_column_is_one_statement() {
    let (baseline, mut draft) = setup();
    draft.add_column("age", "int");
    let preview = plan(&baseline, &draft);
    assert_eq!(
        preview
            .statements
            .iter()
            .map(|s| s.sql.as_str())
            .collect::<Vec<_>>(),
        vec!["ALTER TABLE `app`.`users` ADD COLUMN `age` int NULL"]
    );
    assert_eq!(preview.groups, vec![PreviewGroup::Standalone { statement: 0 }]);
    assert!(!preview.statements[0].destructive);
    assert_eq!(kinds(&preview), vec![NoteKind::Lock]);
}

#[test]
fn combined_changes_share_one_alter_in_plan_order() {
    let (baseline, mut draft) = setup();
    // Dropping `name` also drops its index.
    draft.remove_column(existing(3));
    let age = draft.add_column("age", "int");
    {
        let column = draft.table.column_mut(age).unwrap();
        column.nullable = false;
        column.default = DefaultValue::Literal("0".into());
    }
    draft.table.column_mut(existing(2)).unwrap().data_type = "varchar(150)".into();
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
    let preview = plan(&baseline, &draft);
    assert_eq!(
        preview
            .statements
            .iter()
            .map(|s| s.sql.as_str())
            .collect::<Vec<_>>(),
        vec![
            "ALTER TABLE `app`.`users` DROP INDEX `users_name_idx`, \
             MODIFY COLUMN `email` varchar(150) NOT NULL, \
             ADD COLUMN `age` int NOT NULL DEFAULT '0', \
             DROP COLUMN `name`, \
             ADD INDEX `users_email_idx` (`email`)"
        ]
    );
    assert!(preview.statements[0].destructive);
    assert!(kinds(&preview).contains(&NoteKind::Rewrite));
    assert!(!kinds(&preview).contains(&NoteKind::NonAtomic));
}

#[test]
fn foreign_key_changes_run_in_their_own_statements() {
    let (baseline, mut draft) = setup();
    draft.table.foreign_keys[0].on_delete = ReferentialAction::Restrict;
    draft.add_column("age", "int");
    let preview = plan(&baseline, &draft);
    assert_eq!(
        preview
            .statements
            .iter()
            .map(|s| s.sql.as_str())
            .collect::<Vec<_>>(),
        vec![
            "ALTER TABLE `app`.`users` DROP FOREIGN KEY `users_org_id_fkey`",
            "ALTER TABLE `app`.`users` ADD COLUMN `age` int NULL",
            "ALTER TABLE `app`.`users` ADD CONSTRAINT `users_org_id_fkey` FOREIGN KEY (`org_id`) \
             REFERENCES `app`.`orgs` (`id`) ON DELETE RESTRICT ON UPDATE NO ACTION",
        ]
    );
    assert_eq!(
        preview.groups,
        vec![
            PreviewGroup::Standalone { statement: 0 },
            PreviewGroup::Standalone { statement: 1 },
            PreviewGroup::Standalone { statement: 2 },
        ]
    );
    assert!(kinds(&preview).contains(&NoteKind::NonAtomic));
    assert!(preview.statements[0].destructive);
}

#[test]
fn modify_restates_the_whole_column() {
    let (mut baseline, _) = setup();
    {
        let email = baseline.table.column_mut(existing(2)).unwrap();
        email.extra.charset = Some("utf8mb4".into());
        email.collation = Some("utf8mb4_bin".into());
        email.comment = Some("Login".into());
        email.default = DefaultValue::Literal("nobody@example.com".into());
    }
    let mut updated = col(5, "updated_at", "timestamp");
    updated.default = DefaultValue::Expression(SqlFragment::new("CURRENT_TIMESTAMP"));
    updated.extra.on_update = Some(SqlFragment::new("CURRENT_TIMESTAMP"));
    updated.extra.invisible = true;
    updated.comment = Some("Touched".into());
    baseline.table.columns.push(updated);
    let mut display = col(6, "display", "varchar(201)");
    display.generated = Some(Generated {
        expression: SqlFragment::new("concat(`name`,_utf8mb4' ')"),
        stored: false,
    });
    display.extra.charset = Some("utf8mb4".into());
    baseline.table.columns.push(display);
    let mut draft = SchemaDraft::from_baseline(&baseline);

    draft.table.column_mut(existing(2)).unwrap().data_type = "varchar(200)".into();
    draft.table.column_mut(existing(5)).unwrap().comment = Some("Last touch".into());
    draft.table.column_mut(existing(6)).unwrap().comment = Some("Shown".into());
    let preview = plan(&baseline, &draft);
    assert_eq!(
        preview
            .statements
            .iter()
            .map(|s| s.sql.as_str())
            .collect::<Vec<_>>(),
        vec![
            "ALTER TABLE `app`.`users` \
             MODIFY COLUMN `email` varchar(200) CHARACTER SET utf8mb4 COLLATE utf8mb4_bin NOT NULL \
             DEFAULT 'nobody@example.com' COMMENT 'Login', \
             MODIFY COLUMN `updated_at` timestamp NULL DEFAULT CURRENT_TIMESTAMP \
             ON UPDATE CURRENT_TIMESTAMP INVISIBLE COMMENT 'Last touch', \
             MODIFY COLUMN `display` varchar(201) CHARACTER SET utf8mb4 \
             GENERATED ALWAYS AS (concat(`name`,_utf8mb4' ')) VIRTUAL NULL COMMENT 'Shown'"
        ]
    );
    // Widening varchar(100) to varchar(200) is not destructive.
    assert!(!preview.statements[0].destructive);
}

#[test]
fn rename_uses_rename_column_or_change() {
    let (mut baseline, mut draft) = setup();
    draft.table.column_mut(existing(3)).unwrap().name = "full_name".into();
    assert_eq!(
        sql(&baseline, &draft, &PlanOptions::default()),
        vec!["ALTER TABLE `app`.`users` RENAME COLUMN `name` TO `full_name`"]
    );

    // Without RENAME COLUMN the column is restated with CHANGE.
    baseline.server.features.rename_column = false;
    assert_eq!(
        sql(&baseline, &draft, &PlanOptions::default()),
        vec!["ALTER TABLE `app`.`users` CHANGE COLUMN `name` `full_name` text NULL"]
    );

    // A rename with another change is always CHANGE; text to varchar narrows.
    baseline.server.features.rename_column = true;
    draft.table.column_mut(existing(3)).unwrap().data_type = "varchar(50)".into();
    let preview = plan(&baseline, &draft);
    assert_eq!(
        preview.statements[0].sql,
        "ALTER TABLE `app`.`users` CHANGE COLUMN `name` `full_name` varchar(50) NULL"
    );
    assert!(preview.statements[0].destructive);
}

#[test]
fn reorder_uses_first_and_after() {
    let (baseline, mut draft) = setup();
    draft.move_column(existing(2), 2);
    assert_eq!(
        sql(&baseline, &draft, &PlanOptions::default()),
        vec!["ALTER TABLE `app`.`users` MODIFY COLUMN `email` varchar(100) NOT NULL AFTER `name`"]
    );

    let (baseline, mut draft) = setup();
    draft.move_column(existing(4), 0);
    assert_eq!(
        sql(&baseline, &draft, &PlanOptions::default()),
        vec!["ALTER TABLE `app`.`users` MODIFY COLUMN `org_id` bigint NULL FIRST"]
    );

    // A new column placed mid-table goes after its draft predecessor.
    let (baseline, mut draft) = setup();
    let age = draft.add_column("age", "int");
    draft.move_column(age, 2);
    assert_eq!(
        sql(&baseline, &draft, &PlanOptions::default()),
        vec!["ALTER TABLE `app`.`users` ADD COLUMN `age` int NULL AFTER `email`"]
    );

    // Clauses follow draft order, so AFTER may name a column added earlier
    // in the same statement.
    let (baseline, mut draft) = setup();
    let age = draft.add_column("age", "int");
    draft.move_column(age, 0);
    draft.move_column(existing(4), 1);
    assert_eq!(
        sql(&baseline, &draft, &PlanOptions::default()),
        vec![
            "ALTER TABLE `app`.`users` ADD COLUMN `age` int NULL FIRST, \
             MODIFY COLUMN `org_id` bigint NULL AFTER `age`"
        ]
    );
}

#[test]
fn online_mode_asks_for_instant_or_inplace() {
    let (baseline, mut draft) = setup();
    draft.add_column("age", "int");
    let preview = plan_review(&baseline, &draft, &online()).unwrap();
    assert_eq!(
        preview.statements[0].sql,
        "ALTER TABLE `app`.`users` ADD COLUMN `age` int NULL, ALGORITHM=INSTANT"
    );
    assert!(kinds(&preview).contains(&NoteKind::Online));
    assert!(!kinds(&preview).contains(&NoteKind::Lock));

    let (baseline, mut draft) = setup();
    draft.table.column_mut(existing(2)).unwrap().data_type = "varchar(200)".into();
    assert_eq!(
        sql(&baseline, &draft, &online()),
        vec![
            "ALTER TABLE `app`.`users` MODIFY COLUMN `email` varchar(200) NOT NULL, \
             ALGORITHM=INPLACE, LOCK=NONE"
        ]
    );

    // Instant only where the server version supports it.
    let (mut baseline, mut draft) = setup();
    baseline.server.features.instant_add_column = false;
    draft.add_column("age", "int");
    assert_eq!(
        sql(&baseline, &draft, &online()),
        vec!["ALTER TABLE `app`.`users` ADD COLUMN `age` int NULL, ALGORITHM=INPLACE, LOCK=NONE"]
    );

    // MariaDB never gets INSTANT from this planner.
    let (mut baseline, mut draft) = setup();
    baseline.server.flavor = ServerFlavor::MariaDb;
    draft.add_column("age", "int");
    assert_eq!(
        sql(&baseline, &draft, &online()),
        vec!["ALTER TABLE `app`.`users` ADD COLUMN `age` int NULL, ALGORITHM=INPLACE, LOCK=NONE"]
    );
}

#[test]
fn table_rename_and_comment() {
    let (baseline, mut draft) = setup();
    draft.table.name = "people".into();
    draft.table.comment = Some("Everyone's record".into());
    assert_eq!(
        sql(&baseline, &draft, &PlanOptions::default()),
        vec!["ALTER TABLE `app`.`users` COMMENT = 'Everyone''s record', RENAME TO `people`"]
    );

    // Foreign keys are added after the rename, under the new name.
    draft.table.foreign_keys[0].on_delete = ReferentialAction::SetNull;
    let statements = sql(&baseline, &draft, &PlanOptions::default());
    assert_eq!(statements.len(), 3);
    assert_eq!(
        statements[0],
        "ALTER TABLE `app`.`users` DROP FOREIGN KEY `users_org_id_fkey`"
    );
    assert!(statements[2].starts_with("ALTER TABLE `app`.`people` ADD CONSTRAINT"));
}

#[test]
fn checks_drop_by_flavor_and_rename_by_recreation() {
    let (baseline, mut draft) = setup();
    draft.table.checks.clear();
    assert_eq!(
        sql(&baseline, &draft, &PlanOptions::default()),
        vec!["ALTER TABLE `app`.`users` DROP CHECK `users_email_check`"]
    );
    let mut mariadb = baseline.clone();
    mariadb.server.flavor = ServerFlavor::MariaDb;
    assert_eq!(
        sql(&mariadb, &draft, &PlanOptions::default()),
        vec!["ALTER TABLE `app`.`users` DROP CONSTRAINT `users_email_check`"]
    );

    let (baseline, mut draft) = setup();
    draft.table.checks[0].name = Some("email_len".into());
    assert_eq!(
        sql(&baseline, &draft, &PlanOptions::default()),
        vec![
            "ALTER TABLE `app`.`users` DROP CHECK `users_email_check`, \
             ADD CONSTRAINT `email_len` CHECK (length(email) > 3)"
        ]
    );

    let (baseline, mut draft) = setup();
    draft.table.checks[0].enforced = false;
    assert_eq!(
        sql(&baseline, &draft, &PlanOptions::default()),
        vec![
            "ALTER TABLE `app`.`users` DROP CHECK `users_email_check`, \
             ADD CONSTRAINT `users_email_check` CHECK (length(email) > 3) NOT ENFORCED"
        ]
    );
}

#[test]
fn indexes_and_keys() {
    let (baseline, mut draft) = setup();
    draft.table.indexes[0].members[0].prefix_length = Some(10);
    draft.table.indexes[0].members[0].descending = true;
    assert_eq!(
        sql(&baseline, &draft, &PlanOptions::default()),
        vec![
            "ALTER TABLE `app`.`users` DROP INDEX `users_name_idx`, \
             ADD INDEX `users_name_idx` (`name`(10) DESC)"
        ]
    );

    let (baseline, mut draft) = setup();
    draft.table.indexes[0].name = "users_by_name".into();
    draft.table.uniques[0].name = Some("users_email_uq".into());
    assert_eq!(
        sql(&baseline, &draft, &PlanOptions::default()),
        vec![
            "ALTER TABLE `app`.`users` RENAME INDEX `users_email_key` TO `users_email_uq`, \
             RENAME INDEX `users_name_idx` TO `users_by_name`"
        ]
    );

    let (baseline, mut draft) = setup();
    let origin = draft.new_origin();
    draft.table.indexes.push(IndexModel {
        origin,
        name: "users_lower_name".into(),
        unique: true,
        method: None,
        members: vec![IndexMember {
            target: IndexTarget::Expression(SqlFragment::new("lower(`name`)")),
            descending: false,
            prefix_length: None,
            collation: None,
        }],
        include: Vec::new(),
        predicate: None,
    });
    let key = draft.new_origin();
    draft.table.uniques.push(KeyModel {
        origin: key,
        name: Some("users_org_name_key".into()),
        columns: vec![existing(4), existing(3)],
    });
    assert_eq!(
        sql(&baseline, &draft, &PlanOptions::default()),
        vec![
            "ALTER TABLE `app`.`users` ADD UNIQUE INDEX `users_org_name_key` (`org_id`, `name`), \
             ADD UNIQUE INDEX `users_lower_name` ((lower(`name`)))"
        ]
    );
}

#[test]
fn literals_are_escaped_for_mysql() {
    let (baseline, mut draft) = setup();
    let note = draft.add_column("note", "varchar(40)");
    draft.table.column_mut(note).unwrap().default = DefaultValue::Literal("it's \\ ok".into());
    draft.table.column_mut(note).unwrap().comment = Some("a 'b'".into());
    assert_eq!(
        sql(&baseline, &draft, &PlanOptions::default()),
        vec![
            "ALTER TABLE `app`.`users` ADD COLUMN `note` varchar(40) NULL \
             DEFAULT 'it''s \\\\ ok' COMMENT 'a ''b'''"
        ]
    );
}

#[test]
fn expression_defaults_are_parenthesized() {
    let (baseline, mut draft) = setup();
    let token = draft.add_column("token", "char(36)");
    draft.table.column_mut(token).unwrap().default =
        DefaultValue::Expression(SqlFragment::new("uuid()"));
    let created = draft.add_column("created_at", "timestamp");
    draft.table.column_mut(created).unwrap().default =
        DefaultValue::Expression(SqlFragment::new("CURRENT_TIMESTAMP"));
    assert_eq!(
        sql(&baseline, &draft, &PlanOptions::default()),
        vec![
            "ALTER TABLE `app`.`users` ADD COLUMN `token` char(36) NULL DEFAULT (uuid()), \
             ADD COLUMN `created_at` timestamp NULL DEFAULT CURRENT_TIMESTAMP"
        ]
    );
}

#[test]
fn unsupported_changes_are_refused() {
    let (baseline, mut draft) = setup();
    draft.table.primary_key.as_mut().unwrap().name = Some("pk".into());
    assert!(matches!(
        plan_review(&baseline, &draft, &PlanOptions::default()),
        Err(PlanError::Unsupported { .. })
    ));

    let (baseline, mut draft) = setup();
    draft.table.indexes[0].predicate = Some(SqlFragment::new("name IS NOT NULL"));
    assert!(matches!(
        plan_review(&baseline, &draft, &PlanOptions::default()),
        Err(PlanError::Unsupported { .. })
    ));

    let (baseline, mut draft) = setup();
    draft.table.foreign_keys[0].deferrable = Deferrable::InitiallyDeferred;
    assert!(matches!(
        plan_review(&baseline, &draft, &PlanOptions::default()),
        Err(PlanError::Unsupported { .. })
    ));

    let (baseline, mut draft) = setup();
    let note = draft.add_column("note", "text");
    draft.table.column_mut(note).unwrap().collation = Some("utf8mb4_bin; DROP".into());
    assert!(plan_review(&baseline, &draft, &PlanOptions::default()).is_err());
}

#[test]
fn notes_cover_dependents_and_large_tables() {
    let (mut baseline, mut draft) = setup();
    baseline.estimated_rows = Some(5_000_000);
    baseline.inbound.push(InboundReference {
        schema: "app".into(),
        table: "orders".into(),
        name: "orders_user_fkey".into(),
        columns: vec!["id".into()],
    });
    draft.table.column_mut(existing(2)).unwrap().data_type = "varchar(10)".into();
    let preview = plan(&baseline, &draft);
    let kinds = kinds(&preview);
    assert!(kinds.contains(&NoteKind::Rewrite));
    assert!(kinds.contains(&NoteKind::LargeTable));
    assert!(kinds.contains(&NoteKind::Dependents));
    // Narrowing is destructive.
    assert!(preview.statements[0].destructive);
}

#[test]
fn planning_is_deterministic() {
    let (baseline, mut draft) = setup();
    draft.remove_column(existing(3));
    let age = draft.add_column("age", "int");
    draft.move_column(age, 1);
    draft.table.foreign_keys[0].on_update = ReferentialAction::Cascade;
    draft.table.comment = Some("People".into());
    draft.table.name = "people".into();
    let first = plan_review(&baseline, &draft, &online()).unwrap();
    for _ in 0..3 {
        assert_eq!(plan_review(&baseline, &draft, &online()).unwrap(), first);
    }
    assert!(first.check().is_ok());
}
