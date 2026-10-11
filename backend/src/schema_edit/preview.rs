//! The Schema Edit Preview: what a planner will execute, rendered once at
//! review and again at apply. Apply refuses unless the two are equal, so the
//! reviewed SQL is exactly the SQL that runs (ADR-0011: the UI never sends
//! SQL).
use super::change::{DraftIssue, SchemaChangeSet};
use super::model::{Origin, SchemaEngine};
use serde::{Deserialize, Serialize};

/// Statements in one plan.
pub const MAX_SCHEMA_EDIT_STATEMENTS: usize = 512;
/// Encoded bytes of one preview (statements, summaries, notes and changes).
pub const MAX_SCHEMA_EDIT_PREVIEW_BYTES: usize = 1024 * 1024;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PreviewStatement {
    pub sql: String,
    /// One line for the review list.
    pub summary: String,
    pub destructive: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum PreviewGroup {
    /// Runs in one transaction; all or nothing.
    Atomic { statements: Vec<usize> },
    /// Commits on its own (PostgreSQL `CONCURRENTLY`, every MySQL DDL).
    Standalone { statement: usize },
}

impl PreviewGroup {
    pub fn statements(&self) -> Vec<usize> {
        match self {
            Self::Atomic { statements } => statements.clone(),
            Self::Standalone { statement } => vec![*statement],
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum NoteKind {
    /// The table is rewritten (PostgreSQL type change, MySQL COPY).
    Rewrite,
    /// A lock blocks reads or writes while the change runs.
    Lock,
    /// SQLite re-creates the table and copies every row.
    Rebuild,
    /// A failure part way leaves earlier statements committed (MySQL).
    NonAtomic,
    /// Inbound foreign keys, views or triggers may be affected.
    Dependents,
    /// The table is large enough that the change may take a while.
    LargeTable,
    /// The server refuses rather than copying or locking (MySQL online).
    Online,
    Info,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EngineNote {
    pub kind: NoteKind,
    pub text: String,
}

impl EngineNote {
    pub fn new(kind: NoteKind, text: impl Into<String>) -> Self {
        Self {
            kind,
            text: text.into(),
        }
    }
}

/// Planner choices the user makes in the review. Part of the review: apply
/// re-plans with the same options.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct PlanOptions {
    /// MySQL: append `ALGORITHM=INSTANT|INPLACE, LOCK=NONE` so the server
    /// refuses instead of copying or locking.
    pub online_only: bool,
    /// PostgreSQL: new indexes built `CONCURRENTLY` (each standalone).
    pub concurrent_indexes: Vec<Origin>,
    /// PostgreSQL: add foreign keys and checks `NOT VALID` and validate them
    /// in a later statement, so the long scan does not hold the strong lock.
    pub validate_separately: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SchemaEditPreview {
    pub engine: SchemaEngine,
    pub changes: SchemaChangeSet,
    pub statements: Vec<PreviewStatement>,
    pub groups: Vec<PreviewGroup>,
    pub notes: Vec<EngineNote>,
    pub options: PlanOptions,
}

impl SchemaEditPreview {
    pub fn destructive(&self) -> bool {
        self.statements.iter().any(|s| s.destructive) || self.changes.destructive().next().is_some()
    }

    /// Every statement, `;`-terminated, one per line, for display or copy.
    pub fn sql_text(&self) -> String {
        let mut text = String::new();
        for statement in &self.statements {
            text.push_str(statement.sql.trim_end());
            if !statement.sql.trim_end().ends_with(';') {
                text.push(';');
            }
            text.push('\n');
        }
        text
    }

    /// Structural checks every planner output must pass: groups cover every
    /// statement exactly once, in order, within the bounds.
    pub fn check(&self) -> Result<(), PlanError> {
        if self.statements.is_empty() {
            return Err(PlanError::NoChanges);
        }
        if self.statements.len() > MAX_SCHEMA_EDIT_STATEMENTS {
            return Err(PlanError::TooLarge);
        }
        let covered: Vec<usize> = self
            .groups
            .iter()
            .flat_map(PreviewGroup::statements)
            .collect();
        if covered != (0..self.statements.len()).collect::<Vec<_>>()
            || self
                .groups
                .iter()
                .any(|g| matches!(g, PreviewGroup::Atomic { statements } if statements.is_empty()))
        {
            return Err(PlanError::Internal(
                "statement groups do not cover the plan".into(),
            ));
        }
        if self
            .statements
            .iter()
            .any(|s| s.sql.contains('\0') || s.sql.trim().is_empty())
        {
            return Err(PlanError::Internal("empty statement".into()));
        }
        if serde_json::to_vec(self).map_or(usize::MAX, |bytes| bytes.len())
            > MAX_SCHEMA_EDIT_PREVIEW_BYTES
        {
            return Err(PlanError::TooLarge);
        }
        Ok(())
    }
}

/// Groups consecutive transactional statements into atomic groups; every
/// other statement stands alone. `transactional[i]` describes statement `i`.
pub fn group_statements(transactional: &[bool]) -> Vec<PreviewGroup> {
    let mut groups = Vec::new();
    let mut atomic = Vec::new();
    for (index, inside) in transactional.iter().enumerate() {
        if *inside {
            atomic.push(index);
            continue;
        }
        if !atomic.is_empty() {
            groups.push(PreviewGroup::Atomic {
                statements: std::mem::take(&mut atomic),
            });
        }
        groups.push(PreviewGroup::Standalone { statement: index });
    }
    if !atomic.is_empty() {
        groups.push(PreviewGroup::Atomic { statements: atomic });
    }
    groups
}

/// Why a draft cannot be planned.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum PlanError {
    /// The draft has shape problems; nothing was planned.
    Invalid {
        issues: Vec<DraftIssue>,
    },
    /// The baseline is read-only.
    ReadOnly {
        reasons: Vec<String>,
    },
    /// The engine or server cannot make this change (e.g. column reorder on
    /// PostgreSQL). User-facing text.
    Unsupported {
        reason: String,
    },
    /// The draft equals the baseline.
    NoChanges,
    TooLarge,
    /// A planner bug; never shown as the user's fault.
    Internal(String),
}

impl std::fmt::Display for PlanError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Invalid { issues } => match issues.first() {
                Some(issue) if issues.len() == 1 => f.write_str(&issue.message),
                Some(issue) => write!(f, "{} (and {} more)", issue.message, issues.len() - 1),
                None => f.write_str("The draft is invalid"),
            },
            Self::ReadOnly { reasons } => match reasons.first() {
                Some(reason) => f.write_str(reason),
                None => f.write_str("This table is read-only"),
            },
            Self::Unsupported { reason } => f.write_str(reason),
            Self::NoChanges => f.write_str("Nothing to change"),
            Self::TooLarge => f.write_str("The change is too large to review"),
            Self::Internal(reason) => write!(f, "Planner error: {reason}"),
        }
    }
}

impl std::error::Error for PlanError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn groups_consecutive_transactional_statements() {
        assert_eq!(
            group_statements(&[true, true, false, true]),
            vec![
                PreviewGroup::Atomic {
                    statements: vec![0, 1]
                },
                PreviewGroup::Standalone { statement: 2 },
                PreviewGroup::Atomic {
                    statements: vec![3]
                },
            ]
        );
        assert_eq!(group_statements(&[]), vec![]);
    }

    fn preview(statements: usize, groups: Vec<PreviewGroup>) -> SchemaEditPreview {
        SchemaEditPreview {
            engine: SchemaEngine::Postgres,
            changes: SchemaChangeSet::default(),
            statements: (0..statements)
                .map(|i| PreviewStatement {
                    sql: format!("SELECT {i}"),
                    summary: String::new(),
                    destructive: false,
                })
                .collect(),
            groups,
            notes: Vec::new(),
            options: PlanOptions::default(),
        }
    }

    #[test]
    fn check_requires_groups_to_cover_every_statement_once_in_order() {
        assert_eq!(preview(0, vec![]).check(), Err(PlanError::NoChanges));
        assert!(preview(2, group_statements(&[true, false])).check().is_ok());
        assert!(preview(2, vec![PreviewGroup::Standalone { statement: 0 }])
            .check()
            .is_err());
        assert!(preview(
            2,
            vec![
                PreviewGroup::Standalone { statement: 1 },
                PreviewGroup::Standalone { statement: 0 }
            ]
        )
        .check()
        .is_err());
    }

    #[test]
    fn sql_text_terminates_each_statement() {
        let mut value = preview(2, group_statements(&[true, true]));
        value.statements[1].sql = "SELECT 1;".into();
        assert_eq!(value.sql_text(), "SELECT 0;\nSELECT 1;\n");
    }
}
