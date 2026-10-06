//! Pure preparation for Plan 028, not a native service caller. The runtime must
//! own the opaque DataDocument, cancellation and workspace-wide reservations.
//! Model limits refuse changes before publication; existing drafts are retained.
use dbunk_lib::backend::data::*;
use dbunk_lib::backend::{
    WorkspaceApplyState, WorkspaceMutationDraft, WorkspaceStagedChange, WorkspaceTableState,
};
use uuid::Uuid;

const PAGE_BYTES: usize = 32 * 1024 * 1024;
const QUERY_BYTES: usize = 64 * 1024;
const ANALYSIS_BYTES: usize = 256 * 1024;
const DRAFT_BYTES: usize = 4 * 1024 * 1024;
const CHANGE_LIMIT: usize = 128;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModelError {
    Budget,
    InvalidInput,
    InvalidReply,
    Unavailable,
    Stale,
    Applying,
    AmbiguousIdentity,
    OutcomeUnknown,
}

mod browse;
mod mutations;
mod review_diff;
mod write_safety;
pub use browse::{PageAction, RequestTicket, TableDocument, TableQuery};
pub use mutations::{
    Advance, ApplyResolution, ApplyTicket, BulkOutcome, BulkRow, CellRef, DraftOverlay, EditSeed,
    InsertCell, InsertRow, MutationDraft, OVERLAY_TEXT_BYTES, OverlayKey, OverlayValue, ReviewPlan,
    RowMark,
};
pub use review_diff::{
    DIFF_TEXT_CHARS, DiffCell, DiffChange, DiffKind, DiffValue, apply_error_message, diff_summary,
    format_param, review_diff,
};
pub use write_safety::{
    ConfirmStyle, ConfirmationStep, EffectiveSafeMode, Preconfirmation, TablePolicy,
    on_needs_confirmation, preconfirmation,
};

#[cfg(test)]
#[path = "data_model_tests.rs"]
mod tests;
