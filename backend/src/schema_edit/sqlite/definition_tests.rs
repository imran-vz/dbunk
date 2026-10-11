use super::*;

fn table(sql: &str) -> ParsedTable {
    let parsed = parse_create_table(sql).unwrap_or_else(|error| panic!("{sql}: {error}"));
    cross_check(&parsed, sql).unwrap_or_else(|error| panic!("{sql}: {error}"));
    parsed
}

#[test]
fn columns_keep_exact_type_default_check_and_generated_text() {
    let sql = "CREATE TABLE \"Users\" (\n  id INTEGER PRIMARY KEY AUTOINCREMENT,\n  \
               email varchar ( 100 ) NOT NULL COLLATE NOCASE UNIQUE,\n  \
               score REAL DEFAULT ( 0.5 *  2 ) CHECK ( score >= 0 ),\n  \
               neg INT DEFAULT - 1,\n  \
               label TEXT DEFAULT 'it''s',\n  \
               made TEXT DEFAULT CURRENT_TIMESTAMP,\n  \
               up TEXT GENERATED ALWAYS AS (upper(label)) STORED,\n  \
               short AS (substr(label, 1, 2)),\n  \
               untyped\n)";
    let parsed = table(sql);
    assert_eq!(parsed.name, "Users");
    let names: Vec<&str> = parsed.columns.iter().map(|c| c.name.as_str()).collect();
    assert_eq!(
        names,
        vec!["id", "email", "score", "neg", "label", "made", "up", "short", "untyped"]
    );
    let id = &parsed.columns[0];
    assert_eq!(id.data_type, "INTEGER");
    assert_eq!(
        id.primary_key,
        Some(ColumnPrimaryKey {
            name: None,
            autoincrement: true
        })
    );
    let email = &parsed.columns[1];
    assert_eq!(email.data_type, "varchar ( 100 )");
    assert!(email.not_null);
    assert_eq!(email.collation.as_deref(), Some("NOCASE"));
    assert_eq!(email.uniques, vec![None]);
    let score = &parsed.columns[2];
    // As `PRAGMA table_info` shows it: the inside of the parentheses, trimmed.
    assert_eq!(score.default.as_deref(), Some("0.5 *  2"));
    assert_eq!(score.checks, vec![(None, "score >= 0".to_owned())]);
    assert_eq!(parsed.columns[3].default.as_deref(), Some("- 1"));
    assert_eq!(parsed.columns[4].default.as_deref(), Some("'it''s'"));
    assert_eq!(
        parsed.columns[5].default.as_deref(),
        Some("CURRENT_TIMESTAMP")
    );
    let up = parsed.columns[6].generated.as_ref().unwrap();
    // `GENERATED ALWAYS` is not part of the type.
    assert_eq!(parsed.columns[6].data_type, "TEXT");
    assert_eq!(up.expression.as_str(), "upper(label)");
    assert!(up.stored);
    let short = parsed.columns[7].generated.as_ref().unwrap();
    assert_eq!(parsed.columns[7].data_type, "");
    assert_eq!(short.expression.as_str(), "substr(label, 1, 2)");
    assert!(!short.stored);
    assert_eq!(parsed.columns[8].data_type, "");
    assert!(!parsed.strict && !parsed.without_rowid);
}

#[test]
fn table_constraints_and_options_in_any_order() {
    let sql = "CREATE TABLE t (a INT, b TEXT, c INT REFERENCES p ON UPDATE SET NULL DEFERRABLE, \
               CONSTRAINT pk PRIMARY KEY (a ASC, b), \
               UNIQUE (b), \
               CONSTRAINT positive CHECK (a > 0), \
               CONSTRAINT to_q FOREIGN KEY (b) REFERENCES q (x) ON DELETE CASCADE \
               DEFERRABLE INITIALLY DEFERRED) STRICT, WITHOUT ROWID";
    let parsed = table(sql);
    assert!(parsed.strict && parsed.without_rowid);
    assert_eq!(
        parsed.columns[2].references,
        vec![ParsedReference {
            name: None,
            table: "p".into(),
            columns: vec![],
            on_update: ReferentialAction::SetNull,
            on_delete: ReferentialAction::NoAction,
            deferrable: Deferrable::InitiallyImmediate,
        }]
    );
    assert_eq!(
        parsed.constraints,
        vec![
            ParsedConstraint::PrimaryKey {
                name: Some("pk".into()),
                columns: vec!["a".into(), "b".into()],
            },
            ParsedConstraint::Unique {
                name: None,
                columns: vec!["b".into()],
            },
            ParsedConstraint::Check {
                name: Some("positive".into()),
                expression: "a > 0".into(),
            },
            ParsedConstraint::ForeignKey {
                columns: vec!["b".into()],
                reference: ParsedReference {
                    name: Some("to_q".into()),
                    table: "q".into(),
                    columns: vec!["x".into()],
                    on_update: ReferentialAction::NoAction,
                    on_delete: ReferentialAction::Cascade,
                    deferrable: Deferrable::InitiallyDeferred,
                },
            },
        ]
    );
    let reordered = table("CREATE TABLE t (a INT PRIMARY KEY) WITHOUT ROWID, STRICT");
    assert!(reordered.strict && reordered.without_rowid);
}

#[test]
fn quoted_names_and_schema_prefixes_are_dequoted() {
    let parsed = table("CREATE TABLE IF NOT EXISTS main.[odd name] (\"a\"\"b\" TEXT, `c` INT)");
    assert_eq!(parsed.name, "odd name");
    assert_eq!(parsed.columns[0].name, "a\"b");
    assert_eq!(parsed.columns[1].name, "c");
}

#[test]
fn attributes_the_model_cannot_hold_are_refused() {
    for sql in [
        "CREATE TABLE t (a TEXT UNIQUE ON CONFLICT REPLACE)",
        "CREATE TABLE t (a INTEGER PRIMARY KEY DESC)",
        "CREATE TABLE t (a TEXT, PRIMARY KEY (a COLLATE NOCASE))",
        "CREATE TABLE t (a TEXT, UNIQUE (a DESC))",
        "CREATE TABLE t (a INT REFERENCES p MATCH FULL)",
        "CREATE TABLE t (a INT NOT NULL ON CONFLICT IGNORE)",
        "CREATE VIRTUAL TABLE t USING fts5(a)",
        "CREATE TABLE t AS SELECT 1",
        "CREATE TABLE t (a INT) WITHOUT",
        "CREATE TABLE t (a INT, 'open)",
    ] {
        assert!(parse_create_table(sql).is_err(), "{sql}");
    }
}

#[test]
fn sqlparser_disagreement_is_reported() {
    // SQLite allows table constraints without a separating comma;
    // sqlparser does not, so the cross-check refuses rather than guessing.
    let sql = "CREATE TABLE t (a INT, b INT, PRIMARY KEY (a) UNIQUE (b))";
    let parsed = parse_create_table(sql).unwrap();
    assert_eq!(parsed.constraints.len(), 2);
    assert!(cross_check(&parsed, sql).is_err());
}

#[test]
fn indexes_keep_member_expressions_collations_and_predicates() {
    let index = parse_create_index(
        "CREATE UNIQUE INDEX main.\"i\" ON users (lower(email) COLLATE NOCASE DESC, \"name\", \
         id ASC) WHERE deleted IS NULL AND (a, b) IS NOT NULL",
    )
    .unwrap();
    assert_eq!(index.name, "i");
    assert_eq!(index.table, "users");
    assert!(index.unique);
    assert_eq!(
        index.members,
        vec![
            ParsedIndexMember {
                target: MemberTarget::Expression("lower(email)".into()),
                collation: Some("NOCASE".into()),
                descending: true,
            },
            ParsedIndexMember {
                target: MemberTarget::Name("name".into()),
                collation: None,
                descending: false,
            },
            ParsedIndexMember {
                target: MemberTarget::Name("id".into()),
                collation: None,
                descending: false,
            },
        ]
    );
    assert_eq!(
        index.predicate.as_deref(),
        Some("deleted IS NULL AND (a, b) IS NOT NULL")
    );
}

#[test]
fn triggers_are_qualified_with_their_schema() {
    assert_eq!(
        qualify_trigger(
            "CREATE TRIGGER t_touch AFTER UPDATE ON t BEGIN SELECT 1; END",
            "main"
        )
        .as_deref(),
        Some("CREATE TRIGGER \"main\".t_touch AFTER UPDATE ON t BEGIN SELECT 1; END")
    );
    assert_eq!(
        qualify_trigger(
            "CREATE TRIGGER IF NOT EXISTS \"x\" BEFORE DELETE ON t BEGIN SELECT 1; END",
            "aux"
        )
        .as_deref(),
        Some("CREATE TRIGGER IF NOT EXISTS \"aux\".\"x\" BEFORE DELETE ON t BEGIN SELECT 1; END")
    );
    let qualified = "CREATE TRIGGER main.x AFTER INSERT ON t BEGIN SELECT 1; END";
    assert_eq!(
        qualify_trigger(qualified, "main").as_deref(),
        Some(qualified)
    );
    assert_eq!(qualify_trigger("CREATE VIEW v AS SELECT 1", "main"), None);
}
