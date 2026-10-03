use super::*;
impl OverviewView {
    pub(super) fn enabled(&self, action: Action, cx: &gpui::App) -> bool {
        if action == Action::Cancel {
            return self.can_cancel;
        }
        if !self.editable {
            return false;
        }
        match action {
            Action::Back | Action::Scope(_) => true,
            Action::Section(_) => self.capture.is_some(),
            Action::Connect => !self.ready && !self.busy,
            Action::Refresh => self.ready && !self.busy && self.fields.len() == 2,
            Action::Next => {
                self.ready
                    && !self.busy
                    && self.capture_current
                    && self.scope_matches(cx)
                    && self.capture.as_ref().is_some_and(Capture::has_next)
            }
            Action::Cancel => self.can_cancel,
        }
    }
    pub(super) fn activate(&mut self, action: Action, window: &mut Window, cx: &mut Context<Self>) {
        if !self.enabled(action, cx) {
            return;
        }
        if self.composing(window, cx) {
            self.message =
                Some("Finish text composition before changing overview scope or actions".into());
            cx.notify();
            return;
        }
        match action {
            Action::Back => cx.emit(OverviewEvent::Back),
            Action::Connect => cx.emit(OverviewEvent::Connect),
            Action::Cancel => {
                self.capture_current = false;
                cx.emit(OverviewEvent::Cancel);
            }
            Action::Refresh => match self.request(cx) {
                Ok(request) => {
                    // Failure/cancellation makes values stale, not observed identity
                    // disposable. Administration's Clear captures explicitly rebinds.
                    let request = match &self.capture {
                        Some(capture) => capture.refresh_for(request),
                        None => request,
                    };
                    self.capture_current = false;
                    self.message = None;
                    cx.emit(OverviewEvent::Refresh(request));
                }
                Err(error) => self.message = Some(error.into()),
            },
            Action::Next => {
                if let Some(request) = self.capture.as_ref().and_then(Capture::next_request) {
                    self.capture_current = false;
                    self.message = None;
                    cx.emit(OverviewEvent::Next(request));
                }
            }
            Action::Scope(index) => {
                if let Some(scope) = Scope::ALL.get(index).copied() {
                    self.scope = scope;
                    window.focus(&self.scopes[index], cx);
                }
            }
            Action::Section(index) => {
                if let Some(section) = Section::ALL.get(index).copied() {
                    self.select(section, 0, window, cx);
                    window.focus(&self.sections[index], cx);
                }
            }
        }
        if !matches!(action, Action::Back)
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
            .filter(|(_, (action, _))| self.enabled(*action, cx))
            .map(|(index, _)| self.buttons[index].clone())
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
        handles.push(self.list.clone());
        if let Some(editor) = &self.editor {
            handles.push(editor.editor.focus_handle(cx));
        }
        let current = handles.iter().position(|handle| handle.is_focused(window));
        let next = if reverse {
            current.map_or(handles.len() - 1, |index| {
                (index + handles.len() - 1) % handles.len()
            })
        } else {
            current.map_or(0, |index| (index + 1) % handles.len())
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
        let modifiers = event.keystroke.modifiers;
        if modifiers.control || modifiers.alt || modifiers.platform {
            return;
        }
        match event.keystroke.key.as_str() {
            "tab" => self.focus_control(modifiers.shift, window, cx),
            "escape" => self.activate(Action::Back, window, cx),
            "enter"
                if self
                    .fields
                    .iter()
                    .any(|field| field.focus_handle(cx).contains_focused(window, cx)) =>
            {
                self.activate(Action::Refresh, window, cx)
            }
            key @ ("left" | "right")
                if self.scopes.iter().any(|handle| handle.is_focused(window)) =>
            {
                let current = self
                    .scopes
                    .iter()
                    .position(|handle| handle.is_focused(window))
                    .unwrap();
                if let Some(index) = move_tab(current, self.scopes.len(), key) {
                    self.activate(Action::Scope(index), window, cx);
                }
            }
            key @ ("left" | "right")
                if self.sections.iter().any(|handle| handle.is_focused(window)) =>
            {
                let current = self
                    .sections
                    .iter()
                    .position(|handle| handle.is_focused(window))
                    .unwrap();
                if let Some(index) = move_tab(current, self.sections.len(), key) {
                    self.activate(Action::Section(index), window, cx);
                }
            }
            "enter" if self.list.is_focused(window) => {
                if let Some(editor) = &self.editor {
                    window.focus(&editor.editor.focus_handle(cx), cx);
                }
            }
            key if self.list.is_focused(window) => {
                let count = self
                    .capture
                    .as_ref()
                    .map_or(0, |capture| capture.count(self.section));
                let Some(index) = move_index(self.selected, count, key) else {
                    return;
                };
                self.select(self.section, index, window, cx);
            }
            _ => return,
        }
        cx.stop_propagation();
        window.prevent_default();
    }
}
pub(super) fn move_tab(current: usize, count: usize, key: &str) -> Option<usize> {
    if current >= count {
        return None;
    }
    match key {
        "left" => Some(current.saturating_sub(1)),
        "right" => Some((current + 1).min(count - 1)),
        _ => None,
    }
}
pub(super) fn move_index(current: Option<usize>, count: usize, key: &str) -> Option<usize> {
    if count == 0 {
        return None;
    }
    let value = current.unwrap_or(0).min(count - 1);
    Some(match key {
        "up" => value.saturating_sub(1),
        "down" if current.is_none() => 0,
        "down" => (value + 1).min(count - 1),
        "pageup" => value.saturating_sub(20),
        "pagedown" => value.saturating_add(20).min(count - 1),
        "home" => 0,
        "end" => count - 1,
        _ => return None,
    })
}
