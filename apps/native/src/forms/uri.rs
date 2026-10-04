//! URI import is an atomic form prefill, never a connection operation.
use super::*;
use gpui::EntityInputHandler;

impl Form {
    pub(super) fn import_uri(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy || !matches!(self.kind, Kind::Connection { id: None }) {
            return;
        }
        if self.fields.iter().any(|field| {
            field.editor.update(cx, |editor, cx| {
                editor.marked_text_range(window, cx).is_some()
            })
        }) {
            self.fail("Finish text composition before importing a URI");
            cx.notify();
            return;
        }
        let parsed = match crate::connection_uri::read(cx) {
            Ok(parsed) => parsed,
            Err(error) => {
                self.fail(error);
                cx.notify();
                return;
            }
        };
        // The complete parse succeeds before any field changes. Fresh buffers
        // prevent repeated imports from retaining earlier passwords in undo.
        for (key, value) in [
            ("host", parsed.host),
            ("port", parsed.port.to_string()),
            ("database", parsed.database),
            ("user", parsed.user),
        ] {
            self.replace_imported_field(key, value, window, cx);
        }
        if let Some(password) = parsed.password {
            self.replace_imported_field("password", password, window, cx);
        }
        if let Some(mode) = parsed.tls_mode {
            self.tls = mode;
        }
        self.note(if parsed.ignored_params.is_empty() {
            "URI imported. Review fields before Test or Save".into()
        } else {
            format!(
                "URI imported. Set ignored options manually: {}",
                parsed.ignored_params.join(", ")
            )
        });
        cx.notify();
    }

    fn replace_imported_field(
        &mut self,
        key: &'static str,
        value: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let field = self
            .fields
            .iter_mut()
            .find(|field| field.key == key)
            .unwrap();
        let focused = field.editor.focus_handle(cx).is_focused(window);
        let secret = key == "password";
        let editor = cx.new(|cx| {
            let buffer = cx.new(|cx| language::Buffer::local(value, cx));
            let mut editor = Editor::for_buffer(buffer, None, window, cx);
            editor.set_mode(editor::EditorMode::SingleLine);
            editor.set_masked(secret, cx);
            editor
        });
        if focused {
            window.focus(&editor.focus_handle(cx), cx);
        }
        field.accessible =
            cx.new(|cx| AccessibleEditor::field(editor.clone(), field.label, secret, cx));
        field.editor = editor;
    }
}
