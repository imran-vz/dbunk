# SQL editor accessibility follow-up

2026-10-02. Scope authorized by Imran: get the editor working for the gate in
`plans/gpui-working-version-task.html`. Work is confined to the synthetic
native spike, the external test harness and documentation. No live database,
daily-driver profile, keychain or preview channel was used.

## Change

The original investigation missed GPUI's public `a11y_synthetic_children`
hook. `A11ySubtreeBuilder::parent_node` exposes the full AccessKit node, and
`push_child` accepts text runs. The existing `on_a11y_action` hook receives
selection requests. All are present at the original Zed revision
`506beb34de3f433707b7ebe8d8ad2d80f856af6c`; there is no dependency fork or
revision change.

`spikes/gpui-path-z/src/accessible_editor.rs` wraps the real Zed editor. It:

- Exposes the document value, logical line runs and editor focus.
- Publishes the primary caret and directed selection, with grapheme lengths
  in UTF-8; AccessKit translates these to macOS UTF-16 ranges.
- Applies AX selection requests to the real editor, with autoscroll.
- Invalidates cached text on buffer edits, including undo. Text-run IDs are
  stable during caret movement and change with the text revision to reject
  stale positions. Requests are also checked against the published text.
- Builds no text tree until accessibility is activated. Caret changes reuse
  the cached document; no timers or repaint loops were added.

The only added direct dependency is `unicode-segmentation`, already present
in the pinned editor's dependency closure. Layout, SQL completion, syntax
highlighting and the editing model remain Zed's.

## Automated evidence

The baseline command `/tmp/dbunk-editor-ax <old-spike-pid>` failed with:

```text
PASS: SQL editor is exposed
FAIL: SQL text is readable
```

The new check runs against macOS Accessibility, not an internal test-only
editor API. It checks:

- Readable SQL, focused text area and a caret range.
- Exact Unicode text after keyboard input, including an emoji and combining
  sequence; UTF-16 character count and the caret after a trailing newline.
- Reading an individual line using `AXRangeForLine` and `AXStringForRange`.
- Selecting the emoji through `AXSelectedTextRange`, then replacing that exact
  range by typing.
- Undo, forward and backward keyboard selection, caret movement matching
  Zed's character boundaries, empty-buffer recovery and undo from empty.
- Delivery of `AXValueChanged` and `AXSelectedTextChanged` notifications to
  an external observer, so assistive clients can follow edits and selection.

Final result: **PASS**. The fresh 2,000-line fixture is exposed as
`AXTextArea "SQL editor" value[65227 chars] focused`.

Saved evidence:

- [Native tree](./accessibility/native-editor-tree.txt) and
  [environment/counts](./accessibility/native-editor.json), captured before
  replacing the document with the small test fixture.
- [macOS round-trip assertions](./accessibility/editor-roundtrip.txt), all
  passing against the final build.

Verification completed:

| Command | Result |
| --- | --- |
| `pnpm format`, `pnpm lint`, `pnpm typecheck` | Pass |
| `pnpm test` | 130 files, 1,488 tests pass |
| `just fmt`, `just lint` | Pass, both backend feature configurations |
| `just test` | 633 pass / 70 ignored without Tauri; 657 pass / 85 ignored with Tauri |
| Native `cargo fmt --check` | Pass |
| Native `cargo clippy --release --locked --all-targets -- -D warnings` | Pass |
| Native `cargo test --release --locked` | 3 focused tests pass |
| Native `cargo build --release --locked` | Pass |
| Swift harness compile and macOS round trip | Pass |

Ignored backend tests need external fixtures; none were enabled for this
editor task. Cargo still reports the pinned transitive `block` crate's
future-compatibility warning. Native tests cover Unicode boundary round trips,
invalid text positions and oversized combining clusters without byte loss.

Zed groups edits within 300 ms. The test waits 500 ms between fixture setup
and the replacement so undo checks that replacement independently.

Reproduce from the repository root:

```sh
(cd spikes/gpui-path-z && cargo build --release --locked)
DBUNK_SPIKE_FIXTURE=many spikes/gpui-path-z/target/release/dbunk-gpui-spike
# In another shell, using this spike's PID:
swiftc tools/measure/editor-accessibility.swift -o /tmp/dbunk-editor-ax
/tmp/dbunk-editor-ax <spike-pid>
tools/measure/.build/release/measure ax --pid <spike-pid> --tree
```

The test deliberately replaces the spike's synthetic document and leaves a
small Unicode fixture. It never changes the clipboard. The process name is
checked before any input is sent.

## Human VoiceOver validation

**Complete, confirmed by Imran on 2026-10-02:** “voice over is also complete”.
This records the human check separately from the automated evidence. No
additional per-step observations were supplied.

The listening procedure is retained for future verification (Cmd-F5 enables
VoiceOver; Control-Option is the VoiceOver modifier):

1. Enter the SQL editor. Confirm its name and editable multiline role are
   announced, then interact with it (Control-Option-Shift-Down).
2. Read lines, move the caret and select text with Shift-arrow. Confirm the
   caret and selected text are announced, including the emoji and combining
   sequence in the test fixture.
3. Type over a selection, undo, and read the result. Navigate away to the
   result grid and back; confirm focus returns to the real editor.
4. Compare the behavior with the existing Tauri editor. Record any difference
   before approving the stage 01 gate.

## Adapter limits after geometry and focus verification

- It exposes the primary selection, not secondary cursors.
- It splits visible text at display-row boundaries and exposes shaped glyph
  positions and widths in physical pixels, using the current frame after
  editor layout. Offscreen text remains readable as logical runs without
  stale highlight bounds. Folded/inlay text and bidirectional layout still
  need the separately costed extension in the gate review.
- AccessKit stores individual character lengths in one byte. A pathological
  grapheme longer than 255 UTF-8 bytes is split into scalar units to preserve
  the exact text rather than truncate it.
- The JSON cell-editor probe now uses the same adapter. Its keyboard focus
  flow and character geometry are verified by the external AX harness.
- Full form and focus-order parity still belongs to the native UI migration.

The earlier “must fork or waive accessibility” conclusion is withdrawn.
VoiceOver validation is complete. Geometry and editor/results focus
verification are now complete as recorded below. Remaining full-application
work is costed in [the stage 01 review](./stage01-gate.md); no limit is waived
and daily-driver cutover is not authorized.


## Geometry and keyboard focus completion, 2026-10-02

The adapter now reads the editor's current display snapshot after its prepaint.
A nonpainting layout hook is anchored to the editor's top-left corner. It
shapes only visible rows, splits cached logical runs at visible soft-wrap
boundaries, and supplies AccessKit with run bounds, character positions and
advances. Bounds use the same gutter, horizontal/vertical scroll offsets and
display scale as the editor. No timer or repaint loop was added. Text and IDs
remain independent of geometry; an edit or a changed run boundary rejects old
selection positions.

`F6` / `Shift-F6` move between SQL and results. Tab and Shift-Tab return from
results to SQL; Tab inside an editor keeps its editing behavior. Enter opens
the selected result cell (the first cell when no selection exists). F6 returns
to an open cell editor, while Escape discards and Cmd-S stages, both returning
to results. The cell editor uses the same accessibility adapter.

The extended macOS harness verifies:

- Screen containment in the SQL and cell-editor frames, exact ASCII, emoji
  and combining-sequence widths, adjacent character edges,
  multiline range unions, an empty editor and a trailing-newline caret.
- Vertical and horizontal autoscroll, removal of stale offscreen bounds,
  soft-wrap highlight rows and buffer selection across those run boundaries.
- Window resize and restoration, with highlight geometry reflowing both ways.
- F6, Shift-F6, Tab and Shift-Tab focus transitions; keyboard-only cell-editor
  open, leave, restore, cancel and stage; preserved SQL selection, actual
  keyboard replacement after returning, and undo.

Four native tests pass, including a display-run split test that preserves
Unicode byte offsets and rejects boundaries inside a grapheme. The earlier
repository verification table still describes the earlier adapter run; this
follow-up reran the required format/lint/type checks and both backend test
configurations, plus native format, Clippy, tests and release build. It did
not rerun the unrelated frontend unit suite or any fixture-dependent test.

Final evidence is in `accessibility/editor-geometry-focus.txt`,
`accessibility/native-geometry-tree.txt`, `accessibility/native-geometry.json`
and `accessibility/performance/`. See [the gate review](./stage01-gate.md) for
the decision, performance follow-up and remaining costed work.
