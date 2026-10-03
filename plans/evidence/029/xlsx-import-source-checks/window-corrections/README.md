# Transfer window corrections

The executable `cab926e616d1cdddacbea1a67d3d98a55ecb3a89f4955d8f1d9bcd08b5a77424`
changes three native files from the original XLSX package. The source manifests
and package identities are separate; backend source and dependency graphs did not
change. New seed modules were not declared in this build.

The original window showed a stale Preparing message after completion and did
not respond to receipt Page Up/Home. Request acknowledgements now identify their
request without presenting the old response phase as current progress. Page keys
scroll the body outside text editors; receipt focus additionally supports arrows,
Home and End. Modifiers and marked composition preserve editor ownership.

Scoped CUA recheck passed on the same isolated profile:

- Opened the existing owned workbook through the native file picker. Typed schema,
  table and NULL token with Tab traversal, then selected the sheet with Return.
- Prepared and reviewed XLSX against `native_parity_20261003.rows` without starting
  an import. Page Down exposed the full provenance/constraints and mapping; Page
  Up moved back toward exact target identity. Released that unused inspection.
- Exported the owned table to a new temporary file through the native Save dialog.
  The completed receipt reported publication and cleanup, with no stale Preparing
  message. Selecting the job and pressing Return, End, Home and End navigated the
  receipt body. Screenshots show the changed scroll positions.
- Parsed the resulting file independently: 60 data rows, 1,805 bytes; the owned
  source still has 60 rows. The export receipt correctly avoids inventing a row
  count when the backend does not provide one.
- Quit normally. Both fixtures returned 0 → 0 app connections. All ten workspace
  documents and original drafts were retained; no database writes were dispatched
  by this recheck. The new export file is retained with its hash receipt.

Debug and release Clippy/tests passed, as did packaging and dependency
proof. Debug and release tests: 282 passed, 13 ignored each.
Required pnpm format/lint/typecheck passed. Backend required checks from the
original frozen XLSX source remain applicable because no backend file changed.
The initial native Clippy failure is retained and corrected by the final log.

This verifies the named keyboard actions and visible/AX workflow only. Complete
AX/Tool-tab IME acceptance remains pending; VoiceOver remains deferred. No IME or
system input settings were changed during this correction recheck.
