//! Request identity, paging and bounded table-result retention.
use super::*;
use std::rc::Rc;

#[derive(Clone, PartialEq, Eq)]
pub struct TableQuery {
    pub filters: Vec<BrowseFilter>,
    pub sort: Vec<BrowseSortKey>,
    pub page_size: u32,
}
impl Default for TableQuery {
    fn default() -> Self {
        Self {
            filters: Vec::new(),
            sort: Vec::new(),
            page_size: 100,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RequestTicket {
    owner: Uuid,
    pub(super) sequence: u64,
}
#[derive(Clone, Copy)]
pub enum PageAction {
    First,
    Next,
    Previous,
    Jump(u32),
    Last,
    Refresh,
}

/// A retained page belongs to one query revision. A failed replacement keeps
/// that page available for inspection but cannot navigate it as the new query.
pub struct TableDocument {
    owner: Uuid,
    connection: String,
    tab: String,
    relation: MutationTable,
    query: TableQuery,
    sequence: u64,
    pending: Option<(RequestTicket, Option<u32>)>,
    result: Option<Rc<BrowseTableResult>>,
    retained_bytes: usize,
    page: u32,
    current: bool,
    exact_count: Option<BrowseExactCountResult>,
}
impl TableDocument {
    pub fn new(
        connection: String,
        tab: String,
        relation: MutationTable,
    ) -> Result<Self, ModelError> {
        if [&connection, &tab, &relation.schema, &relation.table]
            .iter()
            .any(|s| s.is_empty() || s.len() > 256 || s.contains('\0'))
        {
            return Err(ModelError::InvalidInput);
        }
        Ok(Self {
            owner: Uuid::new_v4(),
            connection,
            tab,
            relation,
            query: TableQuery::default(),
            sequence: 0,
            pending: None,
            result: None,
            retained_bytes: 0,
            page: 1,
            current: false,
            exact_count: None,
        })
    }
    /// Restores intent without results, cursors, pending requests or a connection.
    pub fn restore(
        connection: String,
        tab: String,
        saved: &WorkspaceTableState,
    ) -> Result<Self, ModelError> {
        saved.validate().map_err(|_| ModelError::InvalidInput)?;
        let mut document = Self::new(
            connection,
            tab,
            MutationTable {
                schema: saved.schema.clone(),
                table: saved.table.clone(),
            },
        )?;
        document.set_query(
            TableQuery {
                filters: saved.filters.clone(),
                sort: saved.sort.clone(),
                page_size: saved.page_size,
            },
            "",
        )?;
        Ok(document)
    }
    pub fn snapshot(
        &self,
        draft: Option<WorkspaceMutationDraft>,
    ) -> Result<WorkspaceTableState, ModelError> {
        let saved = WorkspaceTableState {
            schema: self.relation.schema.clone(),
            table: self.relation.table.clone(),
            filters: self.query.filters.clone(),
            sort: self.query.sort.clone(),
            page_size: self.query.page_size,
            draft,
        };
        saved.validate().map_err(|_| ModelError::InvalidInput)?;
        Ok(saved)
    }
    pub fn query(&self) -> &TableQuery {
        &self.query
    }
    pub fn result(&self) -> Option<&BrowseTableResult> {
        self.result.as_deref()
    }
    /// The grid shares this allocation; it must release its handle on replacement
    /// or clear so workspace retention follows the document's cached byte count.
    pub fn shared_result(&self) -> Option<Rc<BrowseTableResult>> {
        self.result.clone()
    }
    pub fn retained_bytes(&self) -> usize {
        self.retained_bytes
    }
    pub fn page(&self) -> u32 {
        self.page
    }
    pub fn page_is_current(&self) -> bool {
        self.current
    }
    pub fn exact_count(&self) -> Option<&BrowseExactCountResult> {
        self.exact_count.as_ref()
    }

    /// Typed and raw predicates coexist in the baseline. UI filter-mode choice
    /// changes editing presentation, not which already-applied predicates run.
    pub fn set_query(&mut self, mut query: TableQuery, raw: &str) -> Result<(), ModelError> {
        if raw.len() > QUERY_BYTES {
            return Err(ModelError::Budget);
        }
        if !(1..=1000).contains(&query.page_size) {
            return Err(ModelError::InvalidInput);
        }
        if !raw.trim().is_empty() {
            query.filters.push(BrowseFilter::RawSql {
                text: raw.trim().into(),
            });
        }
        if query.filters.len() > 256 || query.sort.len() > 256 {
            return Err(ModelError::Budget);
        }
        if crate::results::encoded_size(&(&query.filters, &query.sort, query.page_size))
            > QUERY_BYTES
        {
            return Err(ModelError::Budget);
        }
        if self.query != query {
            self.query = query;
            self.invalidate();
        }
        Ok(())
    }
    /// Mirrors the baseline header cycle: ascending, descending, removed.
    /// Appending preserves the order and NULL placement of other sort keys.
    pub fn cycle_sort(&mut self, column: &str, append: bool) -> Result<(), ModelError> {
        if column.is_empty() || column.len() > 256 {
            return Err(ModelError::InvalidInput);
        }
        let mut query = self.query.clone();
        let index = query.sort.iter().position(|key| key.column == column);
        if !append && (index != Some(0) || query.sort.len() != 1) {
            query.sort.clear();
        } else if let Some(index) = index {
            if query.sort[index].direction == BrowseSortDirection::Asc {
                query.sort[index].direction = BrowseSortDirection::Desc;
                if !append {
                    query.sort[index].nulls = BrowseNulls::Default;
                }
            } else {
                query.sort.remove(index);
            }
            return self.set_query(query, "");
        }
        query.sort.push(BrowseSortKey {
            column: column.into(),
            direction: BrowseSortDirection::Asc,
            nulls: BrowseNulls::Default,
        });
        self.set_query(query, "")
    }
    /// Reconnect/structure invalidation fences both count and page replies.
    pub fn invalidate(&mut self) {
        self.pending = None;
        self.current = false;
        self.exact_count = None;
    }
    /// Releases retained data while keeping the relation and query settings.
    pub fn clear_page(&mut self) {
        self.invalidate();
        self.result = None;
        self.retained_bytes = 0;
        self.page = 1;
    }
    fn first(&self) -> BrowsePageRequest {
        if self.query.sort.is_empty() {
            BrowsePageRequest::Keyset { cursor: None }
        } else {
            BrowsePageRequest::Offset { page: 1 }
        }
    }
    fn ticket(&mut self, page: Option<u32>) -> Result<RequestTicket, ModelError> {
        self.sequence = self
            .sequence
            .checked_add(1)
            .ok_or(ModelError::Unavailable)?;
        let ticket = RequestTicket {
            owner: self.owner,
            sequence: self.sequence,
        };
        self.pending = Some((ticket, page));
        Ok(ticket)
    }
    pub fn browse(
        &mut self,
        action: PageAction,
        refresh_structure: bool,
    ) -> Result<(RequestTicket, BrowseTableDataPayload), ModelError> {
        let (page, page_request) = match action {
            PageAction::First => (1, self.first()),
            PageAction::Refresh if !self.current => (1, self.first()),
            PageAction::Jump(page) if page > 0 => (page, BrowsePageRequest::Offset { page }),
            PageAction::Jump(_) => return Err(ModelError::InvalidInput),
            _ if !self.current => return Err(ModelError::Unavailable),
            PageAction::Next => {
                let result = self.result.as_ref().ok_or(ModelError::Unavailable)?;
                if !result.page_info.has_more {
                    return Err(ModelError::Unavailable);
                }
                let page = self.page.checked_add(1).ok_or(ModelError::Unavailable)?;
                let request = match (&result.page_info.mode, &result.page_info.next_cursor) {
                    (BrowsePageMode::Keyset, Some(cursor)) => BrowsePageRequest::Keyset {
                        cursor: Some(cursor.clone()),
                    },
                    _ => BrowsePageRequest::Offset { page },
                };
                (page, request)
            }
            PageAction::Previous => {
                if self.page <= 1 {
                    return Err(ModelError::Unavailable);
                }
                let page = self.page - 1;
                (page, BrowsePageRequest::Offset { page })
            }
            PageAction::Last => {
                let count = self
                    .exact_count
                    .as_ref()
                    .map(|c| c.value)
                    .or_else(|| {
                        self.result.as_ref().and_then(|r| {
                            (r.count.kind != BrowseCountKind::Unknown)
                                .then_some(r.count.value)
                                .flatten()
                        })
                    })
                    .ok_or(ModelError::Unavailable)?;
                let page = u32::try_from(count.div_ceil(u64::from(self.query.page_size)).max(1))
                    .map_err(|_| ModelError::Unavailable)?;
                (page, BrowsePageRequest::Offset { page })
            }
            PageAction::Refresh => (self.page, BrowsePageRequest::Offset { page: self.page }),
        };
        let ticket = self.ticket(Some(page))?;
        if refresh_structure {
            self.exact_count = None;
        }
        Ok((
            ticket,
            BrowseTableDataPayload {
                connection_id: self.connection.clone(),
                tab_id: self.tab.clone(),
                request_id: ticket.sequence,
                schema: self.relation.schema.clone(),
                table: self.relation.table.clone(),
                filters: self.query.filters.clone(),
                sort: self.query.sort.clone(),
                page_request,
                page_size: self.query.page_size,
                count_policy: BrowseCountPolicy::Estimated,
                refresh_structure,
            },
        ))
    }
    /// Browse/count share the service's superseding request sequence.
    pub fn count(&mut self) -> Result<(RequestTicket, CountTableBrowseRowsPayload), ModelError> {
        let ticket = self.ticket(None)?;
        Ok((
            ticket,
            CountTableBrowseRowsPayload {
                connection_id: self.connection.clone(),
                tab_id: self.tab.clone(),
                request_id: ticket.sequence,
                schema: self.relation.schema.clone(),
                table: self.relation.table.clone(),
                filters: self.query.filters.clone(),
            },
        ))
    }
    pub fn receive_page(
        &mut self,
        ticket: RequestTicket,
        result: BrowseTableResult,
    ) -> Result<bool, ModelError> {
        self.receive_page_with_limit(ticket, result, PAGE_BYTES)
    }
    /// The allowance is the replacement ceiling after accounting for other
    /// workspace pages. Serialize once for admission, then cache that exact
    /// size and share the accepted page with the grid without cloning its rows.
    pub fn receive_page_with_limit(
        &mut self,
        ticket: RequestTicket,
        result: BrowseTableResult,
        workspace_allowance: usize,
    ) -> Result<bool, ModelError> {
        let Some((expected, Some(page))) = self.pending else {
            return Ok(false);
        };
        if expected != ticket {
            return Ok(false);
        }
        self.pending = None;
        if result.request_id != ticket.sequence
            || result.rows.len() > self.query.page_size as usize
            || result
                .rows
                .iter()
                .any(|row| row.len() != result.columns.len())
            || result.row_identity.as_ref().is_some_and(|rows| {
                rows.len() != result.rows.len()
                    || rows
                        .iter()
                        .any(|row| row.len() != result.identity.columns.len())
            })
            || result.page_info.page == Some(0)
        {
            return Err(ModelError::InvalidReply);
        }
        let retained_bytes = crate::results::encoded_size(&result)
            .saturating_add(result.columns.len().saturating_mul(size_of::<f32>()));
        if retained_bytes > PAGE_BYTES.min(workspace_allowance) {
            return Err(ModelError::Budget);
        }
        self.page = result.page_info.page.unwrap_or(page);
        self.result = Some(Rc::new(result));
        self.retained_bytes = retained_bytes;
        self.current = true;
        Ok(true)
    }
    pub fn receive_count(
        &mut self,
        ticket: RequestTicket,
        result: BrowseExactCountResult,
    ) -> Result<bool, ModelError> {
        if self.pending != Some((ticket, None)) {
            return Ok(false);
        }
        self.pending = None;
        if result.request_id != ticket.sequence || result.kind != BrowseCountKind::Exact {
            return Err(ModelError::InvalidReply);
        }
        self.exact_count = Some(result);
        Ok(true)
    }
    /// Error/cancel only settles its own request and never empties a good page.
    /// InvalidCursor is surfaced; the caller may explicitly request First once.
    pub fn failed(&mut self, ticket: RequestTicket) -> bool {
        if self.pending.is_some_and(|(expected, _)| expected == ticket) {
            self.pending = None;
            true
        } else {
            false
        }
    }
}
