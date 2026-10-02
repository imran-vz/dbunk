# Plan 024 Step 6: behavior the WebView supplies today

Each item is something the React app gets from the browser without
application code. The right-hand columns say what dependency path Z (Zed's
in-tree GPUI plus Zed's GPL crates, pinned at `506beb3`) offers in its place.

Status values: **available** (a component or API exists at the pinned
revision), **buildable** (primitives exist; dbunk writes the component),
**gap** (no primitive; needs upstream work or a design decision),
**out of scope** (the native app does not need it).

Sources are the Zed checkout at the pinned revision. "Proved" means the Plan
024 spike exercised it; everything else is a source reading.

| Behavior | Path Z | Status | Notes |
| --- | --- | --- | --- |
| Text input fields | `ui_input::InputField` (a single-line `Editor`) | available | Selection, undo, IME and clipboard come from the editor. |
| Multi-line code editing | `editor::Editor` | available, proved | Constructed with no project; see `step1-dependency-path.md`. |
| Checkbox, switch | `ui::Checkbox`, `ui::Switch` | available | `Switch` carries an accessibility role. |
| Radio group, slider, number stepper | none in `ui` or `ui_input` | buildable | Small. dbunk uses radios in the credential and TLS forms. |
| Select / dropdown | `ui::DropdownMenu`, `ui::ContextMenu` | available | |
| Context menus | `ui::ContextMenu`, `ui::right_click_menu` | available | Menu items carry accessibility roles. |
| Tooltips | `gpui::Tooltip`, `ui::Tooltip` | available | |
| Modal dialogs | `ui::Modal`; `Window::prompt` for native alerts | available | |
| Native file dialogs | `App::prompt_for_paths`, `prompt_for_new_path` | available | Replaces `tauri-plugin-dialog`. |
| Tabs | `ui::TabBar`, `ui::Tab` | available | |
| Tree view | `ui::TreeViewItem` over `uniform_list` | available | The pattern Zed's project panel uses. |
| Virtual scrolling | `gpui::uniform_list`, `gpui::list` | available, proved | Rows only. Column virtualization is application code (proved in the spike's grid). |
| Scrollbars | `ui::Scrollbar` | available | The spike's grid does not use it yet; wheel only. |
| Command palette | `picker::Picker` | available | `command_palette` itself is tied to Zed's `workspace`; the picker is reusable. |
| Selecting and copying arbitrary text (labels, error messages, cell text) | none for plain elements; `markdown` renders selectable text | buildable | The largest behavioral change for users. Every place where text is copied today needs an explicit copy affordance, a read-only editor, or a selection model like the grid's. |
| Cell text selection and copy in the grid | application code | buildable, proved | Range selection and tab-separated copy in the spike. |
| Keyboard focus order (Tab, Shift-Tab) | `tab_index`, `tab_stop`, `focus_visible` | available | Order is declared per element, not derived from document order. The spike now verifies F6/Shift-F6 pane navigation and Tab/Shift-Tab from results; editor Tab keeps indentation. Full forms remain later work. |
| Drag and drop inside the app | `on_drag`, `on_drop` | available | |
| File drop from Finder | `ExternalPaths` | available | |
| IME composition in the editor | `EntityInputHandler` implemented by `Editor` | available | Not exercised: needs a person with an input method. |
| IME composition in cell editors | a cell editor is an `Editor` | available | Same caveat. |
| Bidirectional text | one mention in GPUI's text system, none in `editor` | gap, unverified | The WebView lays out RTL cell values correctly today. Needs a test with Arabic or Hebrew data before stage 05. |
| Accessibility tree | AccessKit in GPUI; roles on `ui` controls; application-owned `AccessibleEditor` | buildable, partly proved | SQL and cell-editor text, primary selection, focus and shaped geometry are exposed through public hooks. VoiceOver confirmed complete by Imran on 2026-10-02; geometry and editor/results keyboard focus are now verified. Full form parity and display-map extensions are costed in `stage01-gate.md`. See `editor-accessibility.md`. |
| Clipboard | `App::write_to_clipboard`, `read_from_clipboard` | available, proved | |
| Opening links | `App::open_url` | available | Replaces `tauri-plugin-opener`. |
| Application menu | `App::set_menus` | available | |
| Window title bar and traffic lights | `TitlebarOptions` | available | Replaces the `objc2-app-kit` positioning code. |
| Window geometry persistence | window bounds API | buildable | Replaces `tauri-plugin-window-state`. |
| Text zoom | `theme_settings` font sizes | available | |
| Icons | SVG assets through `AssetSource` | available | Tabler icons have to be imported as SVG files. |
| Graph canvas (schema map) | `canvas`, `PathBuilder`, positioned elements | buildable, proved | No node-graph component. GPUI has no element transform, so zoom scales lengths and font size by hand. |
| Charts | none | buildable | Only where dbunk draws them today. |
| Toasts | none in `ui` outside `workspace` | buildable | |
| Resizable split panes | `workspace` has them, bound to Zed's pane model | buildable | |
| CSS animation and transitions | per-frame animation APIs exist | out of scope | The acceptance gates forbid repaint loops. |
| Spell check, print, find-in-page | none | out of scope | Not used by the React app. |

## What this means for the port

- Nothing on the list is blocked outright. SQL editor accessibility now has an
  application-owned adapter using the pinned APIs; geometry and pane focus
  checks pass. Remaining work, including bidirectional text, is costed in
  `stage01-gate.md`.
- "Buildable" rows are small one by one. Together they are the form layer of
  the app: radio groups, number fields, toasts, split panes and selectable
  text appear in most screens.
- Text that users copy by dragging over it (connection errors, server
  versions, DDL previews) has no native equivalent. Each such place needs a
  decision, and the parity checklist has to list them.
