# Plan 024 Step 5: accessibility probe

What an assistive client sees of each host, read through the macOS
Accessibility API (`measure ax`), 2026-10-02. Both windows were 1440 by 900
with the shared document open and the "many rows" fixture loaded. The harness
announces itself the way VoiceOver does and walks the tree twice, because
both WebKit and AccessKit build their tree only once a client asks.

Original dumps: `accessibility/tauri-tree.txt`, `accessibility/native-tree.txt`.
The original comparison below predates the editor adapter. See
[the editor follow-up](./editor-accessibility.md) for the new implementation
and verification. The earlier claim that this requires a fork was incorrect.

## Original result

| | Tauri (WebView) | Native spike |
| --- | ---: | ---: |
| Elements | 1,446 | 172 |
| Elements with a name | 761 | 147 |
| SQL editor | `AXTextArea "Editor content"`, with its text as the value, focused | `AXTextArea "SQL editor"`, **no value** |
| Result grid | `AXTable` with rows, columns and cells | `AXTable "Query results"` with rows and named cells |
| Buttons, checkboxes, pop-up buttons, text fields | 577, 20, 19, 1 | none (the spike has none) |
| Application menu | 147 menu items | none (the spike sets no menu) |

## What the native numbers mean

**The grid is buildable, and proved.** GPUI reports an element to assistive
technology only when it has an id and a role. With no roles the spike exposed
nothing but the window's three title-bar buttons. After adding
`Role::Table`, `Role::Row`, `Role::Cell` and `Role::ColumnHeader`, with row
and column positions and the cell text as the label, the grid appears as a
table. That is about twenty lines in `src/grid.rs`. Every dbunk component
would need the same treatment; Zed's `ui` crate already does it for buttons,
menus, switches and tree items.

**The unwrapped editor is a gap; dbunk can supply the missing semantics.**
The original investigation found the first problem but missed the public
extension hook that solves the second:

1. Zed's `editor` crate assigns no accessibility role anywhere at the pinned
   revision (zero `.role(` calls outside tests). An unwrapped editor is
   invisible.
2. The convenience methods on GPUI's elements do not include text selection,
   but `a11y_synthetic_children` exposes an `A11ySubtreeBuilder`. Its
   `parent_node` and `push_child` methods accept full AccessKit semantics,
   including `TextRun`, character lengths and `TextSelection`.
   `on_a11y_action` receives `SetTextSelection` requests. These APIs already
   exist at the pinned revision. The original wrapper used none of them.

The WebView exposes all of that today through Monaco's hidden text area.

GPUI gained AccessKit support on 2026-05-27 and Zed's first accessible
surface (the settings UI) on 2026-06-17, so the upstream work is recent and
moving. Whether and when it reaches the editor is not something this plan can
establish.

## Updated route for the stage 01 gate

The follow-up implements an application-owned `AccessibleEditor` adapter
without changing GPUI or Zed. It reports text, logical lines, primary selection
and focus, and forwards assistive selection actions into the real editor.
Imran confirmed VoiceOver validation complete on 2026-10-02. The follow-up now also verifies shaped text geometry, scrolling, soft wraps,
resize and keyboard focus through SQL, results and the cell editor. The
[stage 01 review](./stage01-gate.md) records the gate decision and costed
remaining full-application work. No accessibility exception is used.

## Human validation

Imran confirmed the editor VoiceOver check complete on 2026-10-02; see the
follow-up for the record. To repeat the structural check: `tools/measure/.build/release/measure ax --pid <pid> --tree`.
To check by ear, with VoiceOver on (Cmd-F5): move to the editor and type;
move to the grid and read a row; in the Tauri app, do the same and compare.
