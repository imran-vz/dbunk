//! Profile-local successful override records. Captures never contact a database
//! connection, establish a complete audit, or authorize replay of an operation.
use dbunk_lib::backend::safety_audit::{
    MAX_AUDIT_PAGE_BYTES, SAFETY_AUDIT_GLOBAL_RETENTION, SafetyAuditCursor, SafetyAuditLimit,
    SafetyAuditPage,
};
use std::{cell::Cell, rc::Rc};

const WORKSPACE_BYTES: usize = 128 * 1024 * 1024;
const RETAINED_BYTES: usize = 1024 * 1024;

pub struct Capture {
    page: SafetyAuditPage,
    budget: Rc<Cell<usize>>,
}

impl Capture {
    /// The caller retains its previous capture until this admission succeeds.
    /// Identity is checked against the request, not the currently selected tab.
    pub fn new(
        page: SafetyAuditPage,
        expected_connection: &str,
        budget: Rc<Cell<usize>>,
    ) -> Result<Self, &'static str> {
        if page.connection_id != expected_connection {
            return Err(
                "Safety audit reply does not match the requested connection; previous page retained",
            );
        }
        if page
            .checked_heap_bytes()
            .is_none_or(|bytes| bytes > MAX_AUDIT_PAGE_BYTES)
            || page
                .encoded_bytes()
                .is_none_or(|bytes| bytes > MAX_AUDIT_PAGE_BYTES)
        {
            return Err(
                "Safety audit page exceeds its limits or has invalid identities; previous page retained",
            );
        }
        if RETAINED_BYTES > WORKSPACE_BYTES.saturating_sub(budget.get()) {
            return Err(
                "Safety audit needs 1 MiB of shared allowance; clear a capture or close another tool",
            );
        }
        // The 256 KiB page and bounded selected-row strings/cursor clones fit
        // within this lease. No row payloads are cloned into a second model.
        budget.set(budget.get() + RETAINED_BYTES);
        Ok(Self { page, budget })
    }

    pub fn connection(&self) -> &str {
        &self.page.connection_id
    }

    pub fn count(&self) -> usize {
        self.page.rows.len()
    }

    pub fn key(&self, index: usize) -> Option<i64> {
        self.page.rows.get(index).map(|row| row.id)
    }

    /// A hidden or expired ID has no replacement selection. The view must wait
    /// for an explicit user selection rather than silently selecting row zero.
    pub fn index_for_key(&self, key: i64) -> Option<usize> {
        self.page.rows.iter().position(|row| row.id == key)
    }

    pub fn can_continue(&self) -> bool {
        self.page.next_cursor.is_some()
    }

    pub fn next_cursor(&self) -> Option<SafetyAuditCursor> {
        self.page.next_cursor.clone()
    }

    pub fn row_label(&self, index: usize) -> Option<String> {
        let row = self.page.rows.get(index)?;
        Some(format!(
            "{} · {} · {}",
            row.occurred_at,
            row.command,
            classes(&row.classes)
        ))
    }

    pub fn details(&self, index: usize) -> Option<String> {
        let row = self.page.rows.get(index)?;
        Some(format!(
            "Connection: {}\nRecord ID: {}\nTime: {}\nCommand: {}\nStatement classes: {}\n{}",
            self.connection(),
            row.id,
            row.occurred_at,
            row.command,
            classes(&row.classes),
            self.limits()
        ))
    }

    pub fn limits(&self) -> String {
        format!(
            "Profile-local successful safety overrides. Latest {SAFETY_AUDIT_GLOBAL_RETENTION} records retained across all connections; this is not a complete security audit. {} Older records may expire while paging; pages are not one historical snapshot.",
            boundary(self.page.limit, self.page.next_cursor.is_some())
        )
    }

    pub fn empty_label(&self) -> &'static str {
        if self.page.next_cursor.is_some() {
            "No rows in this page range; continue to inspect older retained overrides"
        } else {
            "No retained overrides in this page range"
        }
    }
}

impl Drop for Capture {
    fn drop(&mut self) {
        self.budget
            .set(self.budget.get().saturating_sub(RETAINED_BYTES));
    }
}

fn boundary(limit: Option<SafetyAuditLimit>, continuation: bool) -> &'static str {
    match (limit, continuation) {
        (Some(SafetyAuditLimit::RowLimit), _) => {
            "Page row limit reached; continue for older retained records."
        }
        (Some(SafetyAuditLimit::ByteLimit), _) => {
            "Page byte limit reached; continue for older retained records."
        }
        (None, true) => "Older retained records have a continuation.",
        (None, false) => "End of this retained page range; no further cursor returned.",
    }
}

fn classes(values: &[dbunk_lib::backend::safety_audit::SafetyAuditClass]) -> String {
    if values.is_empty() {
        "None recorded".into()
    } else {
        let mut output = String::new();
        for (index, class) in values.iter().enumerate() {
            if index != 0 {
                output.push_str(", ");
            }
            output.push_str(class.label());
        }
        output
    }
}

#[cfg(test)]
mod tests;
