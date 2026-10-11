//! Capture: version gate, `SHOW CREATE TABLE` parsing, the catalog and
//! definition cross-check, ids, drift and post-apply verification. Pure, on
//! fixture rows; live round trips belong to the native fixture suite.
use super::build::{self, object_id, CHECK_TAG, FOREIGN_KEY_TAG, INDEX_TAG, UNIQUE_TAG};
use super::catalog::{
    Catalog, CheckRow, ColumnRow, ConstraintRow, IndexRow, KeyColumnRow, ReferentialRow,
    TableRow,
};
use super::definition::{self, DefDefault};
use super::executor::verify;
use super::version::Version;
use crate::schema_edit::fixtures::{col, existing, users};
use crate::schema_edit::*;

fn column(
    name: &str,
    ordinal: u32,
    column_type: &str,
    nullable: bool,
    default: Option<&str>,
    extra: &str,
) -> ColumnRow {
    ColumnRow {
        name: name.into(),
        ordinal,
        default: default.map(str::to_owned),
        nullable,
        column_type: column_type.into(),
        charset: None,
        collation: None,
        extra: extra.into(),
        comment: String::new(),
        generation: Some(String::new()),
    }
}

fn text_column(mut row: ColumnRow, charset: &str, collation: &str) -> ColumnRow {
    row.charset = Some(charset.into());
    row.collation = Some(collation.into());
    row
}

fn index(name: &str, unique: bool, column: Option<&str>) -> IndexRow {
    IndexRow {
        index: name.into(),
        non_unique: !unique,
        column: column.map(str::to_owned),
        collation: Some("A".into()),
        sub_part: None,
        index_type: "BTREE".into(),
        expression: None,
        comment: String::new(),
        visible: true,
    }
}

fn constraint(name: &str, kind: &str, enforced: bool) -> ConstraintRow {
    ConstraintRow {
        name: name.into(),
        kind: kind.into(),
        enforced,
    }
}

const MYSQL_DEFINITION: &str = "CREATE TABLE `users` (
  `id` bigint unsigned NOT NULL AUTO_INCREMENT,
  `email` varchar(100) COLLATE utf8mb4_bin NOT NULL COMMENT 'Login',
  `name` text,
  `org_id` bigint unsigned DEFAULT NULL,
  `status` enum('active','disabled') NOT NULL DEFAULT 'active',
  `created_at` timestamp NOT NULL DEFAULT CURRENT_TIMESTAMP,
  `updated_at` timestamp NULL DEFAULT CURRENT_TIMESTAMP ON UPDATE CURRENT_TIMESTAMP,
  `token` char(36) NOT NULL DEFAULT (uuid()) /*!80023 INVISIBLE */,
  `display` varchar(201) GENERATED ALWAYS AS (concat(`name`,_utf8mb4' <',`email`,_utf8mb4'>')) VIRTUAL,
  PRIMARY KEY (`id`),
  UNIQUE KEY `users_email_key` (`email`),
  KEY `users_name_idx` (`name`(20)),
  KEY `users_org_id_fkey` (`org_id`),
  KEY `users_lower_email` ((lower(`email`))),
  CONSTRAINT `users_org_id_fkey` FOREIGN KEY (`org_id`) REFERENCES `orgs` (`id`) ON DELETE CASCADE,
  CONSTRAINT `users_email_check` CHECK ((length(`email`) > 3)),
  CONSTRAINT `users_soft` CHECK ((`id` > 0)) /*!80016 NOT ENFORCED */
) ENGINE=InnoDB AUTO_INCREMENT=42 DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_0900_ai_ci ROW_FORMAT=DYNAMIC COMMENT='People'";

/// MySQL 8.0 catalog rows matching [`MYSQL_DEFINITION`].
fn mysql_catalog() -> Catalog {
    let general = "utf8mb4_0900_ai_ci";
    let mut email = text_column(
        column("email", 2, "varchar(100)", false, None, ""),
        "utf8mb4",
        "utf8mb4_bin",
    );
    email.comment = "Login".into();
    let mut display = text_column(
        column("display", 9, "varchar(201)", true, None, "VIRTUAL GENERATED"),
        "utf8mb4",
        general,
    );
    display.generation = Some("concat(`name`,_utf8mb4\\' <\\',`email`,_utf8mb4\\'>\\')".into());
    let mut prefix = index("users_name_idx", false, Some("name"));
    prefix.sub_part = Some(20);
    let mut functional = index("users_lower_email", false, None);
    functional.expression = Some("lower(`email`)".into());
    Catalog {
        sql_mode: "ONLY_FULL_GROUP_BY,STRICT_TRANS_TABLES,NO_ENGINE_SUBSTITUTION".into(),
        table: TableRow {
            table_type: "BASE TABLE".into(),
            engine: Some("InnoDB".into()),
            collation: Some(general.into()),
            charset: Some("utf8mb4".into()),
            comment: "People".into(),
            rows: Some(10),
            create_options: "row_format=DYNAMIC".into(),
        },
        columns: vec![
            column("id", 1, "bigint unsigned", false, None, "auto_increment"),
            email,
            text_column(column("name", 3, "text", true, None, ""), "utf8mb4", general),
            column("org_id", 4, "bigint unsigned", true, None, ""),
            text_column(
                column(
                    "status",
                    5,
                    "enum('active','disabled')",
                    false,
                    Some("active"),
                    "",
                ),
                "utf8mb4",
                general,
            ),
            column(
                "created_at",
                6,
                "timestamp",
                false,
                Some("CURRENT_TIMESTAMP"),
                "DEFAULT_GENERATED",
            ),
            column(
                "updated_at",
                7,
                "timestamp",
                true,
                Some("CURRENT_TIMESTAMP"),
                "DEFAULT_GENERATED on update CURRENT_TIMESTAMP",
            ),
            text_column(
                column(
                    "token",
                    8,
                    "char(36)",
                    false,
                    Some("uuid()"),
                    "DEFAULT_GENERATED INVISIBLE",
                ),
                "utf8mb4",
                general,
            ),
            display,
        ],
        indexes: vec![
            index("PRIMARY", true, Some("id")),
            index("users_email_key", true, Some("email")),
            functional,
            prefix,
            index("users_org_id_fkey", false, Some("org_id")),
        ],
        constraints: vec![
            constraint("users_email_check", "CHECK", true),
            constraint("users_soft", "CHECK", false),
            constraint("users_org_id_fkey", "FOREIGN KEY", true),
            constraint("PRIMARY", "PRIMARY KEY", true),
            constraint("users_email_key", "UNIQUE", true),
        ],
        key_columns: vec![KeyColumnRow {
            constraint: "users_org_id_fkey".into(),
            column: "org_id".into(),
            referenced_schema: Some("app".into()),
            referenced_table: Some("orgs".into()),
            referenced_column: Some("id".into()),
        }],
        referential: vec![ReferentialRow {
            constraint: "users_org_id_fkey".into(),
            update_rule: "NO ACTION".into(),
            delete_rule: "CASCADE".into(),
        }],
        checks: vec![
            CheckRow {
                name: "users_email_check".into(),
                clause: "(length(`email`) > 3)".into(),
                level: "Table".into(),
            },
            CheckRow {
                name: "users_soft".into(),
                clause: "(`id` > 0)".into(),
                level: "Table".into(),
            },
        ],
        inbound: Vec::new(),
        triggers: vec!["users_audit".into()],
        views: Vec::new(),
        create_table: MYSQL_DEFINITION.into(),
        truncated: false,
    }
}

const MARIADB_DEFINITION: &str = "CREATE TABLE `users` (
  `id` bigint(20) unsigned NOT NULL AUTO_INCREMENT,
  `email` varchar(100) NOT NULL,
  `score` int(11) NOT NULL DEFAULT 0,
  `created_at` timestamp NOT NULL DEFAULT current_timestamp() ON UPDATE current_timestamp(),
  `note` varchar(20) DEFAULT 'it''s',
  PRIMARY KEY (`id`),
  UNIQUE KEY `users_email_key` (`email`),
  CONSTRAINT `users_score` CHECK (`score` >= 0)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_general_ci";

/// MariaDB 10.6 catalog rows matching [`MARIADB_DEFINITION`].
fn mariadb_catalog() -> Catalog {
    let general = "utf8mb4_general_ci";
    Catalog {
        sql_mode: "STRICT_TRANS_TABLES,ERROR_FOR_DIVISION_BY_ZERO,NO_AUTO_CREATE_USER".into(),
        table: TableRow {
            table_type: "BASE TABLE".into(),
            engine: Some("InnoDB".into()),
            collation: Some(general.into()),
            charset: Some("utf8mb4".into()),
            comment: String::new(),
            rows: Some(3),
            create_options: String::new(),
        },
        columns: vec![
            column("id", 1, "bigint(20) unsigned", false, None, "auto_increment"),
            text_column(
                column("email", 2, "varchar(100)", false, None, ""),
                "utf8mb4",
                general,
            ),
            column("score", 3, "int(11)", false, Some("0"), ""),
            column(
                "created_at",
                4,
                "timestamp",
                false,
                Some("current_timestamp()"),
                "on update current_timestamp()",
            ),
            text_column(
                column("note", 5, "varchar(20)", true, Some("'it''s'"), ""),
                "utf8mb4",
                general,
            ),
        ],
        indexes: vec![
            index("PRIMARY", true, Some("id")),
            index("users_email_key", true, Some("email")),
        ],
        constraints: vec![
            constraint("users_score", "CHECK", true),
            constraint("PRIMARY", "PRIMARY KEY", true),
            constraint("users_email_key", "UNIQUE", true),
        ],
        checks: vec![CheckRow {
            name: "users_score".into(),
            clause: "`score` >= 0".into(),
            level: "Table".into(),
        }],
        create_table: MARIADB_DEFINITION.into(),
        ..Catalog::default()
    }
}

fn reasons(baseline: &SchemaBaseline) -> Vec<String> {
    match &baseline.editable {
        Editability::Editable => Vec::new(),
        Editability::ReadOnly { reasons } => reasons.clone(),
    }
}

fn has_reason(baseline: &SchemaBaseline, needle: &str) -> bool {
    reasons(baseline).iter().any(|reason| reason.contains(needle))
}

#[test]
fn versions_parse_and_gate() {
    let mysql = Version::parse("8.0.36").unwrap();
    assert_eq!(mysql.flavor, ServerFlavor::MySql);
    assert_eq!((mysql.major, mysql.minor, mysql.patch), (8, 0, 36));
    assert_eq!(mysql.gate(), None);
    let commercial = Version::parse("8.4.0-commercial").unwrap();
    assert_eq!((commercial.major, commercial.minor), (8, 4));
    let mariadb = Version::parse("10.6.12-MariaDB-1:10.6.12+maria~ubu2004").unwrap();
    assert_eq!(mariadb.flavor, ServerFlavor::MariaDb);
    assert_eq!((mariadb.major, mariadb.minor, mariadb.patch), (10, 6, 12));
    assert_eq!(mariadb.gate(), None);
    let prefixed = Version::parse("5.5.5-10.11.6-MariaDB").unwrap();
    assert_eq!((prefixed.flavor, prefixed.major), (ServerFlavor::MariaDb, 10));
    assert_eq!(Version::parse("garbage"), None);

    assert!(Version::parse("5.7.44").unwrap().gate().is_some());
    assert!(Version::parse("10.5.21-MariaDB").unwrap().gate().is_some());
    assert!(Version::parse("8.0.0").unwrap().gate().is_none());
}

#[test]
fn features_follow_the_version() {
    let features = |text: &str| Version::parse(text).unwrap().features();
    assert!(!features("8.0.11").instant_add_column);
    assert!(features("8.0.12").instant_add_column);
    assert!(!features("8.0.28").instant_add_column_anywhere);
    assert!(features("8.0.29").instant_add_column_anywhere);
    assert!(!features("8.0.15").enforced_check);
    assert!(features("8.0.16").enforced_check);
    assert!(features("8.0.36").rename_column);
    let mariadb = features("10.6.12-MariaDB");
    assert!(mariadb.rename_column && mariadb.enforced_check && mariadb.column_reorder);
    assert!(!mariadb.instant_add_column && !mariadb.transactional_ddl);
}

#[test]
fn old_or_unknown_servers_are_read_only() {
    let old = build::baseline("5.7.44-log", "app", "users", &mysql_catalog());
    assert!(has_reason(&old, "older than 8.0"), "{:?}", reasons(&old));
    let old = build::baseline("10.5.21-MariaDB", "app", "users", &mariadb_catalog());
    assert!(has_reason(&old, "older than 10.6"), "{:?}", reasons(&old));
    let unknown = build::baseline("???", "app", "users", &mariadb_catalog());
    assert!(has_reason(&unknown, "not recognised"));
}

#[test]
fn literal_and_expression_defaults_are_told_apart() {
    let parsed = definition::parse(
        "CREATE TABLE `t` (
  `a` int DEFAULT '5',
  `b` int DEFAULT 0,
  `c` datetime DEFAULT CURRENT_TIMESTAMP,
  `d` char(36) DEFAULT (uuid()),
  `e` int DEFAULT NULL,
  `f` bit(1) DEFAULT b'1',
  `g` timestamp NULL DEFAULT current_timestamp() ON UPDATE current_timestamp(),
  `h` int DEFAULT -1,
  `i` varchar(10) DEFAULT 'it''s \\\\ x' COMMENT 'c'
) ENGINE=InnoDB",
    )
    .unwrap();
    let defaults: Vec<Option<DefDefault>> =
        parsed.columns.iter().map(|c| c.default.clone()).collect();
    assert_eq!(
        defaults,
        vec![
            Some(DefDefault::String("5".into())),
            Some(DefDefault::Number("0".into())),
            Some(DefDefault::Expression("CURRENT_TIMESTAMP".into())),
            Some(DefDefault::Expression("(uuid())".into())),
            Some(DefDefault::Null),
            Some(DefDefault::Bits("b'1'".into())),
            Some(DefDefault::Expression("current_timestamp()".into())),
            Some(DefDefault::Number("-1".into())),
            Some(DefDefault::String("it's \\ x".into())),
        ]
    );
    assert_eq!(parsed.columns[6].on_update.as_deref(), Some("current_timestamp()"));
    assert_eq!(parsed.columns[8].comment.as_deref(), Some("c"));
    assert!(parsed.unsupported.is_empty(), "{:?}", parsed.unsupported);

    let model: Vec<DefaultValue> = defaults
        .iter()
        .map(build::default_from_definition)
        .collect();
    assert_eq!(model[0], DefaultValue::Literal("5".into()));
    assert_eq!(model[1], DefaultValue::Literal("0".into()));
    assert_eq!(
        model[2],
        DefaultValue::Expression(SqlFragment::new("CURRENT_TIMESTAMP"))
    );
    assert_eq!(model[3], DefaultValue::Expression(SqlFragment::new("(uuid())")));
    assert_eq!(model[4], DefaultValue::None);
}

#[test]
fn show_create_parses_keys_constraints_and_options() {
    let parsed = definition::parse(MYSQL_DEFINITION).unwrap();
    assert!(parsed.unsupported.is_empty(), "{:?}", parsed.unsupported);
    assert_eq!(parsed.name, "users");
    assert_eq!(parsed.columns.len(), 9);
    let token = &parsed.columns[7];
    assert!(token.invisible);
    assert_eq!(token.not_null, Some(true));
    let display = &parsed.columns[8];
    assert_eq!(
        display.generated,
        Some((
            "concat(`name`,_utf8mb4' <',`email`,_utf8mb4'>')".into(),
            false
        ))
    );
    assert_eq!(parsed.columns[1].collation.as_deref(), Some("utf8mb4_bin"));
    let prefix = parsed
        .keys
        .iter()
        .find(|k| k.name.as_deref() == Some("users_name_idx"))
        .unwrap();
    assert_eq!(prefix.parts[0].prefix, Some(20));
    let functional = parsed
        .keys
        .iter()
        .find(|k| k.name.as_deref() == Some("users_lower_email"))
        .unwrap();
    assert_eq!(functional.parts[0].expression.as_deref(), Some("lower(`email`)"));
    assert_eq!(parsed.foreign_keys[0].on_delete.as_deref(), Some("CASCADE"));
    assert_eq!(parsed.foreign_keys[0].on_update, None);
    assert_eq!(parsed.checks[0].expression, "(length(`email`) > 3)");
    assert!(parsed.checks[0].enforced);
    assert!(!parsed.checks[1].enforced);
    assert_eq!(parsed.option("ENGINE"), Some("InnoDB"));
    assert_eq!(parsed.option("CHARSET"), Some("utf8mb4"));
    assert_eq!(parsed.option("ROW_FORMAT"), Some("DYNAMIC"));
    assert_eq!(parsed.option("COMMENT"), Some("People"));
    assert!(definition::cross_check(MYSQL_DEFINITION, &parsed).is_empty());
}

#[test]
fn mysql_capture_is_editable_and_complete() {
    let baseline = build::baseline("8.0.36", "app", "users", &mysql_catalog());
    assert_eq!(baseline.editable, Editability::Editable, "{:?}", reasons(&baseline));
    assert_eq!(baseline.server.flavor, ServerFlavor::MySql);
    let table = &baseline.table;
    assert_eq!(table.columns.len(), 9);
    let id = &table.columns[0];
    assert_eq!(id.origin, existing(1));
    assert_eq!(id.identity, Identity::AutoIncrement);
    let email = &table.columns[1];
    assert_eq!(email.collation.as_deref(), Some("utf8mb4_bin"));
    assert_eq!(email.extra.charset.as_deref(), Some("utf8mb4"));
    assert_eq!(email.comment.as_deref(), Some("Login"));
    assert_eq!(table.columns[4].default, DefaultValue::Literal("active".into()));
    assert_eq!(
        table.columns[5].default,
        DefaultValue::Expression(SqlFragment::new("CURRENT_TIMESTAMP"))
    );
    assert_eq!(
        table.columns[6].extra.on_update,
        Some(SqlFragment::new("CURRENT_TIMESTAMP"))
    );
    let token = &table.columns[7];
    assert!(token.extra.invisible);
    assert_eq!(token.default, DefaultValue::Expression(SqlFragment::new("(uuid())")));
    let display = &table.columns[8];
    assert_eq!(
        display.generated,
        Some(Generated {
            expression: SqlFragment::new("concat(`name`,_utf8mb4' <',`email`,_utf8mb4'>')"),
            stored: false,
        })
    );

    assert_eq!(table.primary_key.as_ref().unwrap().columns, vec![existing(1)]);
    assert_eq!(table.uniques.len(), 1);
    assert_eq!(table.uniques[0].origin, object_id(UNIQUE_TAG, 1));
    assert_eq!(table.uniques[0].columns, vec![existing(2)]);
    let names: Vec<&str> = table.indexes.iter().map(|i| i.name.as_str()).collect();
    assert_eq!(
        names,
        vec!["users_lower_email", "users_name_idx", "users_org_id_fkey"]
    );
    assert_eq!(table.indexes[0].origin, object_id(INDEX_TAG, 1));
    assert_eq!(
        table.indexes[0].members[0].target,
        IndexTarget::Expression(SqlFragment::new("lower(`email`)"))
    );
    assert_eq!(table.indexes[1].members[0].prefix_length, Some(20));

    let key = &table.foreign_keys[0];
    assert_eq!(key.origin, object_id(FOREIGN_KEY_TAG, 1));
    assert_eq!(key.referenced_schema, None);
    assert_eq!(key.on_delete, ReferentialAction::Cascade);
    assert_eq!(key.on_update, ReferentialAction::NoAction);
    assert_eq!(table.checks[0].origin, object_id(CHECK_TAG, 1));
    assert_eq!(
        table.checks[0].expression,
        SqlFragment::new("(length(`email`) > 3)")
    );
    assert!(table.checks[0].enforced);
    assert!(!table.checks[1].enforced);

    assert_eq!(table.options.engine.as_deref(), Some("InnoDB"));
    assert_eq!(table.options.charset.as_deref(), Some("utf8mb4"));
    assert_eq!(table.options.collation.as_deref(), Some("utf8mb4_0900_ai_ci"));
    assert_eq!(table.options.row_format.as_deref(), Some("DYNAMIC"));
    assert_eq!(table.comment.as_deref(), Some("People"));
    assert_eq!(baseline.estimated_rows, Some(10));
    assert_eq!(baseline.dependents[0].name, "users_audit");
}

#[test]
fn mariadb_capture_is_editable() {
    let baseline = build::baseline(
        "10.6.12-MariaDB-1:10.6.12+maria~ubu2004",
        "app",
        "users",
        &mariadb_catalog(),
    );
    assert_eq!(baseline.editable, Editability::Editable, "{:?}", reasons(&baseline));
    assert_eq!(baseline.server.flavor, ServerFlavor::MariaDb);
    assert!(!baseline.server.features.instant_add_column);
    let table = &baseline.table;
    assert_eq!(table.columns[0].data_type, "bigint(20) unsigned");
    assert_eq!(table.columns[2].default, DefaultValue::Literal("0".into()));
    assert_eq!(
        table.columns[3].default,
        DefaultValue::Expression(SqlFragment::new("current_timestamp()"))
    );
    assert_eq!(
        table.columns[3].extra.on_update,
        Some(SqlFragment::new("current_timestamp()"))
    );
    assert_eq!(table.columns[4].default, DefaultValue::Literal("it's".into()));
    assert_eq!(table.checks[0].expression, SqlFragment::new("`score` >= 0"));
    assert_eq!(table.options.row_format, None);
}

#[test]
fn disagreements_and_unsupported_features_make_the_baseline_read_only() {
    // The catalog says nullable; the definition says NOT NULL.
    let mut catalog = mysql_catalog();
    catalog.columns[1].nullable = true;
    let baseline = build::baseline("8.0.36", "app", "users", &catalog);
    assert!(
        has_reason(&baseline, "nullability of column email"),
        "{:?}",
        reasons(&baseline)
    );

    // A default the definition does not show.
    let mut catalog = mysql_catalog();
    catalog.columns[3].default = Some("7".into());
    let baseline = build::baseline("8.0.36", "app", "users", &catalog);
    assert!(has_reason(&baseline, "default of column org_id"));

    // Partitioning.
    let mut catalog = mysql_catalog();
    catalog.create_table = format!(
        "{MYSQL_DEFINITION}\n/*!50100 PARTITION BY HASH (`id`)\nPARTITIONS 4 */"
    );
    catalog.table.create_options = "partitioned".into();
    let baseline = build::baseline("8.0.36", "app", "users", &catalog);
    assert!(has_reason(&baseline, "Partitioned"));

    // An index comment cannot be restated.
    let mut catalog = mysql_catalog();
    catalog.create_table = MYSQL_DEFINITION.replace(
        "KEY `users_name_idx` (`name`(20)),",
        "KEY `users_name_idx` (`name`(20)) COMMENT 'lookup',",
    );
    catalog.indexes[3].comment = "lookup".into();
    let baseline = build::baseline("8.0.36", "app", "users", &catalog);
    assert!(has_reason(&baseline, "has a comment"));

    // MariaDB column-level CHECK (as for JSON columns).
    let mut catalog = mariadb_catalog();
    catalog.create_table = MARIADB_DEFINITION.replace(
        "  PRIMARY KEY (`id`),",
        "  `doc` longtext CHARACTER SET utf8mb4 COLLATE utf8mb4_bin DEFAULT NULL CHECK (json_valid(`doc`)),\n  PRIMARY KEY (`id`),",
    );
    catalog.columns.push(text_column(
        column("doc", 6, "longtext", true, Some("NULL"), ""),
        "utf8mb4",
        "utf8mb4_bin",
    ));
    catalog.checks.push(CheckRow {
        name: "doc".into(),
        clause: "json_valid(`doc`)".into(),
        level: "Column".into(),
    });
    let baseline = build::baseline("10.6.12-MariaDB", "app", "users", &catalog);
    assert!(has_reason(&baseline, "Column-level CHECK"), "{:?}", reasons(&baseline));

    // Unparseable definition.
    let mut catalog = mysql_catalog();
    catalog.create_table = "CREATE VIEW `users` AS SELECT 1".into();
    let baseline = build::baseline("8.0.36", "app", "users", &catalog);
    assert!(has_reason(&baseline, "could not be read"));

    // A view is not a base table.
    let mut catalog = mysql_catalog();
    catalog.table.table_type = "VIEW".into();
    catalog.create_table.clear();
    let baseline = build::baseline("8.0.36", "app", "users", &catalog);
    assert!(has_reason(&baseline, "Only base tables"));

    // The session sql_mode changes how definitions render.
    let mut catalog = mysql_catalog();
    catalog.sql_mode = "ANSI_QUOTES,STRICT_TRANS_TABLES".into();
    let baseline = build::baseline("8.0.36", "app", "users", &catalog);
    assert!(has_reason(&baseline, "ANSI_QUOTES"));
}

#[test]
fn drift_token_tracks_structure_not_row_counts() {
    let catalog = mysql_catalog();
    let first = build::baseline("8.0.36", "app", "users", &catalog);
    let again = build::baseline("8.0.36", "app", "users", &catalog);
    assert_eq!(first.drift, again.drift);
    assert_eq!(first, again);

    let mut rows = mysql_catalog();
    rows.table.rows = Some(1_000_000);
    rows.create_table = rows.create_table.replace("AUTO_INCREMENT=42", "AUTO_INCREMENT=99");
    assert_eq!(build::baseline("8.0.36", "app", "users", &rows).drift, first.drift);

    let mut changed = mysql_catalog();
    changed.columns[1].comment = "Sign-in".into();
    changed.create_table = changed.create_table.replace("'Login'", "'Sign-in'");
    let changed = build::baseline("8.0.36", "app", "users", &changed);
    assert_eq!(changed.editable, Editability::Editable, "{:?}", reasons(&changed));
    assert_ne!(changed.drift, first.drift);
}

#[test]
fn sql_mode_is_cleaned_for_the_apply_connection() {
    assert_eq!(
        build::safe_sql_mode("ANSI_QUOTES,STRICT_TRANS_TABLES,NO_BACKSLASH_ESCAPES"),
        "STRICT_TRANS_TABLES"
    );
    assert_eq!(build::safe_sql_mode(""), "");
}

#[test]
fn verification_accepts_server_normalization() {
    let baseline = users(SchemaEngine::MySql);
    let mut draft = SchemaDraft::from_baseline(&baseline);
    draft.add_column("age", "INTEGER");
    draft.add_column("flag", "bool");
    let nick = draft.add_column("nick", "varchar(20)");
    // Edited check text comes back in the server's rendering.
    draft.table.checks[0].expression = SqlFragment::new("length(email) > 5");

    let mut captured = baseline.clone();
    captured.table.columns.push(col(5, "age", "int(11)"));
    captured.table.columns.push(col(6, "flag", "tinyint(1)"));
    let mut nick_column = col(7, "nick", "varchar(20)");
    nick_column.extra.charset = Some("utf8mb4".into());
    nick_column.collation = Some("utf8mb4_0900_ai_ci".into());
    captured.table.columns.push(nick_column);
    captured.table.checks[0].expression = SqlFragment::new("(length(`email`) > 5)");
    assert_eq!(verify(&baseline, &draft, &captured), Vec::<String>::new());

    // A draft charset is checked when given.
    draft.table.column_mut(nick).unwrap().extra.charset = Some("latin1".into());
    assert!(!verify(&baseline, &draft, &captured).is_empty());
}

#[test]
fn verification_expects_the_implicit_foreign_key_index() {
    let baseline = users(SchemaEngine::MySql);
    let mut draft = SchemaDraft::from_baseline(&baseline);
    let manager = draft.add_column("manager_id", "bigint");
    let key = draft.new_origin();
    draft.table.foreign_keys.push(ForeignKeyModel {
        origin: key,
        name: Some("users_manager_fkey".into()),
        columns: vec![manager],
        referenced_schema: None,
        referenced_table: "users".into(),
        referenced_columns: vec!["id".into()],
        on_update: ReferentialAction::NoAction,
        on_delete: ReferentialAction::SetNull,
        deferrable: Deferrable::Never,
    });

    let mut captured = baseline.clone();
    captured.table.columns.push(col(5, "manager_id", "bigint"));
    captured.table.foreign_keys.push(ForeignKeyModel {
        origin: existing(301),
        name: Some("users_manager_fkey".into()),
        columns: vec![existing(5)],
        referenced_schema: None,
        referenced_table: "users".into(),
        referenced_columns: vec!["id".into()],
        on_update: ReferentialAction::Restrict,
        on_delete: ReferentialAction::SetNull,
        deferrable: Deferrable::Never,
    });
    captured.table.indexes.push(IndexModel {
        origin: existing(501),
        name: "users_manager_fkey".into(),
        unique: false,
        method: None,
        members: vec![IndexMember::column(existing(5))],
        include: Vec::new(),
        predicate: None,
    });
    assert_eq!(verify(&baseline, &draft, &captured), Vec::<String>::new());

    // Any other extra index is a mismatch.
    captured.table.indexes[1].name = "surprise".into();
    assert!(!verify(&baseline, &draft, &captured).is_empty());
}

#[test]
fn verification_reports_structural_mismatches() {
    let baseline = users(SchemaEngine::MySql);
    let draft = SchemaDraft::from_baseline(&baseline);
    let mut captured = baseline.clone();
    captured.table.columns[1].nullable = true;
    captured.table.uniques.clear();
    let problems = verify(&baseline, &draft, &captured);
    assert!(problems.iter().any(|p| p.contains("email")), "{problems:?}");
    assert!(problems.iter().any(|p| p.contains("users_email_key")), "{problems:?}");

    let mut captured = baseline.clone();
    captured.table.columns.swap(1, 2);
    assert!(!verify(&baseline, &draft, &captured).is_empty());
}
