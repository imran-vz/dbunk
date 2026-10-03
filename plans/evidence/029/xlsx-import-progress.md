# Native XLSX import progress, 2026-10-03

Status: implementation and verification in progress. This extends the approved
CSV Transfer A workflow in a Tool tab with sheet selection and the existing
bottom review. No layout choice is reopened. Full PostgreSQL parity is not claimed.

The native reader uses a private immutable workbook snapshot and produces a
private canonical CSV for the existing indexed mapping and transactional import
service. Selected-sheet XML remains buffered under a strict size cap; this does
not add the excluded large-workbook streaming product feature. A revision-bound workbook handle exposes ordered sheet names and
visibility without file ownership. Selecting another sheet invalidates the old
inspection and review. Native setup also invalidates its mapping when the sheet
choice changes; keyboard, AX and mouse actions share setup/composition guards.

Numeric XML text is retained exactly, including bigint and decimal values.
Headers follow the baseline heuristic and duplicate names retain source indexes.
Exact NULL-token matches become SQL NULL; other empty or literal values retain
their distinction. Cached formula values are imported without evaluation. Missing
caches refuse preparation. Numeric dates remain Excel serial text. Spreadsheet
string escapes decode once, including surrogate pairs; unsupported control
characters refuse rather than silently changing content.

Preparation is bounded: one 64 MiB parser execution reservation, a 256 MiB source
snapshot, a 512 MiB canonical artifact, 256 sheets, 1,600 columns, 4 million logical
cells, 1 MiB fields and 8 MiB records. XML parts, ZIP metadata and shared strings
have separate admission limits. These are payload/execution bounds, not process
RSS claims. The UI continues to share the workspace 128 MiB retained and 16 MiB
delivery budgets. Encrypted/ZIP64, ambiguous ZIP endings and unsupported XML
refuse visibly; the legacy dense all-sheets parser is not activated.

Source files remain backend-owned across dropped views, accepted jobs and
connection retirement. Cleanup joins the actual parser, file reader and transfer
workers before unlinking. Failed settlement preserves files and ownership, reports
failed cleanup and blocks reuse. Unknown writes retain the existing explicit
reconciliation behavior and are never automatically retried. Transfer jobs and
private paths remain session-only, matching this service's existing lifecycle;
this increment adds no crash-time replay or temporary-file scavenger. Local file
system calls are joined but are not claimed to be hard-cancellable.

Independent review corrected a missing keyboard setup gate and a source-unlink
race after core reader cleanup timeout. Exact XML structural paths are enforced.
Required repository checks, isolated/backend facade checks, native debug/release
checks, dependency proof and a separate package passed. The owned PostgreSQL probe
passed exact values, indexed duplicate headings, source replacement after snapshot
and private-file cleanup. Scoped actual-window import also passed two exact rows,
worksheet selection/review invalidation and normal joined shutdown. Ignored tests
are not passes. See [source, fixture and window evidence](./xlsx-import-source-checks/README.md).

The window exposed stale acknowledgement text and missing receipt keyboard
scrolling; those corrections pass their [separate window recheck](./xlsx-import-source-checks/window-corrections/README.md). Tool-tab IME remains
pending after an inconclusive Pinyin setup outside GPUI. Settings were restored.

Dependency declarations only promote already-locked zip 2.4.2 and quick-xml 0.31.0
with unchanged feature sets. [Graph and license review](./xlsx-import-source-checks/dependency-review.md)
records the evidence. VoiceOver remains deferred. Keyboard/AX and real Tool-tab
IME still require their own actual-window evidence.
