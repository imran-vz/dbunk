use super::*;
impl DdlExportView {
    pub(super) fn enabled(&self, action: Action) -> bool {
        if action == Action::Cancel {
            return self.can_cancel;
        }
        if action == Action::CancelFile {
            return self.file_busy
                && self
                    .cancellation
                    .as_ref()
                    .is_some_and(|token| !token.is_cancelled());
        }
        if !self.editable {
            return false;
        }
        match action {
            Action::Back | Action::Scope(_) => true,
            Action::Connect => !self.ready && !self.busy,
            Action::Refresh => {
                self.ready && !self.busy && !self.file_busy && self.fields.len() == 2
            }
            Action::Clear => !self.busy && !self.file_busy && self.capture.is_some(),
            Action::Save => !self.busy && !self.file_busy && self.capture.is_some(),
            Action::Previous => self.capture.is_some() && self.page > 0,
            Action::Next => self
                .capture
                .as_ref()
                .is_some_and(|c| self.page + 1 < c.pages(self.section)),
            Action::Section(_) => self.capture.is_some(),
            Action::Cancel | Action::CancelFile => false,
        }
    }
    pub(super) fn activate(&mut self, action: Action, window: &mut Window, cx: &mut Context<Self>) {
        if !self.enabled(action) {
            return;
        }
        if self.composing(window, cx) {
            self.message =
                Some("Finish text composition before changing DDL scope or actions".into());
            cx.notify();
            return;
        }
        match action {
            Action::Back => cx.emit(DdlExportEvent::Back),
            Action::Connect => cx.emit(DdlExportEvent::Connect),
            Action::Cancel => {
                self.capture_current = false;
                cx.emit(DdlExportEvent::Cancel);
            }
            Action::Clear => {
                self.editor = None;
                self.capture = None;
                self.capture_current = false;
                self.page = 0;
                self.message=Some("Capture cleared. The next capture will inspect the current names and bind their identity.".into());
            }
            Action::Refresh => match self.request(cx) {
                Ok(request) => {
                    let request = match &self.capture {
                        Some(c) => c.refresh_for(request),
                        None => request,
                    };
                    self.capture_current = false;
                    self.message = None;
                    cx.emit(DdlExportEvent::Refresh(request));
                }
                Err(error) => self.message = Some(error.into()),
            },
            Action::Save => self.save(window, cx),
            Action::CancelFile => {
                if let Some(token) = &self.cancellation {
                    token.cancel();
                }
                self.message = Some(
                    "File cancellation requested. Publication already admitted may still finish."
                        .into(),
                );
            }
            Action::Previous => self.show_page(self.section, self.page - 1, window, cx),
            Action::Next => self.show_page(self.section, self.page + 1, window, cx),
            Action::Scope(index) => {
                if let Some(scope) = Scope::ALL.get(index) {
                    self.scope = *scope;
                    window.focus(&self.scopes[index], cx);
                }
            }
            Action::Section(index) => {
                if let Some(section) = Section::ALL.get(index) {
                    self.show_page(*section, 0, window, cx);
                    window.focus(&self.sections[index], cx);
                }
            }
        }
        if action != Action::Back
            && let Some(index) = ACTIONS.iter().position(|(item, _)| *item == action)
        {
            window.focus(&self.buttons[index], cx);
        }
        cx.notify();
    }
    pub(super) fn focus_control(
        &mut self,
        reverse: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.composing(window, cx) {
            return;
        }
        let mut handles = ACTIONS
            .iter()
            .enumerate()
            .filter(|(_, (a, _))| self.enabled(*a))
            .map(|(i, _)| self.buttons[i].clone())
            .collect::<Vec<_>>();
        if self.editable {
            handles.extend(self.scopes.iter().cloned());
            if self.scope != Scope::Database
                && let Some(field) = self.fields.first()
            {
                handles.push(field.focus_handle(cx));
            }
            if self.scope == Scope::Relation
                && let Some(field) = self.fields.get(1)
            {
                handles.push(field.focus_handle(cx));
            }
            if self.capture.is_some() {
                handles.extend(self.sections.iter().cloned());
            }
        }
        if let Some(editor) = &self.editor {
            handles.push(editor.editor.focus_handle(cx));
        }
        if handles.is_empty() {
            return;
        }
        let current = handles.iter().position(|handle| handle.is_focused(window));
        let next = if reverse {
            current.map_or(handles.len() - 1, |i| {
                (i + handles.len() - 1) % handles.len()
            })
        } else {
            current.map_or(0, |i| (i + 1) % handles.len())
        };
        window.focus(&handles[next], cx);
        cx.notify();
    }
    pub(super) fn key(
        &mut self,
        event: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.composing(window, cx) {
            return;
        }
        let m = event.keystroke.modifiers;
        if m.control || m.alt || m.platform {
            return;
        }
        match event.keystroke.key.as_str() {
            "tab" => self.focus_control(m.shift, window, cx),
            "escape" => self.activate(Action::Back, window, cx),
            "enter"
                if self
                    .fields
                    .iter()
                    .any(|field| field.focus_handle(cx).contains_focused(window, cx)) =>
            {
                self.activate(Action::Refresh, window, cx)
            }
            key @ ("left" | "right") if self.scopes.iter().any(|h| h.is_focused(window)) => {
                let current = self
                    .scopes
                    .iter()
                    .position(|h| h.is_focused(window))
                    .unwrap();
                self.activate(
                    Action::Scope(if key == "left" {
                        current.saturating_sub(1)
                    } else {
                        (current + 1).min(Scope::ALL.len() - 1)
                    }),
                    window,
                    cx,
                );
            }
            key @ ("left" | "right") if self.sections.iter().any(|h| h.is_focused(window)) => {
                let current = self
                    .sections
                    .iter()
                    .position(|h| h.is_focused(window))
                    .unwrap();
                self.activate(
                    Action::Section(if key == "left" {
                        current.saturating_sub(1)
                    } else {
                        (current + 1).min(Section::ALL.len() - 1)
                    }),
                    window,
                    cx,
                );
            }
            _ => return,
        }
        cx.stop_propagation();
        window.prevent_default();
    }
}
