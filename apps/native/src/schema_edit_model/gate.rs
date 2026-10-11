//! The review dialog's pure rules: phase, apply gate, escape handling,
//! button label, connection policy, planner option toggles and the text for
//! outcomes, notes and SQL. Same shape as the row-change review
//! (`table_changes/review.rs`); the backend stays the enforcement boundary
//! (ADR-0004) and its NeedsConfirmation always wins over this gate.
use crate::apply_flow::ApplyFlow;
use crate::data_model::{ConfirmStyle, TablePolicy};
use dbunk_lib::backend::schema_edit::{
    ChangeKind, Destructive, NoteKind, Origin, PlanOptions, PreviewGroup, SchemaChangeSet,
    SchemaEditOutcome, SchemaEditPreview, SchemaEngine, SchemaObject,
};
use dbunk_lib::backend::{DevelopmentConnection, DevelopmentEngineConnection};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReviewPhase {
    /// Reviewed; Apply follows the gate.
    Ready,
    /// The attempt is being journaled before the first dispatch.
    Saving,
    /// Journaled again before the one automatic confirmation.
    AutoConfirming,
    /// Sent; only the backend can settle it now.
    Applying,
    /// The backend wants a confirmation the user must type.
    NeedsTyped,
    /// Planning failed (after an option change); nothing can be applied.
    Failed,
    /// The backend settled the attempt; the outcome is shown.
    Settled,
}

/// A pending apply wins over everything else; without one, a settled
/// outcome wins over a stale review.
pub fn review_phase<T>(
    flow: Option<&ApplyFlow<T>>,
    auto_confirms: u32,
    reviewed: bool,
    settled: bool,
) -> ReviewPhase {
    if let Some(flow) = flow {
        return if flow.confirming() {
            ReviewPhase::NeedsTyped
        } else if flow.dispatched() {
            ReviewPhase::Applying
        } else if auto_confirms > 0 {
            ReviewPhase::AutoConfirming
        } else {
            ReviewPhase::Saving
        };
    }
    if settled {
        ReviewPhase::Settled
    } else if reviewed {
        ReviewPhase::Ready
    } else {
        ReviewPhase::Failed
    }
}

/// Apply is enabled only on a fresh review (or an escalated confirmation),
/// never on a read-only connection, and typed policies need the exact word.
pub fn apply_enabled(
    style: Option<ConfirmStyle>,
    typed_matched: bool,
    phase: ReviewPhase,
    read_only: bool,
) -> bool {
    if read_only {
        return false;
    }
    match (phase, style) {
        (_, None) => false,
        (ReviewPhase::Ready, Some(ConfirmStyle::Plain)) => true,
        (ReviewPhase::Ready, Some(ConfirmStyle::Typed)) | (ReviewPhase::NeedsTyped, Some(_)) => {
            typed_matched
        }
        _ => false,
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReviewEscape {
    /// Nothing was sent: close (the target goes back to the editor).
    Close,
    /// The journal or confirmation is not dispatched yet: revoke it.
    CancelBeforeDispatch,
    /// Dispatched: Escape never cancels a schema change.
    Ignore,
}

pub fn review_escape(phase: ReviewPhase) -> ReviewEscape {
    match phase {
        ReviewPhase::Ready | ReviewPhase::Failed | ReviewPhase::Settled => ReviewEscape::Close,
        ReviewPhase::Saving | ReviewPhase::AutoConfirming | ReviewPhase::NeedsTyped => {
            ReviewEscape::CancelBeforeDispatch
        }
        ReviewPhase::Applying => ReviewEscape::Ignore,
    }
}

/// Footer label and whether it takes the danger styling (typed policy with
/// destructive changes).
pub fn apply_label(
    policy: &TablePolicy,
    phase: ReviewPhase,
    objects: usize,
    destructive: bool,
) -> (String, bool) {
    let style = policy.confirm_style();
    let label = match phase {
        ReviewPhase::Saving | ReviewPhase::AutoConfirming | ReviewPhase::Applying => {
            "Applying…".to_owned()
        }
        ReviewPhase::NeedsTyped => "Confirm and apply".to_owned(),
        _ if style == Some(ConfirmStyle::Plain) && policy.expects_backend_confirmation() => {
            "Confirm and apply".to_owned()
        }
        _ => format!(
            "Apply {objects} change{}",
            if objects == 1 { "" } else { "s" }
        ),
    };
    (label, style == Some(ConfirmStyle::Typed) && destructive)
}

/// The write policy of connection `id`, resolved like the row editor does,
/// for every engine that stores one. A missing record gets the strictest
/// `UNKNOWN` policy; the backend decides either way.
pub fn policy_for(records: &[DevelopmentConnection], id: &str) -> TablePolicy {
    let Some(record) = records.iter().find(|record| record.id == id) else {
        return TablePolicy::UNKNOWN;
    };
    if let Some(postgres) = &record.postgres {
        return TablePolicy::resolve(postgres.environment, postgres.safe_mode, postgres.read_only);
    }
    match &record.settings {
        Some(DevelopmentEngineConnection::PostgreSQL(postgres)) => {
            TablePolicy::resolve(postgres.environment, postgres.safe_mode, postgres.read_only)
        }
        Some(DevelopmentEngineConnection::MySQL(mysql)) => {
            TablePolicy::resolve(mysql.environment, mysql.safe_mode, mysql.read_only)
        }
        Some(DevelopmentEngineConnection::SQLite(sqlite)) => {
            TablePolicy::resolve(sqlite.environment, sqlite.safe_mode, sqlite.read_only)
        }
        _ => TablePolicy::UNKNOWN,
    }
}

/// Planner options a user can change in the review. Each change re-plans.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PlanToggle {
    /// MySQL: refuse instead of copying or locking.
    OnlineOnly,
    /// PostgreSQL: build new indexes `CONCURRENTLY`, each standalone.
    ConcurrentIndexes,
    /// PostgreSQL: add foreign keys and checks `NOT VALID`, validate later.
    ValidateSeparately,
}

impl PlanToggle {
    pub fn label(self) -> &'static str {
        match self {
            Self::OnlineOnly => "Online only (ALGORITHM=INSTANT|INPLACE, LOCK=NONE)",
            Self::ConcurrentIndexes => "Build new indexes concurrently",
            Self::ValidateSeparately => "Validate new constraints separately",
        }
    }
}

/// The toggles that apply to this engine and change set.
pub fn plan_toggles(engine: SchemaEngine, changes: &SchemaChangeSet) -> Vec<PlanToggle> {
    match engine {
        SchemaEngine::MySql => vec![PlanToggle::OnlineOnly],
        SchemaEngine::Postgres => {
            let mut toggles = Vec::new();
            if !concurrent_candidates(changes).is_empty() {
                toggles.push(PlanToggle::ConcurrentIndexes);
            }
            if changes.changes.iter().any(|change| {
                matches!(
                    change.object,
                    SchemaObject::ForeignKey | SchemaObject::Check
                ) && (change.kind == ChangeKind::Added
                    || change.destructive == Some(Destructive::Recreate))
            }) {
                toggles.push(PlanToggle::ValidateSeparately);
            }
            toggles
        }
        SchemaEngine::Sqlite => Vec::new(),
    }
}

/// Indexes the planner creates: new ones and re-created ones.
pub fn concurrent_candidates(changes: &SchemaChangeSet) -> Vec<Origin> {
    let mut origins = Vec::new();
    for change in &changes.changes {
        let creates =
            change.kind == ChangeKind::Added || change.destructive == Some(Destructive::Recreate);
        if change.object == SchemaObject::Index
            && creates
            && let Some(origin) = change.origin
            && !origins.contains(&origin)
        {
            origins.push(origin);
        }
    }
    origins
}

pub fn toggle_on(options: &PlanOptions, toggle: PlanToggle) -> bool {
    match toggle {
        PlanToggle::OnlineOnly => options.online_only,
        PlanToggle::ConcurrentIndexes => !options.concurrent_indexes.is_empty(),
        PlanToggle::ValidateSeparately => options.validate_separately,
    }
}

/// Flips one toggle and returns the options to re-plan with.
pub fn toggle(options: &PlanOptions, toggle: PlanToggle, changes: &SchemaChangeSet) -> PlanOptions {
    let mut next = options.clone();
    match toggle {
        PlanToggle::OnlineOnly => next.online_only = !next.online_only,
        PlanToggle::ConcurrentIndexes => {
            next.concurrent_indexes = if next.concurrent_indexes.is_empty() {
                concurrent_candidates(changes)
            } else {
                Vec::new()
            }
        }
        PlanToggle::ValidateSeparately => next.validate_separately = !next.validate_separately,
    }
    next
}

pub fn note_label(kind: NoteKind) -> &'static str {
    match kind {
        NoteKind::Rewrite => "Table rewrite",
        NoteKind::Lock => "Lock",
        NoteKind::Rebuild => "Rebuild",
        NoteKind::NonAtomic => "Not atomic",
        NoteKind::Dependents => "Dependents",
        NoteKind::LargeTable => "Large table",
        NoteKind::Online => "Online only",
        NoteKind::Info => "Note",
    }
}

/// Notes that warn rather than inform get the caution tint.
pub fn note_warns(kind: NoteKind) -> bool {
    !matches!(kind, NoteKind::Info | NoteKind::Online)
}

/// Every statement in execution order, with a comment heading each group so
/// the read-only editor shows which statements commit together.
pub fn sql_document(preview: &SchemaEditPreview) -> String {
    let mut text = String::new();
    let count = preview.groups.len();
    for (number, group) in preview.groups.iter().enumerate() {
        if number > 0 {
            text.push('\n');
        }
        let heading = match group {
            PreviewGroup::Atomic { statements } if statements.len() == 1 => {
                "runs in one transaction".to_owned()
            }
            PreviewGroup::Atomic { statements } => {
                format!("{} statements in one transaction", statements.len())
            }
            PreviewGroup::Standalone { .. } => "commits on its own".to_owned(),
        };
        if count > 1 || matches!(group, PreviewGroup::Standalone { .. }) {
            text.push_str(&format!("-- Group {} of {count}: {heading}\n", number + 1));
        }
        for index in group.statements() {
            if let Some(statement) = preview.statements.get(index) {
                let sql = statement.sql.trim_end();
                text.push_str(sql);
                if !sql.ends_with(';') {
                    text.push(';');
                }
                text.push('\n');
            }
        }
    }
    text
}

/// An outcome as user-facing text. `error` picks the banner styling.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OutcomeText {
    pub text: String,
    pub error: bool,
}

pub fn outcome_text(outcome: &SchemaEditOutcome, statements: usize) -> OutcomeText {
    let (text, error) = match outcome {
        SchemaEditOutcome::Applied { runtime_ms } => (
            format!("Applied in {runtime_ms} ms. The structure was verified after the change."),
            false,
        ),
        SchemaEditOutcome::NotDispatched { reason } => (
            format!("Not applied: {reason}. Nothing reached the server."),
            true,
        ),
        SchemaEditOutcome::RolledBack { reason } => (
            format!("Rolled back: {reason}. The table is unchanged."),
            true,
        ),
        SchemaEditOutcome::Stopped {
            committed,
            failed_statement,
            reason,
        } => (
            format!(
                "Stopped at statement {} of {statements}: {reason}. {} The table is partly changed; reload the structure before another change.",
                failed_statement + 1,
                match committed {
                    0 => "No statement committed.".to_owned(),
                    1 => "Statement 1 committed.".to_owned(),
                    n => format!("Statements 1 to {n} committed."),
                }
            ),
            true,
        ),
        SchemaEditOutcome::OutcomeUnknown { reason } => (
            format!(
                "Outcome unknown: {reason}. Inspect the table and reload its structure before another attempt. This change is never replayed."
            ),
            true,
        ),
    };
    OutcomeText { text, error }
}
