//! Table browsing and query results share staged-write mechanics, but never
//! exchange analysis authority or invent a browse page for a query result.
use super::*;
use crate::query_result::{Provenance, QueryRows};

pub(super) enum ChangeSource {
    Table(MutationTable),
    Query {
        provenance: Rc<Provenance>,
        rows: Option<Rc<QueryRows>>,
    },
}
impl ChangeSource {
    pub(super) fn relation(&self) -> Option<&MutationTable> {
        match self {
            Self::Table(relation) => Some(relation),
            Self::Query { .. } => None,
        }
    }
    pub(super) fn analysis_source(&self) -> Result<AnalyzeSource, String> {
        match self {
            Self::Table(relation) => Ok(AnalyzeSource::Relation {
                schema: relation.schema.clone(),
                table: relation.table.clone(),
            }),
            Self::Query { provenance, .. } => provenance
                .analysis_source()
                .map_err(|error| error.to_string()),
        }
    }
    pub(super) fn compatible(
        &self,
        page: Option<&BrowseTableResult>,
        analysis: &AnalyzeResultSetResult,
    ) -> bool {
        match self {
            Self::Table(_) => page.is_some_and(|page| {
                page.columns.len() == analysis.columns.len()
                    && page
                        .columns
                        .iter()
                        .zip(&analysis.columns)
                        .all(|(page, column)| {
                            page.name == column.name && page.cast_type == column.cast_type
                        })
            }),
            Self::Query { provenance, rows } => rows.as_ref().is_none_or(|rows| {
                Rc::ptr_eq(provenance, &rows.origin.provenance) && rows.compatible(analysis)
            }),
        }
    }
}
impl TableChanges {
    pub fn new_query(
        provenance: Rc<Provenance>,
        rows: Option<Rc<QueryRows>>,
        draft: Option<WorkspaceMutationDraft>,
        budget: Rc<Cell<usize>>,
    ) -> Self {
        Self::create(ChangeSource::Query { provenance, rows }, draft, budget)
    }
    /// Called only after workspace serialization has admitted its bounded copy.
    pub fn query_snapshot(&self) -> Option<dbunk_lib::backend::WorkspaceQueryChanges> {
        let provenance = self.query_provenance()?;
        let draft = self.snapshot()?;
        Some(dbunk_lib::backend::WorkspaceQueryChanges {
            source: provenance.source.clone(),
            draft,
        })
    }
    pub fn query_snapshot_bytes(&self) -> usize {
        let draft = self.snapshot_bytes();
        if draft == 0 {
            return 0;
        }
        self.query_provenance().map_or(0, |provenance| {
            // Exact object delimiters and field names, plus borrowed payload sizes.
            b"{\"source\":,\"draft\":}"
                .len()
                .saturating_add(crate::results::encoded_size(&provenance.source))
                .saturating_add(draft)
        })
    }
    pub fn is_query(&self) -> bool {
        matches!(self.source, ChangeSource::Query { .. })
    }
    pub fn query_provenance(&self) -> Option<&Rc<Provenance>> {
        match &self.source {
            ChangeSource::Query { provenance, .. } => Some(provenance),
            _ => None,
        }
    }
    pub fn has_intent(&self) -> bool {
        self.unrestored.is_some()
            || self
                .draft
                .as_ref()
                .is_some_and(|draft| !draft.is_empty() || draft.outcome_unknown())
    }
    pub fn query_connected(&mut self, controls: TableControls, cx: &mut Context<Self>) {
        self.controls = Some(controls);
        self.analyze(cx);
    }
    /// Applying changes invalidates the old values. Explicit rerun remains a
    /// query-session action; mutation success never re-executes captured SQL.
    pub fn clear_query_rows(&mut self, cx: &mut Context<Self>) {
        self.edit = None;
        if let ChangeSource::Query { rows, .. } = &mut self.source {
            *rows = None;
        }
        self.finish_work();
        cx.notify();
    }
}

pub(super) enum CapturedRows {
    Table(Rc<BrowseTableResult>),
    Query(Rc<QueryRows>),
}
impl CapturedRows {
    pub(super) fn row(&self, index: usize) -> Option<&[Option<String>]> {
        match self {
            Self::Table(page) => page.rows.get(index).map(Vec::as_slice),
            Self::Query(rows) => rows.rows.get(index).map(AsRef::as_ref),
        }
    }
    pub(super) fn hidden(&self, index: usize) -> Option<&[String]> {
        match self {
            Self::Table(page) => page.row_identity.as_ref()?.get(index).map(Vec::as_slice),
            Self::Query(_) => None,
        }
    }
    pub(super) fn truncated(&self) -> bool {
        matches!(self, Self::Table(page) if page.truncated_cells > 0)
    }
}
impl TableChanges {
    pub(super) fn captured_rows(&self) -> Option<CapturedRows> {
        match &self.source {
            ChangeSource::Table(_) => self.page.clone().map(CapturedRows::Table),
            ChangeSource::Query { rows, .. } => rows.clone().map(CapturedRows::Query),
        }
    }
}
