//! Inline related-row detail with a bounded back stack. Frames own small read-only
//! pages; the grid origin is checked by exact page allocation, never re-targeted.
use crate::{
    data_model::TableQuery,
    fk_navigation::{Refusal, Selection, Target},
    results::encoded_size,
};
use dbunk_lib::backend::{data::*, objects::ForeignKey};
use std::{cell::Cell, rc::Rc, rc::Weak};

pub const DEPTH: usize = 32;
pub const ROW_LIMIT: u32 = 5;
const WORKSPACE_BYTES: usize = 128 * 1024 * 1024;
const STACK_BYTES: usize = 4 * 1024 * 1024;
const FRAME_BYTES: usize = 1024;

/// The exact grid state a detail stack returns to.
pub struct Origin {
    connection: String,
    schema: String,
    table: String,
    query: TableQuery,
    page_number: u32,
    page: Weak<BrowseTableResult>,
    pub cell: (usize, usize),
    key: Option<Vec<String>>,
}
pub struct Current<'a> {
    pub connection: Option<&'a str>,
    pub schema: &'a str,
    pub table: &'a str,
    pub query: &'a TableQuery,
    pub page_number: u32,
    pub page: Option<Rc<BrowseTableResult>>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stale {
    Connection,
    Relation,
    Query,
    Page,
}
impl Stale {
    pub fn message(self) -> &'static str {
        match self {
            Self::Connection => "The original connection is no longer bound; close details",
            Self::Relation => "The original table changed; close details",
            Self::Query => "Filters, sort or page size changed; the original row was not restored",
            Self::Page => {
                "The original page was reloaded or released; the row was not restored to avoid identity drift"
            }
        }
    }
}
impl Origin {
    pub fn capture(current: &Current, cell: (usize, usize)) -> Result<Self, &'static str> {
        let connection = current
            .connection
            .ok_or("The table connection is unavailable")?;
        let page = current.page.as_ref().ok_or("Refresh the table first")?;
        if page.rows.get(cell.0).is_none_or(|row| cell.1 >= row.len()) {
            return Err("Select a loaded source cell");
        }
        Ok(Self {
            connection: connection.into(),
            schema: current.schema.into(),
            table: current.table.into(),
            query: current.query.clone(),
            page_number: current.page_number,
            page: Rc::downgrade(page),
            cell,
            key: page
                .row_identity
                .as_ref()
                .and_then(|rows| rows.get(cell.0).cloned()),
        })
    }
    /// The weak handle keeps only the allocation identity, not the rows.
    pub fn verify(&self, current: &Current) -> Result<(usize, usize), Stale> {
        if current.connection != Some(self.connection.as_str()) {
            return Err(Stale::Connection);
        }
        if current.schema != self.schema || current.table != self.table {
            return Err(Stale::Relation);
        }
        if *current.query != self.query || current.page_number != self.page_number {
            return Err(Stale::Query);
        }
        let page = current.page.as_ref().ok_or(Stale::Page)?;
        if !std::ptr::eq(self.page.as_ptr(), Rc::as_ptr(page))
            || page
                .row_identity
                .as_ref()
                .and_then(|rows| rows.get(self.cell.0))
                != self.key.as_ref()
        {
            return Err(Stale::Page);
        }
        Ok(self.cell)
    }
    fn bytes(&self) -> usize {
        encoded_size(&(
            &self.connection,
            &self.schema,
            &self.table,
            &self.query.filters,
            &self.query.sort,
            &self.key,
        ))
        .saturating_add(FRAME_BYTES)
    }
}

pub enum State {
    Loading,
    Loaded(Rc<BrowseTableResult>),
    NotFound,
    Refused(Refusal),
    Failed(String),
    Cancelled,
}
pub struct Frame {
    pub constraint: String,
    pub schema: String,
    pub table: String,
    pub filters: Vec<BrowseFilter>,
    pub state: State,
    pub cell: (usize, usize),
    bytes: usize,
}
impl Frame {
    fn new(key: &ForeignKey, target: Result<Target, Refusal>) -> Self {
        let (filters, state) = match target {
            Ok(target) => (target.filters, State::Loading),
            Err(refusal) => (Vec::new(), State::Refused(refusal)),
        };
        let mut frame = Self {
            constraint: key.name.clone(),
            schema: key.referenced_schema.clone(),
            table: key.referenced_table.clone(),
            filters,
            state,
            cell: (0, 0),
            bytes: 0,
        };
        frame.bytes = encoded_size(&(
            &frame.constraint,
            &frame.schema,
            &frame.table,
            &frame.filters,
        ))
        .saturating_add(FRAME_BYTES);
        frame
    }
    pub fn page(&self) -> Option<&Rc<BrowseTableResult>> {
        match &self.state {
            State::Loaded(page) => Some(page),
            _ => None,
        }
    }
    pub fn multiple(&self) -> bool {
        self.page()
            .is_some_and(|page| page.rows.len() > 1 || page.page_info.has_more)
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum Back {
    Frame,
    Origin((usize, usize)),
}

pub struct Detail {
    origin: Origin,
    frames: Vec<Frame>,
    generation: u64,
    loading: Option<u64>,
    budget: Rc<Cell<usize>>,
    bytes: usize,
}
impl Detail {
    /// Returns the generation to fetch, or None for an explicit refusal frame.
    pub fn open(
        origin: Origin,
        key: &ForeignKey,
        target: Result<Target, Refusal>,
        budget: Rc<Cell<usize>>,
    ) -> Result<(Self, Option<u64>), &'static str> {
        let mut detail = Self {
            origin,
            frames: Vec::new(),
            generation: 0,
            loading: None,
            budget,
            bytes: 0,
        };
        detail.reserve(detail.origin.bytes())?;
        let generation = detail.push(key, target)?;
        Ok((detail, generation))
    }
    pub fn push(
        &mut self,
        key: &ForeignKey,
        target: Result<Target, Refusal>,
    ) -> Result<Option<u64>, &'static str> {
        if self.loading.is_some() {
            return Err("Wait for the related row or cancel it first");
        }
        if self.frames.len() >= DEPTH {
            return Err("Related-row history is limited to 32 steps; go back first");
        }
        let frame = Frame::new(key, target);
        self.reserve(self.bytes.saturating_add(frame.bytes))?;
        let loading = matches!(frame.state, State::Loading);
        self.frames.push(frame);
        self.generation = self.generation.wrapping_add(1);
        self.loading = loading.then_some(self.generation);
        Ok(self.loading)
    }
    /// A stale generation (popped, cancelled or superseded) is discarded unchanged.
    pub fn settle(
        &mut self,
        generation: u64,
        request_id: u64,
        result: Result<BrowseTableResult, String>,
    ) -> bool {
        if self.loading != Some(generation) {
            return false;
        }
        self.loading = None;
        let state = match result {
            Err(error) => State::Failed(error),
            Ok(page)
                if page.request_id != request_id
                    || page.rows.len() > ROW_LIMIT as usize
                    || page.rows.iter().any(|row| row.len() != page.columns.len()) =>
            {
                State::Failed("The related-row reply did not match its request".into())
            }
            Ok(page) if page.rows.is_empty() => State::NotFound,
            Ok(page) => {
                let bytes = encoded_size(&page);
                let total = self.bytes.saturating_add(bytes);
                match self.reserve(total) {
                    Ok(()) => {
                        let frame = self.frames.last_mut().unwrap();
                        frame.bytes = frame.bytes.saturating_add(bytes);
                        State::Loaded(Rc::new(page))
                    }
                    Err(error) => State::Failed(error.into()),
                }
            }
        };
        self.frames.last_mut().unwrap().state = state;
        true
    }
    pub fn cancel(&mut self) -> bool {
        if self.loading.take().is_none() {
            return false;
        }
        self.generation = self.generation.wrapping_add(1);
        self.frames.last_mut().unwrap().state = State::Cancelled;
        true
    }
    /// Pops one frame, or verifies the exact grid origin before closing.
    pub fn back(&mut self, current: &Current) -> Result<Back, Stale> {
        if self.frames.len() > 1 {
            let frame = self.frames.pop().unwrap();
            self.loading = None;
            self.generation = self.generation.wrapping_add(1);
            let bytes = self.bytes.saturating_sub(frame.bytes);
            self.account(bytes);
            return Ok(Back::Frame);
        }
        self.origin.verify(current).map(Back::Origin)
    }
    pub fn top(&self) -> &Frame {
        self.frames.last().unwrap()
    }
    pub fn depth(&self) -> usize {
        self.frames.len()
    }
    pub fn origin(&self) -> &Origin {
        &self.origin
    }
    pub fn origin_table(&self) -> (&str, &str) {
        (&self.origin.schema, &self.origin.table)
    }
    pub fn next_row(&mut self) {
        let frame = self.frames.last_mut().unwrap();
        if let State::Loaded(page) = &frame.state {
            frame.cell = ((frame.cell.0 + 1) % page.rows.len(), frame.cell.1);
        }
    }
    pub fn move_column(&mut self, forward: bool) {
        let frame = self.frames.last_mut().unwrap();
        if let State::Loaded(page) = &frame.state
            && !page.columns.is_empty()
        {
            let count = page.columns.len();
            let column = if forward {
                (frame.cell.1 + 1) % count
            } else {
                (frame.cell.1 + count - 1) % count
            };
            frame.cell = (frame.cell.0, column);
        }
    }
    /// Uses the loaded related values for chaining; nothing staged is read.
    pub fn selection(&self, connection: &str) -> Result<Selection, &'static str> {
        let frame = self.top();
        let page = frame.page().ok_or("Load a related row first")?;
        Selection::new(page.clone(), connection.into(), frame.cell.0, frame.cell.1)
    }
    pub fn owns(&self, selection: &Selection) -> bool {
        let frame = self.top();
        frame.page().is_some_and(|page| {
            selection.matches(
                page,
                Some(self.origin.connection.as_str()),
                Some(frame.cell),
            )
        })
    }
    fn reserve(&mut self, total: usize) -> Result<(), &'static str> {
        let others = self.budget.get().saturating_sub(self.bytes);
        if total > STACK_BYTES {
            return Err("Related-row details exceed the 4 MiB history limit; go back first");
        }
        if others.saturating_add(total) > WORKSPACE_BYTES {
            return Err("Workspace memory budget is full; clear another result");
        }
        self.account(total);
        Ok(())
    }
    fn account(&mut self, bytes: usize) {
        self.budget.set(
            self.budget
                .get()
                .saturating_sub(self.bytes)
                .saturating_add(bytes),
        );
        self.bytes = bytes;
    }
}
impl Drop for Detail {
    fn drop(&mut self) {
        self.account(0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::data_model::{PageAction, TableDocument};

    fn page(request_id: u64, rows: usize) -> BrowseTableResult {
        BrowseTableResult {
            request_id,
            columns: ["id", "parent"]
                .into_iter()
                .map(|name| BrowseColumn {
                    name: name.into(),
                    cast_type: "text".into(),
                    nullable: true,
                })
                .collect(),
            rows: (0..rows)
                .map(|row| vec![Some(row.to_string()), Some("p".into())])
                .collect(),
            identity: BrowseIdentity {
                kind: BrowseIdentityKind::PrimaryKey,
                columns: vec!["id".into()],
            },
            row_identity: Some((0..rows).map(|row| vec![row.to_string()]).collect()),
            page_info: BrowsePageInfo {
                mode: BrowsePageMode::Keyset,
                page: Some(1),
                has_more: false,
                next_cursor: None,
            },
            count: BrowseCount {
                kind: BrowseCountKind::Unknown,
                value: None,
            },
            inspection: BrowseInspection {
                sql: "SELECT ...".into(),
                params: vec![],
            },
            omitted_rows: 0,
            truncated_cells: 0,
            runtime_ms: 0,
        }
    }
    fn key(name: &str) -> ForeignKey {
        ForeignKey {
            name: name.into(),
            columns: vec!["parent".into()],
            referenced_schema: "public".into(),
            referenced_table: "parent".into(),
            referenced_columns: vec!["id".into()],
            on_update: None,
            on_delete: None,
        }
    }
    fn target() -> Result<Target, Refusal> {
        Ok(Target {
            schema: "public".into(),
            table: "parent".into(),
            filters: vec![BrowseFilter::Comparison {
                column: "id".into(),
                operator: ComparisonOperator::Eq,
                value: "p".into(),
            }],
        })
    }
    struct Grid {
        query: TableQuery,
        page: Rc<BrowseTableResult>,
    }
    impl Grid {
        fn new() -> Self {
            Self {
                query: TableQuery::default(),
                page: Rc::new(page(1, 3)),
            }
        }
        fn current(&self) -> Current<'_> {
            Current {
                connection: Some("connection"),
                schema: "public",
                table: "child",
                query: &self.query,
                page_number: 2,
                page: Some(self.page.clone()),
            }
        }
    }
    fn open(grid: &Grid, budget: &Rc<Cell<usize>>) -> (Detail, u64) {
        let origin = Origin::capture(&grid.current(), (2, 1)).unwrap();
        let (detail, generation) =
            Detail::open(origin, &key("fk"), target(), budget.clone()).unwrap();
        (detail, generation.unwrap())
    }

    #[test]
    fn back_restores_exact_origin_and_refuses_stale_grid_state() {
        let grid = Grid::new();
        let budget = Rc::new(Cell::new(0));
        let (mut detail, generation) = open(&grid, &budget);
        assert!(detail.settle(generation, 7, Ok(page(7, 1))));
        assert_eq!(detail.back(&grid.current()), Ok(Back::Origin((2, 1))));
        let mut other = grid.current();
        other.connection = Some("other");
        assert_eq!(detail.back(&other), Err(Stale::Connection));
        let mut other = grid.current();
        other.table = "child2";
        assert_eq!(detail.back(&other), Err(Stale::Relation));
        let mut other = grid.current();
        other.page_number = 3;
        assert_eq!(detail.back(&other), Err(Stale::Query));
        let mut query = grid.query.clone();
        query.page_size = 50;
        let mut other = grid.current();
        other.query = &query;
        assert_eq!(detail.back(&other), Err(Stale::Query));
        let mut other = grid.current();
        other.page = None;
        assert_eq!(detail.back(&other), Err(Stale::Page));
        // An identical reload is still a different page: staged edits keyed to
        // the original row are never re-targeted by restoring its index.
        let mut other = grid.current();
        other.page = Some(Rc::new((*grid.page).clone()));
        assert_eq!(detail.back(&other), Err(Stale::Page));
        assert_eq!(detail.depth(), 1);
        assert_eq!(detail.back(&grid.current()), Ok(Back::Origin((2, 1))));
    }

    #[test]
    fn released_origin_page_refuses_without_retaining_rows() {
        let mut grid = Grid::new();
        let budget = Rc::new(Cell::new(0));
        let (mut detail, _) = open(&grid, &budget);
        let weak = Rc::downgrade(&grid.page);
        grid.page = Rc::new(page(2, 3));
        assert_eq!(weak.strong_count(), 0);
        assert_eq!(detail.back(&grid.current()), Err(Stale::Page));
    }

    #[test]
    fn stack_is_bounded_and_pops_frames_exactly() {
        let grid = Grid::new();
        let budget = Rc::new(Cell::new(0));
        let (mut detail, generation) = open(&grid, &budget);
        assert!(detail.settle(generation, 1, Ok(page(1, 2))));
        detail.next_row();
        detail.move_column(false);
        assert_eq!(detail.top().cell, (1, 1));
        let first = budget.get();
        for depth in 2..=DEPTH {
            let generation = detail.push(&key(&depth.to_string()), target()).unwrap();
            assert!(detail.settle(generation.unwrap(), 9, Ok(page(9, 1))));
        }
        assert_eq!(detail.depth(), DEPTH);
        assert!(detail.push(&key("over"), target()).is_err());
        assert_eq!(detail.depth(), DEPTH);
        for _ in 2..=DEPTH {
            assert_eq!(detail.back(&grid.current()), Ok(Back::Frame));
        }
        assert_eq!(detail.top().constraint, "fk");
        assert_eq!(detail.top().cell, (1, 1));
        assert!(detail.top().multiple());
        assert_eq!(budget.get(), first);
        drop(detail);
        assert_eq!(budget.get(), 0);
    }

    #[test]
    fn stale_generations_and_cancelled_replies_are_discarded() {
        let grid = Grid::new();
        let budget = Rc::new(Cell::new(0));
        let (mut detail, first) = open(&grid, &budget);
        assert!(detail.push(&key("busy"), target()).is_err());
        assert!(detail.cancel());
        assert!(matches!(detail.top().state, State::Cancelled));
        assert!(!detail.settle(first, 1, Ok(page(1, 1))));
        assert!(matches!(detail.top().state, State::Cancelled));
        let second = detail.push(&key("next"), target()).unwrap().unwrap();
        assert_ne!(first, second);
        assert_eq!(detail.back(&grid.current()), Ok(Back::Frame));
        assert!(!detail.settle(second, 2, Ok(page(2, 1))));
        let third = detail.push(&key("again"), target()).unwrap().unwrap();
        assert!(detail.settle(third, 3, Ok(page(4, 1))));
        assert!(matches!(detail.top().state, State::Failed(_)));
        let fourth = detail.push(&key("many"), target()).unwrap().unwrap();
        assert!(detail.settle(fourth, 5, Ok(page(5, 6))));
        assert!(matches!(detail.top().state, State::Failed(_)));
    }

    #[test]
    fn explicit_states_and_null_refusal_issue_no_request() {
        let grid = Grid::new();
        let budget = Rc::new(Cell::new(0));
        let origin = Origin::capture(&grid.current(), (0, 1)).unwrap();
        let (mut detail, generation) = Detail::open(
            origin,
            &key("fk"),
            Err(Refusal::NoReferencedRow),
            budget.clone(),
        )
        .unwrap();
        assert_eq!(generation, None);
        assert!(matches!(
            detail.top().state,
            State::Refused(Refusal::NoReferencedRow)
        ));
        assert!(detail.selection("connection").is_err());
        let generation = detail.push(&key("a"), target()).unwrap().unwrap();
        assert!(detail.settle(generation, 1, Ok(page(1, 0))));
        assert!(matches!(detail.top().state, State::NotFound));
        let generation = detail.push(&key("b"), target()).unwrap().unwrap();
        assert!(detail.settle(generation, 2, Err("boom".into())));
        assert!(matches!(&detail.top().state, State::Failed(error) if error == "boom"));
        let generation = detail.push(&key("c"), target()).unwrap().unwrap();
        assert!(detail.settle(generation, 3, Ok(page(3, 1))));
        assert!(!detail.top().multiple());
        let selection = detail.selection("connection").unwrap();
        assert!(detail.owns(&selection));
        detail.move_column(true);
        assert!(!detail.owns(&selection));
    }

    #[test]
    fn budget_refuses_oversized_detail_and_releases_on_drop() {
        let grid = Grid::new();
        let budget = Rc::new(Cell::new(WORKSPACE_BYTES));
        let origin = Origin::capture(&grid.current(), (0, 0)).unwrap();
        assert!(Detail::open(origin, &key("fk"), target(), budget.clone()).is_err());
        assert_eq!(budget.get(), WORKSPACE_BYTES);
        budget.set(0);
        let (mut detail, generation) = open(&grid, &budget);
        let mut large = page(1, 1);
        large.rows[0][0] = Some("x".repeat(STACK_BYTES));
        assert!(detail.settle(generation, 1, Ok(large)));
        assert!(matches!(detail.top().state, State::Failed(_)));
        assert!(budget.get() < STACK_BYTES);
        drop(detail);
        assert_eq!(budget.get(), 0);
    }

    #[test]
    fn related_read_shares_sequence_without_touching_the_grid_lane() {
        let mut document = TableDocument::new(
            "connection".into(),
            "tab".into(),
            MutationTable {
                schema: "public".into(),
                table: "child".into(),
            },
        )
        .unwrap();
        let (grid_ticket, grid) = document.browse(PageAction::First, false).unwrap();
        let filters = target().unwrap().filters;
        assert!(
            document
                .related("public", "parent", filters.clone(), ROW_LIMIT)
                .is_err()
        );
        assert!(
            document
                .receive_page(grid_ticket, page(grid.request_id, 3))
                .unwrap()
        );
        let shared = document.shared_result().unwrap();
        let (ticket, payload) = document
            .related("public", "parent", filters.clone(), ROW_LIMIT)
            .unwrap();
        assert!(payload.request_id > grid.request_id);
        assert_eq!(payload.filters, filters);
        assert!(payload.sort.is_empty());
        assert_eq!(payload.page_size, ROW_LIMIT);
        assert_eq!(
            (payload.schema.as_str(), payload.table.as_str()),
            ("public", "parent")
        );
        assert!(
            !document
                .receive_page(ticket, page(payload.request_id, 1))
                .unwrap()
        );
        assert!(!document.failed(ticket));
        assert!(Rc::ptr_eq(&document.shared_result().unwrap(), &shared));
        assert!(document.page_is_current());
        assert!(document.query().filters.is_empty());
        assert!(
            document
                .related("public", "parent", vec![], ROW_LIMIT)
                .is_err()
        );
        let (_, next) = document.browse(PageAction::First, false).unwrap();
        assert!(next.request_id > payload.request_id);
    }
}
