# Whole-table export window checks, 2026-10-03

Initial package: `/private/tmp/dbunk-native-package-20261003-whole-export/dbunk Native Preflight.app`.
Executable SHA256: `c1f0e7482d9e008f9b92cf150de6eb62f937b185f13d5211dbc78f1d71db8674`.
The 678-file `../source-final-sha256.json` receipt matches this variant. Launch
used `workspace_launch.py`, stage03/stage04 ownership checks, and profile
`/private/tmp/dbunk-native-auto-fit-20261003-review` (ID
`c16f87dc-e21a-4ea2-b2ed-a0fdc9aa8a42`). PID 40206 quit normally; both fixtures
returned to zero backends. The fixture objects in `setup.json` were removed after corrected checks using
exact OID/name/owner/comment guards and RESTRICT; `teardown.json` records absence
and zero backends. The temporary fifteenth table tab was closed; the original
14 tabs and local saved recipe remain.

Scoped observations:

- The disconnected Administration Connection settings mirror shows the exact
  connection ID and saved endpoint. Home/Down/End select fields; Tab reaches the
  exact read-only editor and typing does not modify its contents. Shift-Tab back
  to Edit and Return opens that connection's existing form. A temporary folder
  save updates the mirror; the original empty folder is restored. Escape cancels
  Edit, and Escape returns from settings to Administration.
- The new owned table contains 13 rows with five columns. A typed `id = 1` grid
  filter displays one row. Whole-table capture independently returns all 13 rows
  and five columns, with capture interval and role/RLS scope shown visibly.
- Native save dialogs publish JSON (1,867 bytes), UTF-16LE with BOM plus gzip SQL
  (421 bytes), and XLSX (5,798 bytes). Byte/content receipts compare every value
  to `setup.json`. Numeric and bigint text, quoted Unicode/newline text, NULL and
  empty strings remain exact. XLSX uses text cells and the explicit `NULL雪`
  marker; empty text is blank. Its text-encoding control is disabled.
- Save configuration persists exact SQL/UTF-16LE/gzip/`NULL雪` options locally.
  After changing controls to XLSX, Load latest restores those saved options and
  disables Save captured table by clearing the old capture. It does not execute.
- CSV with UTF-16LE/gzip explicitly refuses without opening Transfer. Switching
  to UTF-8/no compression opens the existing CSV Tool tab with the exact target
  and NULL token. Inspect → review → start completes and joins cleanup; the
  resulting 775-byte CSV matches all 13 rows. The finished transfer is dismissed
  and its transient setup cleared. Files are retained as evidence.

Three reproducible UI defects were found: the parent tab remains labelled
“Capturing” after success; whole-export target/status/disclosures and the settings
status are absent from AX; clicking an export button leaves keyboard focus on
the previous control. A three-file correction is under separate verification in
`../correction/`. These initial observations do not pass the affected AX/focus
gates. The subsequent single-key check below resolves the initial unexpected format
sequence; it was a reproducible duplicate activation.

The first corrected package (`0519ac168584d3adc73e544682bbeb7c89e8d10950a1dec7d7c4640302752a60`)
exposes whole-export target, status and both scope paragraphs in AX. Its
saved-configuration process reopen passes: local Load configurations → Load
latest restores SQL INSERT / UTF-16LE / gzip / `NULL雪`, while Capture and Save
remain disabled without a connection. `recipe-reopened.txt` and
`recipe-reopened-activity.json` record the state and zero backends on both
fixtures. PID 49008 quit normally through Cmd-Q with launcher exit 0.

One Return changes Markdown to XLSX instead of Text; individual Return and Space
also skip formats earlier in that run. `keyboard-single-key-red.json` is the
explicit failed oracle. The pinned GPUI `div.rs` already turns an unmodified
Enter/Space down/up pair into `on_click`; the view's manual key-down activation
runs the same action first. The narrow correction removes that extra handler,
retaining the click/AX action and composition/disabled guards. Its separately
hashed package/checks live in `../keyboard-correction/`; corrected individual-key
window checks pass on SHA256
`051d070cac2c9cc7a752a4e236716237f2c7dc5a99ec19f7ad13baf64b0add08`:

- Seven individual alternating Return/Space actions traverse the seven formats
  exactly once (`keyboard-single-key-green.json`), including the original
  Markdown → Text Return oracle.
- Clicking Encoding changes UTF-16LE to UTF-8; the next Return changes that same
  button back once, demonstrating click-to-keyboard focus (`pointer-keyboard-focus.txt`).
- Reloading the saved recipe and explicitly connecting/capturing reads 13 rows
  from the one-row filtered grid. Both parent tab and accessible status settle to
  complete (`corrected-capture.txt`).
- A separately owned 20-second transaction holds ACCESS EXCLUSIVE on the exact
  fixture table after OID/name/owner/comment validation. Capture blocks; Cancel
  settles the parent and status to “no new capture accepted,” preserves the old
  immutable capture, and joins the reader. The blocker ends with ROLLBACK and
  performs no row/DDL writes (`corrected-capture-blocked.txt`,
  `corrected-capture-cancelled.txt`).
- Connection settings exposes its redacted metadata disclosure in AX
  (`settings-corrected-ax.txt`).
- PID 53431 quits normally; both fixtures return to zero backends
  (`../keyboard-correction/quit.json`). Native debug/release checks pass with
  369 tests passed and 13 ignored per mode; release package/dependency proof pass.

A source audit found the same duplicate activation pattern in other native
button builders. [The broader correction](../../button-activation-source-checks/README.md)
is tracked separately; these export observations do not pass every affected
control. A refusal-path parent-status window check and full Tool-tab keyboard/AX
coverage remain pending.
Real Tool-tab IME remains pending after the separate inconclusive activation
attempt. VoiceOver remains deferred. This is scoped evidence, not full export,
keyboard/AX or PostgreSQL parity acceptance.
