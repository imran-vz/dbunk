//! Whole-draft formatting applies approved gap and keyword edits together in
//! one undo transaction, retaining ordinary editor selection and draft tracking.
use super::*;
use std::{cell::Cell, rc::Rc};

use crate::sql_format::WORKING_BYTES as WORK_BYTES;
const WORKSPACE_BYTES: usize = 128 * 1024 * 1024;

struct FormatLease(Option<Rc<Cell<usize>>>);
impl Drop for FormatLease {
    fn drop(&mut self) {
        if let Some(budget) = &self.0 {
            budget.set(budget.get().saturating_sub(WORK_BYTES));
        }
    }
}

impl Workbench {
    pub(super) fn format_sql(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.editor.focus_handle(cx).is_focused(window) {
            self.status = "Focus the SQL editor before formatting".into();
            cx.notify();
            return;
        }
        self.sync_completion_input(window, cx);
        if self.closing || self.execution.is_some() || self.editor.read(cx).read_only(cx) {
            self.status = "Formatting unavailable while the query editor is busy".into();
            cx.notify();
            return;
        }
        if self.completion_composing {
            self.status = "Finish composing before formatting SQL".into();
            cx.notify();
            return;
        }
        let bytes = self.editor.read(cx).buffer().read(cx).len(cx).0;
        if bytes > crate::sql_format::MAX_INPUT_BYTES {
            self.status = "SQL is too large to format; draft unchanged".into();
            cx.notify();
            return;
        }
        if let Some(budget) = &self.retained_budget {
            if WORK_BYTES > WORKSPACE_BYTES.saturating_sub(budget.get()) {
                self.status =
                    "Formatting needs shared memory; clear a retained result or tool".into();
                cx.notify();
                return;
            }
            budget.set(budget.get() + WORK_BYTES);
        }
        let _lease = FormatLease(self.retained_budget.clone());
        let source = self.editor.read(cx).text(cx);
        let edits = match crate::sql_format::format_edits(&source) {
            Ok(edits) => edits,
            Err(error) => {
                self.status = format!("SQL not formatted: {error}; draft unchanged");
                cx.notify();
                return;
            }
        };
        if edits.is_empty() {
            self.status = "SQL format unchanged".into();
            cx.notify();
            return;
        }
        self.editor.update(cx, |editor, cx| {
            crate::sql_completion::dismiss(editor, window, cx);
            editor.finalize_last_transaction(cx);
            editor.transact(window, cx, |editor, _, cx| {
                editor.edit(
                    edits.into_iter().map(|edit| {
                        (
                            MultiBufferOffset(edit.range.start)..MultiBufferOffset(edit.range.end),
                            edit.text,
                        )
                    }),
                    cx,
                );
            });
            editor.finalize_last_transaction(cx);
        });
        self.status = "SQL formatted".into();
        cx.notify();
    }
}
