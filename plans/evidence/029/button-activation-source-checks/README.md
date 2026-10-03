# Native button keyboard activation, 2026-10-03

Status: source correction implemented; scoped representative release-window
checks pass below. Full keyboard/AX parity remains open.

[Whole-table export window checks](../whole-table-export-source-checks/window/README.md)
reproduce one Return changing Markdown to XLSX rather than Text. The pinned GPUI
revision `506beb34de3f433707b7ebe8d8ad2d80f856af6c` records unmodified Enter/Space
key-down on a focused clickable element and invokes `on_click` on key-up
(`crates/gpui/src/elements/div.rs:3017–3074`). The native manual key-down handler
also invoked the same action. Preventing default there did not clear the pending
key-up activation. No dependency or toolkit changes are needed.

The narrow WholeExport correction passes all seven individual format steps and
click → Return on Encoding in an owned release window. A source audit finds 30
more duplicate handlers in 29 files. This increment removes those callbacks,
retaining the existing click/AX actions, enabled/composition checks, arrow
navigation, list/tree/editor submission and Cmd-Enter staging. This deliberately
does not change pointer-focus behavior across unrelated views. The source audit
proves duplicate code paths, not actual-window outcomes for all controls.

`source-sha256.json` records 678 files. The applied external review patch SHA256
is `aa019e4f14f76c900295c9597af6caa8769f012185361f1587015144c1aed998`.
No synthetic unit test mirrors GPUI event routing; the actual-window failed and
passing single-key oracle is the regression check. Native debug/release builds,
Clippy and existing behavior tests are being rerun because the change spans
multiple event paths. Unchanged backend/frontend suites passed in the linked
export increment; ignored fixture tests are not passes.

Real Tool-tab IME remains pending. VoiceOver remains deferred, not passed.

The first broader package is
`/private/tmp/dbunk-native-package-20261003-buttons/dbunk Native Preflight.app`,
SHA256 `a796671c5096fd7f8a19f64a0278cbc43751ad68e543a7ec2b7e31bd6731ebed`.
All native debug/release checks pass: 369 tests passed and 13 ignored per mode,
with release package and dependency proof. Frontend format/lint/typecheck pass.

Scoped CUA observations on PID 58733 and the owned review profile:

- Filter operator Return changes `=` → `<>`; Space changes `<>` → `>` once each.
  No new filter is applied.
- Retained export Return changes CSV → JSON; Space changes JSON → SQL once each.
  Escape returns to the grid without publishing a file.
- Administration section arrows, reached through keyboard focus, select Sessions
  → Locks → Sessions. An earlier AX click did not focus the tab; its unchanged
  arrow observation is retained, not counted as a pass.
- Connection form Read-only toggles off → on with one Return. The subsequent
  Space and Escape do nothing: the toggle's dynamic label also keys its focus
  handle and element, so the focused control is replaced on rerender. The form
  is cancelled through its AX button without saving any connection changes.
  `form-single-return.txt` and `form-next-space.txt` record this second failure.

The two dynamic form toggles now use stable element/focus keys independent of
labels. `focus-correction/` records this one-file correction and its new matrix;
actual repeated-key and Escape verification passes in the release window below. The failed and
passing keyboard scopes remain separate. No pointer-focus parity claim is made
for unrelated helpers.

The temporary export tab is closed, the app exits normally, and the exact owned
fixture schema/table are removed with RESTRICT; both fixtures return to zero
backends. [Teardown receipt](../whole-table-export-source-checks/window/teardown.json).

Final focus-correction package:
`/private/tmp/dbunk-native-package-20261003-button-focus/dbunk Native Preflight.app`,
SHA256 `f118aefead9eeaa19b72546df01d0e30822edb27507d1396030a55048e5e9ae5`.
The `focus-correction/source-sha256.json` receipt records the exact source.
CUA repeats the original 26-Tab form route: alternating Return/Space yields
Read-only on/off/on/off; the next Tab and Return/Space yield Favorite yes/no.
Escape closes the dialog. These are actual input-routing checks, not pasted
Unicode or IME evidence. No form changes are saved. PID 63557 quits normally,
both fixtures have zero backends, and the original 14 tabs remain. The JSON/key
and AX receipts are under `focus-correction/`.

The final focus-correction debug/release Clippy, tests, debug build and release
package/dependency proof all pass. Each test mode reports 369 passed and 13
ignored. The post-check source proof matches all 678 recorded files. Required
backend suites remain the unchanged passing whole-export increment.
