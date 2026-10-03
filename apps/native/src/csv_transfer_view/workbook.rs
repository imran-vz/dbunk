//! Sheet selection shares the transfer setup's revision and composition fences.
use super::*;

impl CsvTransferView {
    pub(super) fn workbook_data<'a>(&self, cx: &'a gpui::App) -> Option<&'a CsvWorkbookData> {
        let (id, token) = self.inspection.as_ref()?;
        let setup = self.setup.as_ref()?;
        if !setup.xlsx() || !setup.is_current(token) {
            return None;
        }
        let data = self.store.read(cx).workbook()?.data();
        (data.inspection_id == *id).then_some(data)
    }
    pub(super) fn can_load_workbook(&self, cx: &gpui::App) -> bool {
        self.setup_enabled(cx)
            && self.setup.as_ref().is_some_and(Setup::xlsx)
            && self.inspection.as_ref().is_some_and(|(id, _)| {
                self.store
                    .read(cx)
                    .capture()
                    .and_then(|capture| capture.inspection(*id))
                    .is_some_and(|row| {
                        matches!(
                            row.phase,
                            CsvInspectionPhase::WorkbookReady | CsvInspectionPhase::Ready
                        )
                    })
            })
    }
    pub(super) fn inspect_sheet(&mut self, cx: &mut Context<Self>) -> Result<(), &'static str> {
        let data = self
            .workbook_data(cx)
            .ok_or("Load this setup's workbook sheets first")?;
        let index = self.sheet.ok_or("Select a worksheet")?;
        let id = data.inspection_id;
        if self
            .store
            .update(cx, |store, cx| store.select_sheet(id, index, cx))
        {
            self.mapping = None;
            self.mapping_source = None;
            self.sample_cell = None;
            self.review_pair = None;
            self.status = "Worksheet inspection requested. Check its status, then load it and review its mapping.".into();
        }
        Ok(())
    }
    fn pick_sheet(
        &mut self,
        id: CsvInspectionId,
        index: u16,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.setup_enabled(cx) || self.composing(window, cx) {
            return;
        }
        if self.workbook_data(cx).is_some_and(|book| {
            book.inspection_id == id && book.sheets.iter().any(|sheet| sheet.index == index)
        }) {
            self.change_sheet(id, index, cx);
            window.focus(&self.sheet_focus, cx);
            cx.notify();
        }
    }
    fn change_sheet(&mut self, id: CsvInspectionId, index: u16, cx: &mut Context<Self>) {
        if self.sheet != Some(index) {
            self.sheet = Some(index);
            self.mapping = None;
            self.mapping_source = None;
            self.sample_cell = None;
            self.review_pair = None;
            self.store
                .update(cx, |store, cx| store.discard_setup_review(id, cx));
        }
    }
    pub(super) fn workbook_element(&mut self, cx: &Context<Self>) -> AnyElement {
        let Some(book) = self.workbook_data(cx) else {
            return div().into_any_element();
        };
        let id = book.inspection_id;
        let count = book.sheets.len();
        if self
            .sheet
            .is_none_or(|index| !book.sheets.iter().any(|sheet| sheet.index == index))
        {
            self.sheet = book.sheets.first().map(|sheet| sheet.index);
        }
        let summary = format!(
            "Workbook: {} · {} bytes · {} worksheets",
            book.file_name, book.workbook_bytes, count
        );
        div()
            .child(
                div()
                    .id("xlsx-workbook-summary")
                    .role(Role::Label)
                    .aria_label(summary.clone())
                    .child(summary),
            )
            .child(
                div()
                    .id("xlsx-sheet-list")
                    .role(Role::ListBox)
                    .aria_label("Workbook worksheets")
                    .track_focus(&self.sheet_focus)
                    .tab_index(0)
                    .on_key_down(cx.listener(move |this, event: &KeyDownEvent, window, cx| {
                        if !this.setup_enabled(cx)
                            || !this.sheet_focus.is_focused(window)
                            || this.composing(window, cx)
                        {
                            return;
                        }
                        if event.keystroke.modifiers.platform
                            || event.keystroke.modifiers.control
                            || event.keystroke.modifiers.alt
                        {
                            return;
                        }
                        let Some(book) = this
                            .workbook_data(cx)
                            .filter(|book| book.inspection_id == id)
                        else {
                            return;
                        };
                        let current = book
                            .sheets
                            .iter()
                            .position(|sheet| Some(sheet.index) == this.sheet)
                            .unwrap_or(0);
                        let next = match event.keystroke.key.as_str() {
                            "up" => current.saturating_sub(1),
                            "down" => (current + 1).min(book.sheets.len().saturating_sub(1)),
                            "home" => 0,
                            "end" => book.sheets.len().saturating_sub(1),
                            "enter" => {
                                this.activate(Action::SelectSheet, window, cx);
                                cx.stop_propagation();
                                return;
                            }
                            _ => return,
                        };
                        if let Some(sheet) = book.sheets.get(next) {
                            let index = sheet.index;
                            this.change_sheet(id, index, cx);
                            this.sheet_scroll
                                .scroll_to_item(next, gpui::ScrollStrategy::Top);
                        }
                        cx.notify();
                        cx.stop_propagation();
                    }))
                    .child(
                        gpui::uniform_list(
                            "xlsx-sheets",
                            count,
                            cx.processor(move |this, range: std::ops::Range<usize>, _, cx| {
                                range
                                    .map(|position| {
                                        let Some(book) = this
                                            .workbook_data(cx)
                                            .filter(|book| book.inspection_id == id)
                                        else {
                                            return div().into_any_element();
                                        };
                                        let Some(sheet) = book.sheets.get(position) else {
                                            return div().into_any_element();
                                        };
                                        let index = sheet.index;
                                        let label = format!(
                                            "{}: {} · {:?}",
                                            position + 1,
                                            sheet.name,
                                            sheet.visibility
                                        );
                                        let selected = this.sheet == Some(index);
                                        let weak = cx.weak_entity();
                                        div()
                                            .id(("xlsx-sheet", position))
                                            .role(Role::ListBoxOption)
                                            .aria_label(label.clone())
                                            .aria_selected(selected)
                                            .h(px(28.))
                                            .px_2()
                                            .when(selected, |row| row.bg(rgb(0x252525)))
                                            .child(label)
                                            .on_click(cx.listener(move |this, _, window, cx| {
                                                this.pick_sheet(id, index, window, cx)
                                            }))
                                            .on_a11y_action(
                                                gpui::accesskit::Action::Click,
                                                move |_, window, cx| {
                                                    weak.update(cx, |this, cx| {
                                                        this.pick_sheet(id, index, window, cx)
                                                    })
                                                    .ok();
                                                },
                                            )
                                            .into_any_element()
                                    })
                                    .collect()
                            }),
                        )
                        .track_scroll(&self.sheet_scroll)
                        .h(px(140.)),
                    ),
            )
            .child(self.button(29, cx))
            .into_any_element()
    }
}
