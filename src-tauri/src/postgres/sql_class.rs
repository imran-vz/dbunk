use serde::{Deserialize, Serialize};

use super::sql_lex::{lex_sql, SqlIdentifier, SqlToken};

struct ReadEscalation {
    identifier: &'static str,
    destructive: bool,
}

/// Token-exact write-capable functions that must not retain the read class.
/// Destructive entries also require confirmation in protected mode.
const READ_ESCALATION_DENYLIST: &[ReadEscalation] = &[
    ReadEscalation {
        identifier: "setval",
        destructive: false,
    },
    ReadEscalation {
        identifier: "nextval",
        destructive: false,
    },
    ReadEscalation {
        identifier: "set_config",
        destructive: false,
    },
    ReadEscalation {
        identifier: "pg_cancel_backend",
        destructive: false,
    },
    ReadEscalation {
        identifier: "lo_import",
        destructive: false,
    },
    ReadEscalation {
        identifier: "lo_create",
        destructive: false,
    },
    ReadEscalation {
        identifier: "dblink",
        destructive: false,
    },
    ReadEscalation {
        identifier: "pg_reload_conf",
        destructive: false,
    },
    ReadEscalation {
        identifier: "pg_rotate_logfile",
        destructive: false,
    },
    ReadEscalation {
        identifier: "pg_terminate_backend",
        destructive: true,
    },
    ReadEscalation {
        identifier: "lo_unlink",
        destructive: true,
    },
    ReadEscalation {
        identifier: "dblink_exec",
        destructive: true,
    },
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum StatementClass {
    Read,
    Dml { unbounded: bool, destructive: bool },
    Ddl { destructive: bool },
    Transaction,
    Session,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum StatementClassKind {
    Read,
    Dml,
    Ddl,
    Transaction,
    Session,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct StatementClassSummary {
    pub(crate) index: usize,
    pub(crate) class: StatementClassKind,
    pub(crate) unbounded: bool,
    pub(crate) destructive: bool,
}

impl StatementClass {
    pub(crate) fn summary(&self, index: usize) -> StatementClassSummary {
        let (class, unbounded, destructive) = match self {
            Self::Read => (StatementClassKind::Read, false, false),
            Self::Dml {
                unbounded,
                destructive,
            } => (StatementClassKind::Dml, *unbounded, *destructive),
            Self::Ddl { destructive } => (StatementClassKind::Ddl, false, *destructive),
            Self::Transaction => (StatementClassKind::Transaction, false, false),
            Self::Session => (StatementClassKind::Session, false, false),
            Self::Unknown => (StatementClassKind::Unknown, false, true),
        };
        StatementClassSummary {
            index,
            class,
            unbounded,
            destructive,
        }
    }

    pub(crate) fn label(&self) -> &'static str {
        match self {
            Self::Read => "read",
            Self::Dml { .. } => "dml",
            Self::Ddl { .. } => "ddl",
            Self::Transaction => "transaction",
            Self::Session => "session",
            Self::Unknown => "unknown",
        }
    }
}

pub(crate) fn classify_script(sql: &str) -> Vec<StatementClass> {
    let Ok(tokens) = lex_sql(sql) else {
        return vec![StatementClass::Unknown];
    };
    let Ok(statements) = split_statements(tokens) else {
        return vec![StatementClass::Unknown];
    };
    statements
        .iter()
        .map(|statement| classify_statement(statement))
        .collect()
}

fn split_statements(tokens: Vec<SqlToken>) -> Result<Vec<Vec<SqlToken>>, ()> {
    let mut statements = Vec::new();
    let mut statement = Vec::new();
    let mut depth = 0usize;
    for token in tokens {
        match token {
            SqlToken::Symbol('(') => {
                depth += 1;
                statement.push(token);
            }
            SqlToken::Symbol(')') => {
                depth = depth.checked_sub(1).ok_or(())?;
                statement.push(token);
            }
            SqlToken::Symbol(';') if depth == 0 => {
                if !statement.is_empty() {
                    statements.push(std::mem::take(&mut statement));
                }
            }
            _ => statement.push(token),
        }
    }
    if depth != 0 {
        return Err(());
    }
    if !statement.is_empty() {
        statements.push(statement);
    }
    Ok(statements)
}

fn classify_statement(tokens: &[SqlToken]) -> StatementClass {
    let Some(head) = tokens.first().and_then(identifier) else {
        return StatementClass::Unknown;
    };
    if head.quoted {
        return StatementClass::Unknown;
    }
    let keyword = head.value.to_ascii_uppercase();
    match keyword.as_str() {
        "SELECT" | "VALUES" | "TABLE" | "SHOW" => classified_read(tokens),
        "EXPLAIN" => classify_explain(tokens),
        "COPY" => classify_copy(tokens),
        "WITH" => classify_with(tokens),
        "INSERT" | "MERGE" => StatementClass::Dml {
            unbounded: false,
            destructive: false,
        },
        "UPDATE" | "DELETE" => StatementClass::Dml {
            unbounded: is_unbounded_at(tokens, 0, 0),
            destructive: false,
        },
        "CREATE" | "ALTER" | "COMMENT" | "GRANT" | "REVOKE" | "REINDEX" | "VACUUM" | "ANALYZE"
        | "CLUSTER" | "REFRESH" | "SECURITY" => StatementClass::Ddl { destructive: false },
        "DROP" | "TRUNCATE" => StatementClass::Ddl { destructive: true },
        "BEGIN" | "START" | "COMMIT" | "END" | "ROLLBACK" | "SAVEPOINT" | "RELEASE" | "LOCK" => {
            StatementClass::Transaction
        }
        "SET" | "RESET" | "DISCARD" => StatementClass::Session,
        _ => StatementClass::Unknown,
    }
}

fn classified_read(tokens: &[SqlToken]) -> StatementClass {
    let escalation = read_escalation(tokens);
    if escalation == Some(true) {
        return StatementClass::Dml {
            unbounded: false,
            destructive: true,
        };
    }
    if contains_top_level_select_into(tokens) {
        return StatementClass::Ddl { destructive: false };
    }
    if escalation == Some(false) {
        return StatementClass::Dml {
            unbounded: false,
            destructive: false,
        };
    }
    StatementClass::Read
}

fn classify_explain(tokens: &[SqlToken]) -> StatementClass {
    let mut index = 1usize;
    let analyze = if matches!(tokens.get(index), Some(SqlToken::Symbol('('))) {
        let Some(close_index) = matching_close(tokens, index) else {
            return StatementClass::Unknown;
        };
        let Ok(analyze) = explain_options_analyze(&tokens[index + 1..close_index]) else {
            return StatementClass::Unknown;
        };
        index = close_index + 1;
        analyze
    } else {
        let analyze = tokens
            .get(index)
            .is_some_and(|token| is_keyword(token, "analyze"));
        if analyze {
            index += 1;
        }
        if tokens
            .get(index)
            .is_some_and(|token| is_keyword(token, "verbose"))
        {
            index += 1;
        }
        analyze
    };
    if index >= tokens.len() {
        return StatementClass::Unknown;
    }
    if analyze {
        classify_statement(&tokens[index..])
    } else {
        // Without ANALYZE, EXPLAIN plans but does not execute the wrapped
        // statement, including any denylisted read-shaped expressions.
        StatementClass::Read
    }
}

fn explain_options_analyze(tokens: &[SqlToken]) -> Result<bool, ()> {
    if tokens.is_empty() {
        return Err(());
    }

    let mut analyze = None;
    for option in tokens.split(|token| matches!(token, SqlToken::Symbol(','))) {
        let Some(name) = option.first() else {
            return Err(());
        };
        if !is_keyword(name, "analyze") {
            // An ANALYZE token anywhere other than the option name is
            // malformed or ambiguous, so it cannot prove non-execution.
            if option
                .iter()
                .skip(1)
                .any(|token| is_keyword(token, "analyze"))
            {
                return Err(());
            }
            continue;
        }
        if analyze.is_some() {
            return Err(());
        }
        analyze = Some(match &option[1..] {
            [] => true,
            [value] if is_keyword(value, "true") || is_keyword(value, "on") => true,
            [value] if is_keyword(value, "false") || is_keyword(value, "off") => false,
            // The lexer intentionally erases opaque spellings, so 0 and 1
            // cannot be distinguished here. Fail closed instead of guessing.
            _ => return Err(()),
        });
    }
    Ok(analyze.unwrap_or(false))
}

fn matching_close(tokens: &[SqlToken], open_index: usize) -> Option<usize> {
    let mut depth = 0usize;
    for (index, token) in tokens.iter().enumerate().skip(open_index) {
        match token {
            SqlToken::Symbol('(') => depth += 1,
            SqlToken::Symbol(')') => {
                depth = depth.checked_sub(1)?;
                if depth == 0 {
                    return Some(index);
                }
            }
            _ => {}
        }
    }
    None
}

fn classify_copy(tokens: &[SqlToken]) -> StatementClass {
    let depths = token_depths(tokens);
    if let Some(direction_index) = token_index_at_depth(tokens, &depths, "from", 0) {
        let destructive = tokens
            .get(direction_index + 1)
            .is_some_and(|token| is_keyword(token, "program"));
        return copy_write_class(tokens, destructive);
    }

    let Some(direction_index) = token_index_at_depth(tokens, &depths, "to", 0) else {
        return StatementClass::Unknown;
    };
    match tokens.get(direction_index + 1) {
        Some(token) if is_keyword(token, "stdout") => {
            if contains_write_keyword(tokens) {
                copy_write_class(tokens, false)
            } else {
                classified_read(tokens)
            }
        }
        // A string literal is a server file path. PROGRAM and server file
        // destinations have effects beyond returning rows to the client.
        Some(token) if is_keyword(token, "program") => copy_write_class(tokens, true),
        Some(SqlToken::Opaque) => copy_write_class(tokens, true),
        _ => StatementClass::Unknown,
    }
}

fn copy_write_class(tokens: &[SqlToken], destructive: bool) -> StatementClass {
    StatementClass::Dml {
        unbounded: contains_unbounded_update_delete(tokens),
        destructive,
    }
}

fn classify_with(tokens: &[SqlToken]) -> StatementClass {
    if contains_write_keyword(tokens) {
        StatementClass::Dml {
            unbounded: contains_unbounded_update_delete(tokens),
            destructive: false,
        }
    } else {
        classified_read(tokens)
    }
}

fn contains_write_keyword(tokens: &[SqlToken]) -> bool {
    tokens.iter().any(|token| {
        ["insert", "update", "delete", "merge"]
            .iter()
            .any(|keyword| is_keyword(token, keyword))
    })
}

fn contains_top_level_select_into(tokens: &[SqlToken]) -> bool {
    let depths = token_depths(tokens);
    let mut saw_select = false;
    for (token, depth) in tokens.iter().zip(depths) {
        if depth != 0 {
            continue;
        }
        if is_keyword(token, "select") {
            saw_select = true;
        } else if saw_select && is_keyword(token, "into") {
            return true;
        }
    }
    false
}

fn contains_unbounded_update_delete(tokens: &[SqlToken]) -> bool {
    let depths = token_depths(tokens);
    tokens.iter().enumerate().any(|(index, token)| {
        (is_keyword(token, "update") || is_keyword(token, "delete"))
            && is_unbounded_at(tokens, index, depths[index])
    })
}

fn is_unbounded_at(tokens: &[SqlToken], head_index: usize, head_depth: usize) -> bool {
    let depths = token_depths(tokens);
    !tokens
        .iter()
        .enumerate()
        .skip(head_index + 1)
        .take_while(|(index, token)| {
            depths[*index] >= head_depth
                && !(depths[*index] == head_depth
                    && ["insert", "update", "delete", "merge"]
                        .iter()
                        .any(|keyword| is_keyword(token, keyword)))
        })
        .any(|(index, token)| depths[index] == head_depth && is_keyword(token, "where"))
}

fn token_depths(tokens: &[SqlToken]) -> Vec<usize> {
    let mut depth = 0usize;
    tokens
        .iter()
        .map(|token| {
            if matches!(token, SqlToken::Symbol(')')) {
                depth = depth.saturating_sub(1);
            }
            let token_depth = depth;
            if matches!(token, SqlToken::Symbol('(')) {
                depth += 1;
            }
            token_depth
        })
        .collect()
}

fn token_index_at_depth(
    tokens: &[SqlToken],
    depths: &[usize],
    keyword: &str,
    depth: usize,
) -> Option<usize> {
    tokens
        .iter()
        .zip(depths)
        .position(|(token, token_depth)| *token_depth == depth && is_keyword(token, keyword))
}

fn identifier(token: &SqlToken) -> Option<&SqlIdentifier> {
    match token {
        SqlToken::Identifier(identifier) => Some(identifier),
        _ => None,
    }
}

fn is_keyword(token: &SqlToken, keyword: &str) -> bool {
    matches!(
        token,
        SqlToken::Identifier(SqlIdentifier { value, quoted: false })
            if value.eq_ignore_ascii_case(keyword)
    )
}

fn read_escalation(tokens: &[SqlToken]) -> Option<bool> {
    let mut matched = false;
    let mut destructive = false;
    for token in tokens {
        let SqlToken::Identifier(SqlIdentifier {
            value,
            quoted: false,
        }) = token
        else {
            continue;
        };
        for escalation in READ_ESCALATION_DENYLIST {
            if value.eq_ignore_ascii_case(escalation.identifier) {
                matched = true;
                destructive |= escalation.destructive;
            }
        }
    }
    matched.then_some(destructive)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::safety::policy::{
        assert_permitted, requires_confirmation, ResolvedSafetyPolicy, SafetyLevel, WriteIntent,
    };
    use crate::Environment;

    fn one(sql: &str) -> StatementClass {
        classify_script(sql).into_iter().next().unwrap()
    }

    #[test]
    fn classifies_every_head_family_and_comments() {
        for sql in ["SELECT 1", "VALUES (1)", "TABLE t", "SHOW search_path"] {
            assert_eq!(one(sql), StatementClass::Read, "{sql}");
        }
        for sql in ["INSERT INTO t VALUES (1)", "MERGE INTO t USING s ON false"] {
            assert_eq!(
                one(sql),
                StatementClass::Dml {
                    unbounded: false,
                    destructive: false
                },
                "{sql}"
            );
        }
        for sql in [
            "CREATE TABLE t (id int)",
            "ALTER TABLE t ADD COLUMN x int",
            "COMMENT ON TABLE t IS 'x'",
            "GRANT SELECT ON t TO u",
            "REVOKE SELECT ON t FROM u",
            "REINDEX TABLE t",
            "VACUUM t",
            "ANALYZE t",
            "CLUSTER t",
            "REFRESH MATERIALIZED VIEW v",
            "SECURITY LABEL ON TABLE t IS 'x'",
        ] {
            assert_eq!(
                one(sql),
                StatementClass::Ddl { destructive: false },
                "{sql}"
            );
        }
        for sql in [
            "BEGIN",
            "START TRANSACTION",
            "COMMIT",
            "END",
            "ROLLBACK",
            "SAVEPOINT x",
            "RELEASE x",
            "LOCK t",
        ] {
            assert_eq!(one(sql), StatementClass::Transaction, "{sql}");
        }
        for sql in ["SET search_path = public", "RESET ALL", "DISCARD ALL"] {
            assert_eq!(one(sql), StatementClass::Session, "{sql}");
        }
        assert_eq!(
            one("-- lead\n/* nested /* comment */ */ SELECT 1"),
            StatementClass::Read
        );
        assert_eq!(
            one("DO $$ BEGIN DELETE FROM t; END $$"),
            StatementClass::Unknown
        );
        assert_eq!(one("CALL f()"), StatementClass::Unknown);
    }

    #[test]
    fn explain_only_escalates_when_analyze_executes_the_wrapped_statement() {
        assert_eq!(one("EXPLAIN UPDATE t SET x = 1"), StatementClass::Read);
        assert_eq!(one("EXPLAIN VERBOSE SELECT 1"), StatementClass::Read);
        assert_eq!(
            one("EXPLAIN SELECT nextval('sequence_name')"),
            StatementClass::Read
        );
        assert_eq!(
            one("EXPLAIN (FORMAT JSON) SELECT nextval('sequence_name')"),
            StatementClass::Read
        );
        assert_eq!(
            one("EXPLAIN (ANALYZE) UPDATE t SET x = 1 WHERE id = 2"),
            StatementClass::Dml {
                unbounded: false,
                destructive: false
            }
        );
        assert_eq!(
            one("EXPLAIN ANALYZE UPDATE t SET x = 1"),
            StatementClass::Dml {
                unbounded: true,
                destructive: false
            }
        );
        assert_eq!(
            one("EXPLAIN ANALYZE VERBOSE DELETE FROM t WHERE id = 2"),
            StatementClass::Dml {
                unbounded: false,
                destructive: false
            }
        );
        assert_eq!(
            one("EXPLAIN (ANALYZE, BUFFERS, FORMAT JSON) DELETE FROM t"),
            StatementClass::Dml {
                unbounded: true,
                destructive: false
            }
        );
        assert_eq!(
            one("EXPLAIN (ANALYZE) DROP TABLE t"),
            StatementClass::Ddl { destructive: true }
        );
        for sql in [
            "EXPLAIN (ANALYZE TRUE) UPDATE t SET x = 1",
            "EXPLAIN (ANALYZE ON) UPDATE t SET x = 1",
        ] {
            assert_eq!(
                one(sql),
                StatementClass::Dml {
                    unbounded: true,
                    destructive: false
                },
                "{sql}"
            );
        }
        for sql in [
            "EXPLAIN (ANALYZE FALSE) SELECT nextval('sequence_name')",
            "EXPLAIN (ANALYZE OFF, FORMAT JSON) UPDATE t SET x = 1",
        ] {
            assert_eq!(one(sql), StatementClass::Read, "{sql}");
        }
        assert_eq!(
            one("EXPLAIN (ANALYZE 0) UPDATE t SET x = 1"),
            StatementClass::Unknown
        );
    }

    #[test]
    fn copy_combines_direction_and_embedded_write_scanning() {
        assert_eq!(one("COPY t TO STDOUT"), StatementClass::Read);
        assert_eq!(one("COPY program TO STDOUT"), StatementClass::Read);
        assert_eq!(
            one("COPY (SELECT program FROM t) TO STDOUT"),
            StatementClass::Read
        );
        assert_eq!(
            one("COPY t FROM STDIN"),
            StatementClass::Dml {
                unbounded: false,
                destructive: false
            }
        );
        for sql in [
            "COPY t TO '/var/tmp/export.csv'",
            "COPY t TO PROGRAM 'gzip > /var/tmp/export.csv.gz'",
            "COPY t FROM PROGRAM 'generate_rows'",
        ] {
            assert_eq!(
                one(sql),
                StatementClass::Dml {
                    unbounded: false,
                    destructive: true
                },
                "{sql}"
            );
        }
        assert_eq!(
            one("COPY (WITH d AS (DELETE FROM t RETURNING *) SELECT * FROM d) TO STDOUT"),
            StatementClass::Dml {
                unbounded: true,
                destructive: false
            }
        );
        assert_eq!(
            one("COPY (INSERT INTO t VALUES (1) RETURNING *) TO PROGRAM 'consume_rows'"),
            StatementClass::Dml {
                unbounded: false,
                destructive: true
            }
        );
    }

    #[test]
    fn read_denylist_distinguishes_writes_from_destructive_calls() {
        for sql in [
            "SELECT setval('s', 5)",
            "SELECT nextval('s')",
            "SELECT set_config('x', 'y', false)",
            "SELECT pg_cancel_backend(42)",
            "SELECT lo_import('/tmp/file')",
            "SELECT lo_create(42)",
            "SELECT dblink('connection', 'SELECT 1')",
            "SELECT pg_reload_conf()",
            "SELECT pg_rotate_logfile()",
        ] {
            assert_eq!(
                one(sql),
                StatementClass::Dml {
                    unbounded: false,
                    destructive: false
                },
                "{sql}"
            );
        }
        for sql in [
            "SELECT pg_terminate_backend(42)",
            "SELECT lo_unlink(42)",
            "SELECT dblink_exec('connection', 'DROP TABLE t')",
        ] {
            assert_eq!(
                one(sql),
                StatementClass::Dml {
                    unbounded: false,
                    destructive: true
                },
                "{sql}"
            );
        }
    }

    #[test]
    fn read_denylist_is_token_exact_and_conservatively_matches_columns() {
        assert_eq!(
            one("SELECT nextval FROM t"),
            StatementClass::Dml {
                unbounded: false,
                destructive: false
            }
        );
        assert_eq!(one("SELECT * FROM setval_log"), StatementClass::Read);
        assert_eq!(one("SELECT \"nextval\" FROM t"), StatementClass::Read);
    }

    #[test]
    fn select_into_is_non_destructive_ddl_only_at_statement_depth() {
        for sql in [
            "SELECT id INTO new_table FROM source_table",
            "WITH source AS (SELECT id FROM source_table) SELECT id INTO new_table FROM source",
            "EXPLAIN ANALYZE SELECT id INTO new_table FROM source_table",
            "EXPLAIN (ANALYZE) WITH source AS (SELECT 1 AS id) SELECT id INTO new_table FROM source",
        ] {
            assert_eq!(
                one(sql),
                StatementClass::Ddl { destructive: false },
                "{sql}"
            );
        }

        for sql in [
            "SELECT 'INTO new_table'",
            "SELECT \"into\" FROM source_table",
            "SELECT (SELECT id INTO nested_table FROM source_table)",
            "SELECT 1 /* INTO new_table */",
        ] {
            assert_eq!(one(sql), StatementClass::Read, "{sql}");
        }
        assert_eq!(
            one("EXPLAIN SELECT id INTO new_table FROM source_table"),
            StatementClass::Read
        );
    }

    #[test]
    fn scripts_classify_select_into_in_order() {
        assert_eq!(
            classify_script(
                "SELECT 1; SELECT id INTO new_table FROM source_table; SELECT lo_unlink(42)"
            ),
            vec![
                StatementClass::Read,
                StatementClass::Ddl { destructive: false },
                StatementClass::Dml {
                    unbounded: false,
                    destructive: true,
                },
            ]
        );
    }

    #[test]
    fn read_shaped_writes_drive_the_expected_policy_decisions() {
        let policy = |level, read_only| ResolvedSafetyPolicy {
            environment: Environment::Production,
            level,
            read_only,
        };
        let intent = |sql| WriteIntent::Statement {
            classes: classify_script(sql),
        };

        let cancel = intent("SELECT pg_cancel_backend(42)");
        assert!(!requires_confirmation(
            &policy(SafetyLevel::Protected, false),
            &cancel
        ));
        assert!(assert_permitted(&policy(SafetyLevel::Protected, true), &cancel, false).is_err());

        let terminate = intent("SELECT pg_terminate_backend(42)");
        assert!(requires_confirmation(
            &policy(SafetyLevel::Protected, false),
            &terminate
        ));

        let select_into = intent("SELECT id INTO new_table FROM source_table");
        assert!(requires_confirmation(
            &policy(SafetyLevel::Strict, false),
            &select_into
        ));
        assert!(
            assert_permitted(&policy(SafetyLevel::Disabled, true), &select_into, true).is_err()
        );

        let copy_to_program = intent("COPY t TO PROGRAM 'consume_rows'");
        assert!(requires_confirmation(
            &policy(SafetyLevel::Protected, false),
            &copy_to_program
        ));
        assert!(requires_confirmation(
            &policy(SafetyLevel::Strict, false),
            &copy_to_program
        ));
        assert!(
            assert_permitted(&policy(SafetyLevel::Disabled, true), &copy_to_program, true).is_err()
        );
    }

    #[test]
    fn with_scans_writes_and_unboundedness_at_the_write_depth() {
        assert_eq!(
            one("WITH c AS (SELECT 1) SELECT * FROM c"),
            StatementClass::Read
        );
        assert_eq!(
            one("WITH c AS (SELECT 1) DELETE FROM t WHERE id = 1"),
            StatementClass::Dml {
                unbounded: false,
                destructive: false
            }
        );
        assert_eq!(
            one("WITH d AS (DELETE FROM t RETURNING *) SELECT * FROM d"),
            StatementClass::Dml {
                unbounded: true,
                destructive: false
            }
        );
    }

    #[test]
    fn unbounded_detection_only_accepts_where_at_the_statement_depth() {
        assert_eq!(
            one("UPDATE t SET x = 1 WHERE id IN (SELECT id FROM u WHERE ok)"),
            StatementClass::Dml {
                unbounded: false,
                destructive: false
            }
        );
        assert_eq!(
            one("UPDATE t SET x = (SELECT max(y) FROM u WHERE ok)"),
            StatementClass::Dml {
                unbounded: true,
                destructive: false
            }
        );
        assert_eq!(
            one("DELETE FROM t"),
            StatementClass::Dml {
                unbounded: true,
                destructive: false
            }
        );
    }

    #[test]
    fn ddl_destructiveness_is_explicit() {
        assert_eq!(
            one("DROP TABLE t"),
            StatementClass::Ddl { destructive: true }
        );
        assert_eq!(one("TRUNCATE t"), StatementClass::Ddl { destructive: true });
        assert_eq!(
            one("CREATE TABLE t (id int)"),
            StatementClass::Ddl { destructive: false }
        );
        assert_eq!(
            one("ALTER TABLE t ADD x int"),
            StatementClass::Ddl { destructive: false }
        );
    }

    #[test]
    fn scripts_preserve_order_without_splitting_opaque_content() {
        assert_eq!(
            classify_script("SELECT ';'; /* ; */ INSERT INTO t VALUES ($tag$;$tag$); DROP TABLE t"),
            vec![
                StatementClass::Read,
                StatementClass::Dml {
                    unbounded: false,
                    destructive: false
                },
                StatementClass::Ddl { destructive: true },
            ]
        );
        assert!(classify_script(" ; -- blank\n ; ").is_empty());
        assert_eq!(
            classify_script("SELECT 'unterminated"),
            vec![StatementClass::Unknown]
        );
        assert_eq!(classify_script("SELECT (1"), vec![StatementClass::Unknown]);
    }
}
