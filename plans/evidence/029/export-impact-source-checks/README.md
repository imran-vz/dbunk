# Retained file export and read-only drop impact, 2026-10-03

Status: integrated checks, exact fixture probe and separate package pass. This follows the frozen
[metadata/FK variant](../metadata-fk-source-checks/README.md). No new actual-window,
keyboard/AX or real IME pass is recorded. VoiceOver remains deferred, not passed.

## File export

Query/table grids capture the selected cells, or all retained rows when no
selection exists, in display column order. Captures preserve source values and
NULL, remain immutable during subsequent query/table updates, and never fetch
missing rows. The source and selected-row scope are disclosed. Whole-table
streaming exports and saved rerunnable export configurations remain absent.

Native controls choose CSV, JSON, INSERT, HTML, Markdown, TXT, TSV or XLSX,
UTF-8/UTF-16LE for text, gzip, NULL token and a separately quoted SQL table name.
XLSX writes strings without numeric/formula/hyperlink inference; exact numeric
text is preserved. NULL tokens can collide with real text, and XLSX empty text
becomes a blank cell. A pinned XLSX library Unicode escape hazard is explicitly
refused before workbook assembly; text export remains available.

Snapshots cap at 100,000 cells including headings, 1,024 columns and 8 MiB encoded
input. Formatted/compressed files cap at 8 MiB; XLSX cells additionally cap at
32,767 UTF-16 units. The tool reserves 36 MiB from shared retention for captured
and derived payloads and bounded field/history/AX text. This is not peak process
RSS: the XLSX library also assembles a bounded workbook/XML representation.
The shared allowance remains held by the completion waiter if the tool closes.

At most two blocking file jobs run per host. The host stops admission, cancels
and joins them on app shutdown. A cleanup deadline is a failure, not proof the
blocking thread ended. Local preparation/publication failures remain visible in
the tool. Reaped worker panics remain cleanup failures. Each export uses a private
sibling temporary file, flush/sync and atomic no-clobber publication. Existing
files/symlinks are preserved. Cancellation wins before publication admission;
after admission, the actual publication result wins. No automatic retry occurs.

New field guards cap current text at 8 KiB and bound cumulative editor history,
refusing oversized edits visibly and preserving the last accepted text. Soft
history compaction waits for unmarked input; ordinary IME composition remains
owned by Zed. The guard rejects after the editor event, so transient allocation from a single
enormous paste is not a peak-memory bound. This is source behavior, not observed
real composition evidence.

## Drop impact

The Objects Tool tab exposes a read-only Drop impact action. It uses the same
admitted document, dedicated joined socket, read-only snapshot, cancellation and
deadline as catalog reads. Full kind/schema/routine signature identity is bound.
Late successful catalog/description/impact replies after Cancel are discarded.

Traversal preserves baseline downstream semantics: normal/automatic/internal
edges, silent intermediate nodes, reported owned/identity sequences, view-return
rule normalization, custom rules and exact column subaddresses. Schema impact
can include objects in other schemas. This is not an upstream reference list.

Limits are depth 8, 201 addresses per depth, 8,192 edges per breadth query,
200 displayed dependents, 8 KiB text fields and 1 MiB encoded output. Limits retain
explicit uncertainty, including an empty visible list. The viewer includes exact
object identity, shared retained accounting, copy and no DDL execution action.

## Evidence

- Eight focused file backend tests and six dependency model tests pass.
- `dependencies-live.txt`: the exact opt-in facade probe passed for direct and
  cross-schema views, identity sequences, a custom rule, domain column-specific
  dependencies, a ten-view chain with truncation, exact overloads, missing/wrong
  references, idle cancellation, retired ownership and joined shutdown.
- Only owned schemas `native_impact_20261003` and
  `native_impact_external_20261003` were created on stage03
  `127.0.0.1:15432/dbunk_demo`, UUID `2283820d-33ec-4c4c-ae03-7051092bd410`.
  Setup refused existing names; cleanup rechecked both OIDs/comments and UUID,
  removed the two schemas, and verified activity zero. Setup and cleanup records
  are retained. No DDL was run by the read-only facade probe.
- Initial integration checks caught an editor snapshot borrow and one collapsible
  conditional lint; both were corrected. Their failed logs remain separate from
  the final checks.
- `pnpm format`, `pnpm lint`, `pnpm typecheck`, `just fmt`, `just lint` and
  serialized `just test` pass (core 675 passed/71 ignored; Tauri 692 passed/85 ignored).
- Isolated backend 799 passed/80 ignored; two documentation tests pass;
  Tauri facade subset 69 passed/9 ignored. Tauri custom-protocol build passes.
- Native debug/release tests each 131 passed/13 ignored; format, all-target
  Clippy, fixture-harness Clippy and debug/release builds pass.
- Separate package `/private/tmp/dbunk-native-package-20261003-export-impact/dbunk Native Preflight.app`:
  executable SHA256 `0e569efb94d420d3ca07d19522a272bb9062b5f432d13e5e6e6a78cd6b2cc570`.
  All 302 recorded source hashes match; dependency proof passes.

Ignored aggregate tests are not passes. Only the named opt-in dependency probe
was run live for this increment.

System Settings discovery still returned `cgWindowNotFound`, but a later Finder
Desktop AX/screenshot probe succeeded. This does not establish a system-wide
outage. The [fresh isolated package launch](../export-impact-window-20261003/identity.json)
also failed discovery by both bundle path and bundle ID. The exact owned PID
was stopped with SIGTERM; fixture activity returned to zero. This is not a normal
quit pass. See its [failure record](../export-impact-window-20261003/failed-discovery.json).
New file-dialog, keyboard/AX and real IME acceptance remains pending; earlier
window passes do not transfer. No OS settings changed during source verification.
