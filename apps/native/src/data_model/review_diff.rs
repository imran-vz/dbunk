//! Plan 032 review content: a per-change `old → new` diff built from the
//! reviewed `MutationPlan` alone, parameter lines for the SQL preview, and
//! apply-failure text that never repeats row values.
use dbunk_lib::backend::data::{
    DmlParam, InvalidPlanReason, MutationOp, MutationPlan, MutationTable, MutationValue,
    NotAnalyzableReason, ResultMutationError,
};

/// Values in the diff and parameter lines are cut at this many characters.
pub const DIFF_TEXT_CHARS: usize = 256;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DiffValue {
    /// `None` is SQL NULL.
    pub text: Option<String>,
    pub truncated: bool,
}

/// One column of a change. Updates carry both sides, inserts only `new`,
/// deletes only `old`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DiffCell {
    pub column: String,
    pub old: Option<DiffValue>,
    pub new: Option<DiffValue>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DiffKind {
    Update,
    Insert,
    Delete,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DiffChange {
    /// Position in the plan; the backend reports failures by this index.
    pub op_index: usize,
    pub kind: DiffKind,
    /// `schema.table`.
    pub target: String,
    /// `a = 1, b = 'x'`; empty for inserts.
    pub identity: String,
    pub cells: Vec<DiffCell>,
    /// Inserts list only their explicit values; every other column takes
    /// its default.
    pub omitted_defaults: bool,
}

fn diff_value(value: &Option<String>) -> DiffValue {
    match value {
        None => DiffValue {
            text: None,
            truncated: false,
        },
        Some(text) => {
            let (text, truncated) = cut(text);
            DiffValue {
                text: Some(text.to_owned()),
                truncated,
            }
        }
    }
}

/// The first `DIFF_TEXT_CHARS` characters, on a char boundary.
fn cut(text: &str) -> (&str, bool) {
    match text.char_indices().nth(DIFF_TEXT_CHARS) {
        Some((end, _)) => (&text[..end], true),
        None => (text, false),
    }
}

/// A SQL-style literal for display: NULL, a bare number, or a quoted string.
fn literal(value: &Option<String>, quote_numbers: bool) -> String {
    let Some(text) = value else {
        return "NULL".into();
    };
    let (shown, truncated) = cut(text);
    let ellipsis = if truncated { "…" } else { "" };
    if !quote_numbers && is_number(text) {
        return format!("{shown}{ellipsis}");
    }
    format!("'{}{ellipsis}'", shown.replace('\'', "''"))
}

fn is_number(text: &str) -> bool {
    let digits = text.strip_prefix('-').unwrap_or(text);
    let (whole, fraction) = digits.split_once('.').unwrap_or((digits, "0"));
    !whole.is_empty()
        && !fraction.is_empty()
        && whole.bytes().all(|byte| byte.is_ascii_digit())
        && fraction.bytes().all(|byte| byte.is_ascii_digit())
}

fn target(table: &MutationTable) -> String {
    format!("{}.{}", table.schema, table.table)
}

fn identity(values: &[MutationValue]) -> String {
    values
        .iter()
        .map(|value| format!("{} = {}", value.column, literal(&value.value, false)))
        .collect::<Vec<_>>()
        .join(", ")
}

pub fn review_diff(plan: &MutationPlan) -> Vec<DiffChange> {
    plan.operations
        .iter()
        .enumerate()
        .map(|(op_index, operation)| match operation {
            MutationOp::Update {
                table,
                identity: key,
                guards,
                set,
            } => DiffChange {
                op_index,
                kind: DiffKind::Update,
                target: target(table),
                identity: identity(key),
                // Ctid and virtual-key guards cover the whole row; only the
                // assigned columns change.
                cells: set
                    .iter()
                    .map(|value| DiffCell {
                        column: value.column.clone(),
                        old: guards
                            .iter()
                            .find(|guard| guard.column == value.column)
                            .map(|guard| diff_value(&guard.value)),
                        new: Some(diff_value(&value.value)),
                    })
                    .collect(),
                omitted_defaults: false,
            },
            MutationOp::Delete {
                table,
                identity: key,
                guards,
            } => DiffChange {
                op_index,
                kind: DiffKind::Delete,
                target: target(table),
                identity: identity(key),
                cells: guards
                    .iter()
                    .map(|guard| DiffCell {
                        column: guard.column.clone(),
                        old: Some(diff_value(&guard.value)),
                        new: None,
                    })
                    .collect(),
                omitted_defaults: false,
            },
            MutationOp::Insert { table, values } => DiffChange {
                op_index,
                kind: DiffKind::Insert,
                target: target(table),
                identity: String::new(),
                cells: values
                    .iter()
                    .map(|value| DiffCell {
                        column: value.column.clone(),
                        old: None,
                        new: Some(diff_value(&value.value)),
                    })
                    .collect(),
                omitted_defaults: true,
            },
        })
        .collect()
}

/// `"2 updates · 1 insert · 3 deletes"`; kinds with no changes are left out.
pub fn diff_summary(changes: &[DiffChange]) -> String {
    let count = |kind| changes.iter().filter(|change| change.kind == kind).count();
    let parts = [
        (count(DiffKind::Update), "update"),
        (count(DiffKind::Insert), "insert"),
        (count(DiffKind::Delete), "delete"),
    ]
    .into_iter()
    .filter(|(count, _)| *count > 0)
    .map(|(count, noun)| format!("{count} {noun}{}", if count == 1 { "" } else { "s" }))
    .collect::<Vec<_>>();
    if parts.is_empty() {
        "No changes".into()
    } else {
        parts.join(" · ")
    }
}

/// `"$1 = 'text'"` or `"$1 = NULL"`. `index` is the zero-based position in
/// `PreviewStatement::params`; the placeholder is one-based like the SQL.
pub fn format_param(index: usize, param: &DmlParam) -> String {
    let DmlParam::Text { value } = param;
    format!("${} = {}", index + 1, literal(value, true))
}

fn sqlstate(code: &str) -> Option<&'static str> {
    Some(match code {
        "23505" => "unique violation",
        "23503" => "foreign key violation",
        "23502" => "not-null violation",
        "23514" => "check violation",
        "23P01" => "exclusion violation",
        "22001" => "value too long",
        "22003" => "numeric value out of range",
        "22007" | "22008" => "invalid date or time",
        "22P02" => "invalid input syntax",
        "42501" => "insufficient privilege",
        "40001" => "serialization failure",
        "40P01" => "deadlock detected",
        "55P03" => "lock not available",
        "57014" => "statement cancelled",
        _ => return None,
    })
}

fn invalid_plan(reason: InvalidPlanReason) -> &'static str {
    match reason {
        InvalidPlanReason::EmptySet => "an update sets no columns",
        InvalidPlanReason::EmptyIdentity => "a change has no row identity",
        InvalidPlanReason::NullKeyedIdentity => "a key value is NULL",
        InvalidPlanReason::MissingGuard => "a change is missing its original value",
        InvalidPlanReason::IdentityMismatch => "a row identity no longer matches the table",
        InvalidPlanReason::TableMismatch => "a change targets a different table",
        InvalidPlanReason::DuplicateColumn => "a column is assigned twice",
        InvalidPlanReason::GeneratedColumn => "a generated column cannot be written",
        InvalidPlanReason::IdentityAlwaysColumn => {
            "a GENERATED ALWAYS identity column cannot be written"
        }
        InvalidPlanReason::SystemColumn => "a system column cannot be written",
        InvalidPlanReason::NoIdentity => "the table has no row identity",
        InvalidPlanReason::MultipleOriginTables => "changes span more than one table",
    }
}

fn not_analyzable(reason: &NotAnalyzableReason) -> String {
    match reason {
        NotAnalyzableReason::MultiStatement => "it has more than one statement".into(),
        NotAnalyzableReason::NoProjectedColumns => "it projects no columns".into(),
        NotAnalyzableReason::NoTableOrigins => "no column comes from a table".into(),
        NotAnalyzableReason::PossibleTempShadowing => {
            "a temporary table may shadow the target".into()
        }
        NotAnalyzableReason::SessionDependentTypes => "column types depend on the session".into(),
        NotAnalyzableReason::Database { code, .. } => database(code.as_deref()),
    }
}

/// SQLSTATE only: database messages can quote row values.
fn database(code: Option<&str>) -> String {
    match code {
        Some(code) => match sqlstate(code) {
            Some(name) => format!("{name} (SQLSTATE {code})"),
            None => format!("database error (SQLSTATE {code})"),
        },
        None => "database error".into(),
    }
}

const NOTHING_APPLIED: &str = "No changes were applied.";
const UNKNOWN_OUTCOME: &str = "The outcome is unknown: refresh, then mark resolved.";

/// One line for the review dialog. Change numbers are one-based; row values,
/// parameters and raw database messages are never included.
pub fn apply_error_message(error: &ResultMutationError) -> String {
    match error {
        ResultMutationError::UnsupportedEngine => {
            "Editing is not supported on this connection's engine.".into()
        }
        ResultMutationError::NotAnalyzable { reason } => format!(
            "The table cannot be edited because {}.",
            not_analyzable(reason)
        ),
        ResultMutationError::UnknownColumn { column } => {
            format!("Unknown column {column}. Refresh, then review again.")
        }
        ResultMutationError::InvalidPlan { reason } => format!(
            "The changes were refused because {}. {NOTHING_APPLIED}",
            invalid_plan(*reason)
        ),
        ResultMutationError::AnalysisExpired => {
            "Column analysis expired. Refresh, then review again.".into()
        }
        ResultMutationError::Conflict { op_index } => format!(
            "Change {} conflicts: the row changed or was deleted since it was loaded. {NOTHING_APPLIED}",
            op_index + 1
        ),
        ResultMutationError::IdentityNotUnique { op_index } => format!(
            "Change {} matches more than one row. {NOTHING_APPLIED}",
            op_index + 1
        ),
        ResultMutationError::LockTimeout { op_index } => format!(
            "Change {} timed out waiting for a row lock. {NOTHING_APPLIED}",
            op_index + 1
        ),
        ResultMutationError::Busy => {
            "Another change operation is running on this connection. Try again.".into()
        }
        ResultMutationError::Superseded => "A newer request replaced this apply.".into(),
        ResultMutationError::Cancelled => "Apply was cancelled.".into(),
        ResultMutationError::ConnectionClosing | ResultMutationError::ConnectionLost => {
            format!("The connection was lost during apply. {UNKNOWN_OUTCOME}")
        }
        ResultMutationError::TlsFailed { tls_kind, .. } => {
            format!("The TLS connection failed ({tls_kind:?}).")
        }
        ResultMutationError::PolicyBlocked { reason } => {
            format!("Blocked by the safety policy: {reason}")
        }
        ResultMutationError::PolicyNeedsConfirmation { .. } => {
            "This connection's safe mode requires confirmation.".into()
        }
        ResultMutationError::Timeout { operation } => {
            format!("Timed out during {operation}. {UNKNOWN_OUTCOME}")
        }
        ResultMutationError::Database { code, op_index, .. } => {
            let what = database(code.as_deref());
            match op_index {
                Some(index) => format!("Change {} failed: {what}. {NOTHING_APPLIED}", index + 1),
                None => format!("Apply failed: {what}. {NOTHING_APPLIED}"),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn value(column: &str, value: Option<&str>) -> MutationValue {
        MutationValue {
            column: column.into(),
            value: value.map(str::to_owned),
        }
    }
    fn table() -> MutationTable {
        MutationTable {
            schema: "public".into(),
            table: "users".into(),
        }
    }
    fn text(value: &str) -> Option<DiffValue> {
        Some(DiffValue {
            text: Some(value.into()),
            truncated: false,
        })
    }
    const NULL: Option<DiffValue> = Some(DiffValue {
        text: None,
        truncated: false,
    });

    #[test]
    fn keyed_update_pairs_old_and_new_for_each_set_column() {
        let plan = MutationPlan {
            operations: vec![MutationOp::Update {
                table: table(),
                identity: vec![value("id", Some("1"))],
                guards: vec![value("name", Some("Ada")), value("note", None)],
                set: vec![value("name", Some("Bob")), value("note", Some("x"))],
            }],
        };
        let diff = review_diff(&plan);
        assert_eq!(
            diff,
            vec![DiffChange {
                op_index: 0,
                kind: DiffKind::Update,
                target: "public.users".into(),
                identity: "id = 1".into(),
                cells: vec![
                    DiffCell {
                        column: "name".into(),
                        old: text("Ada"),
                        new: text("Bob"),
                    },
                    DiffCell {
                        column: "note".into(),
                        old: NULL,
                        new: text("x"),
                    },
                ],
                omitted_defaults: false,
            }]
        );
    }

    #[test]
    fn ctid_update_lists_only_set_columns_despite_full_row_guards() {
        let plan = MutationPlan {
            operations: vec![MutationOp::Update {
                table: table(),
                identity: vec![value("ctid", Some("(0,1)"))],
                guards: vec![
                    value("id", Some("1")),
                    value("name", Some("Ada")),
                    value("note", None),
                    value("ctid", Some("(0,1)")),
                ],
                set: vec![value("name", None)],
            }],
        };
        let diff = review_diff(&plan);
        assert_eq!(diff[0].identity, "ctid = '(0,1)'");
        assert_eq!(
            diff[0].cells,
            vec![DiffCell {
                column: "name".into(),
                old: text("Ada"),
                new: NULL,
            }]
        );
    }

    #[test]
    fn deletes_remove_guards_and_inserts_add_values_over_defaults() {
        let plan = MutationPlan {
            operations: vec![
                MutationOp::Delete {
                    table: table(),
                    identity: vec![value("id", Some("7"))],
                    guards: vec![value("id", Some("7")), value("name", None)],
                },
                MutationOp::Insert {
                    table: table(),
                    values: vec![value("name", Some("New")), value("note", None)],
                },
            ],
        };
        let diff = review_diff(&plan);
        assert_eq!(diff[0].kind, DiffKind::Delete);
        assert_eq!(
            diff[0].cells,
            vec![
                DiffCell {
                    column: "id".into(),
                    old: text("7"),
                    new: None,
                },
                DiffCell {
                    column: "name".into(),
                    old: NULL,
                    new: None,
                },
            ]
        );
        assert!(!diff[0].omitted_defaults);
        assert_eq!(diff[1].op_index, 1);
        assert_eq!(diff[1].kind, DiffKind::Insert);
        assert_eq!(diff[1].identity, "");
        assert!(diff[1].omitted_defaults);
        assert_eq!(
            diff[1].cells,
            vec![
                DiffCell {
                    column: "name".into(),
                    old: None,
                    new: text("New"),
                },
                DiffCell {
                    column: "note".into(),
                    old: None,
                    new: NULL,
                },
            ]
        );
        assert_eq!(diff_summary(&diff), "1 insert · 1 delete");
        assert_eq!(diff_summary(&[]), "No changes");
        let updates = vec![diff[0].clone(), diff[0].clone()]
            .into_iter()
            .map(|change| DiffChange {
                kind: DiffKind::Update,
                ..change
            })
            .chain(diff.iter().cloned())
            .collect::<Vec<_>>();
        assert_eq!(diff_summary(&updates), "2 updates · 1 insert · 1 delete");
    }

    #[test]
    fn values_truncate_on_a_char_boundary() {
        let long = "é".repeat(DIFF_TEXT_CHARS + 5);
        let cut = diff_value(&Some(long));
        assert!(cut.truncated);
        assert_eq!(cut.text.unwrap(), "é".repeat(DIFF_TEXT_CHARS));
        let exact = "a".repeat(DIFF_TEXT_CHARS);
        assert_eq!(
            diff_value(&Some(exact.clone())),
            DiffValue {
                text: Some(exact),
                truncated: false,
            }
        );
        let param = format_param(
            0,
            &DmlParam::Text {
                value: Some("🙂".repeat(DIFF_TEXT_CHARS + 1)),
            },
        );
        assert_eq!(param, format!("$1 = '{}…'", "🙂".repeat(DIFF_TEXT_CHARS)));
    }

    #[test]
    fn identities_and_params_render_like_sql() {
        assert_eq!(
            identity(&[
                value("a", Some("1")),
                value("b", Some("x")),
                value("c", None),
                value("d", Some("-2.5")),
                value("e", Some("it's")),
                value("f", Some("1e3")),
            ]),
            "a = 1, b = 'x', c = NULL, d = -2.5, e = 'it''s', f = '1e3'"
        );
        assert_eq!(
            format_param(
                0,
                &DmlParam::Text {
                    value: Some("text".into())
                }
            ),
            "$1 = 'text'"
        );
        assert_eq!(
            format_param(2, &DmlParam::Text { value: None }),
            "$3 = NULL"
        );
        assert_eq!(
            format_param(
                1,
                &DmlParam::Text {
                    value: Some("42".into())
                }
            ),
            "$2 = '42'"
        );
    }

    #[test]
    fn apply_errors_number_changes_from_one_and_never_leak_values() {
        let secret = "secret-row-value";
        let errors = [
            ResultMutationError::UnsupportedEngine,
            ResultMutationError::NotAnalyzable {
                reason: NotAnalyzableReason::Database {
                    code: Some("42P01".into()),
                    message: secret.into(),
                    severity: None,
                    position: None,
                },
            },
            ResultMutationError::NotAnalyzable {
                reason: NotAnalyzableReason::MultiStatement,
            },
            ResultMutationError::UnknownColumn {
                column: "name".into(),
            },
            ResultMutationError::InvalidPlan {
                reason: InvalidPlanReason::GeneratedColumn,
            },
            ResultMutationError::AnalysisExpired,
            ResultMutationError::Conflict { op_index: 2 },
            ResultMutationError::IdentityNotUnique { op_index: 2 },
            ResultMutationError::LockTimeout { op_index: 2 },
            ResultMutationError::Busy,
            ResultMutationError::Superseded,
            ResultMutationError::Cancelled,
            ResultMutationError::ConnectionClosing,
            ResultMutationError::ConnectionLost,
            ResultMutationError::TlsFailed {
                tls_kind: dbunk_lib::backend::TlsFailureKind::CertificateUntrusted,
                message: secret.into(),
            },
            ResultMutationError::PolicyBlocked {
                reason: "This connection is read-only".into(),
            },
            ResultMutationError::PolicyNeedsConfirmation { statements: vec![] },
            ResultMutationError::Timeout {
                operation: "apply".into(),
            },
            ResultMutationError::Database {
                code: Some("23505".into()),
                message: format!("duplicate key ({secret})"),
                severity: Some("ERROR".into()),
                position: None,
                op_index: Some(2),
            },
            ResultMutationError::Database {
                code: None,
                message: secret.into(),
                severity: None,
                position: None,
                op_index: None,
            },
        ];
        for error in &errors {
            let message = apply_error_message(error);
            assert!(!message.is_empty(), "{error:?}");
            assert!(!message.contains(secret), "{error:?}: {message}");
            assert!(!message.contains("change 2") && !message.contains("Change 2"));
        }
        for index in [6, 7, 8, 18] {
            assert!(
                apply_error_message(&errors[index]).starts_with("Change 3 "),
                "{:?}",
                errors[index]
            );
        }
        assert_eq!(
            apply_error_message(&errors[18]),
            "Change 3 failed: unique violation (SQLSTATE 23505). No changes were applied."
        );
        assert_eq!(
            apply_error_message(&errors[19]),
            "Apply failed: database error. No changes were applied."
        );
        assert!(apply_error_message(&errors[13]).contains("outcome is unknown"));
    }
}
