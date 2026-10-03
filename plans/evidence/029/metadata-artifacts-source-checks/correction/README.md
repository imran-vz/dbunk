# Corrected map window checks, 2026-10-03

Frozen package: `/private/tmp/dbunk-native-package-20261003-map-correction/dbunk Native Preflight.app`.
Executable SHA256: `f69e00d844dfa20ba0a392e5f585985c21562583ce949dc19ac08a28ccef49c9`.
The 615-file source receipt identifies this package; later Connection settings work
is outside that receipt. Native debug/release Clippy, tests and builds pass:
355 passed, 13 ignored in each test profile. Fixture-feature Clippy and native fmt
also pass. The initial corrected test attempt failed on a removed private model
accessor; the test was corrected before the recorded final matrix. Ignored tests
are not passes. The debug linker emitted an oversized compact-unwind warning;
its build completed, and release packaging completed separately.

Both launches used `workspace_launch.py`, the owned stage03/stage04 fixtures and
`/private/tmp/dbunk-native-auto-fit-20261003-review`. PIDs 94958 and 98105 quit
normally. The original 13 documents and the added map tab remain; a temporary
browse tab was opened and closed. No production or daily-driver profile was used.

Observed passes:

- Database and schema Refresh/Fit render paths, captions and nodes together.
  The previous camera mismatch and opaque-caption occlusion did not reproduce.
  Composite captions fit the adaptive rank gap in the on-screen map and PNG.
- Schema routing reloads Step while Database defaults remain Curve. The comment
  toggle saves to the schema scope and reloads after a normal quit/reopen.
- List Shift+Right moves the child from x=500 to x=510 world units and receives
  the exact-save acknowledgement. Pointer dragging then records x=556.07,
  y=63.035; canvas Shift+Down records y=73.035. Reopen plus list Shift+Up records
  y=63.035, proving movement starts at the saved position; Shift+Down restores it.
  Read-only SQLite receipts record exact revisions and positions.
- Open selected table rechecks its OID and opens the exact child relation with
  four columns, zero rows and no active filter. Its temporary tab closes normally.
- A nonexistent setup schema retains the old capture visibly marked STALE.
  Preference controls and Open selected table disable; Shift+Arrow explicitly
  refuses movement, leaving the saved position unchanged.
- Native PNG publication produces 43,760 bytes, RGBA8 2400×692, with readable
  composite labels. The save panel retained its default `schema-map.png` after
  an unsuccessful pasted rename; this is the actual output recorded in exports.json.
  A subsequent AX setValue correctly sets the SVG filename.
- Enabling the Unicode comment causes explicit PNG missing-glyph refusal and
  creates no destination file. SVG preserves the comment and saves 5,501 bytes.
  PNG and SVG here use different display settings; no byte-equivalence is claimed.

`teardown.json` records guarded removal of all four tables and two schemas from
`../window/setup.json`, using exact OID/name/owner/comment guards and RESTRICT.
Recorded OIDs are absent and both fixtures have zero other backends.

This is scoped actual-window and keyboard/AX evidence, not full acceptance.
Large/cyclic map window interaction, all display/reset/cancel paths, full keyboard
coverage and Tool-tab real IME remain open. Unicode PNG and legacy preference
import remain parity gaps. VoiceOver remains deferred and was not tested.
