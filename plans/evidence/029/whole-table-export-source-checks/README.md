# Whole-table export and saved recipes, 2026-10-03

Status: implementation and scoped window verification recorded below. Full
export and PostgreSQL parity acceptance remain open.

Baseline `102568b:src/components/table-editor-panel.tsx` routes whole-table CSV
through Transfer and explicitly refuses UTF-16LE or gzip for that route. The six
other file formats acquire the complete relation before formatting. Native work
keeps scalable CSV separate and adds a bounded, complete-or-error read-only
capture for JSON, SQL INSERT, HTML, Markdown, TXT and XLSX. Grid filters, hidden
columns, selections and staged changes do not define whole-table export scope.
Four focused capture tests and the explicit stage03 live probe pass. The probe
covers quoted/Unicode tables, views, materialized views, an empty view and
partitions, exact bigint/numeric/NULL/empty values, wrong-OID refusal and an
oversized field. All probe objects were removed by exact ownership guards;
activity returned to zero. Foreign tables, role/RLS variations and deterministic
concurrent namespace/ALTER races remain unverified.

Capture takes a verified relation AccessShare lock, reads fresh metadata under
READ COMMITTED and retrieves all rows in one SELECT snapshot. A primitive
row-type witness is bound to that same FROM source, including empty relations;
it refuses a schema-name swap without recursively loading custom composite type
graphs. It does not claim a multi-command REPEATABLE READ snapshot. Capture is
complete or refused at 1,024 columns, 100,000 rows/cells including headers,
1 MiB per field, 8 MiB text and 16 MiB retained/encoded payload.

The saved recipe facade stores only exact connection/schema/table, format,
encoding, compression, NULL token, service-generated UUID and creation time.
It never stores file paths, rows, credentials or execution authority. One
profile-local versioned record is bounded at 128 recipes, 256 KiB encoded and
512 KiB retained payload; NULL tokens are at most 8 KiB. Saves compare an opaque
observed revision inside BEGIN IMMEDIATE and return the exact committed capture.
Future, corrupt, oversized and stale records refuse without replacement or
silent eviction. Loading is local and remains available while disconnected.
Rerunning needs a fresh target capture and destination selection.

Five focused configuration tests pass: exact quoted/Unicode and NULL-token
values, latest matching tuple and stale-save refusal, invalid-storage preservation,
count limits/profile separation, and escaped-JSON expansion before commit. The
initial compile failed on an unavailable UUID serde feature and concurrent missing
test modules; canonical UUID string serialization fixes it without changing the
pinned dependency graph. The final test log is the passing receipt. The debug
linker's compact-unwind-size warning does not change the successful test result.

Baseline `src/lib/export-tasks.ts` exposes Save and Run latest matching scope.
It does not provide a configuration edit/delete manager; the ledger wording is
corrected to that observed behavior. Existing browser `dbunk.exportTasks.v1`
import remains Plan 030 work.

Native controls now open options before capture. CSV prefills the exact NULL
token in the owned Transfer tool and refuses unsupported encoding/compression;
the six other formats require a complete immutable capture and a new save picker.
XLSX ignores text encoding, matching the baseline container behavior. Local
configuration loading does not execute a query. Load latest clears any prior
capture so rerunning requires fresh retrieval. Cancelled local acknowledgements
cannot replace newer state, and the document-owned configuration lane reopens
after a joined disconnect. Eight focused native export tests pass.

The initial 678-file source receipt predates two corrections: equivalent checked
division for Clippy and the missing isolated-profile module cfg found by the
first broad `just lint`. `source-final-sha256.json` records both corrections.
Required frontend checks and `just fmt`, `just lint`, serialized `just test` pass.
Core-only tests report 677 passed/71 ignored; Tauri tests report 694/85. The
isolated backend reports 1,079/99 plus two compile-fail doc tests; Tauri plus
isolated-profile backend tests report 231/28. The custom-protocol Tauri build and
36 Python tooling tests pass. The combined Tauri/isolated test build reports
existing cfg-dependent unused-import warnings; the recorded command succeeds.
Ignored tests are not passes.

The final native source matrix passes debug/release Clippy, tests and debug
build, with 369 tests passed and 13 ignored. Release packaging and its dependency
proof pass. [Initial window checks](window/README.md) verify the settings mirror,
all-row capture from a filtered grid, exact JSON/SQL/XLSX/CSV files, recipe save
and explicit reload, and normal quit with activity zero. They also find three
AX/focus/status defects; corrected target/status AX, click-to-keyboard focus,
complete/cancelled parent status and local recipe process reopen now pass in the
separately hashed `correction/` and `keyboard-correction/` builds. Individual key
checks exposed and fixed duplicate button activation; seven format steps pass.
The linked window record scopes each observation to its exact executable.
Broader button correction and full acceptance remain pending.
Real Tool-tab IME acceptance remains pending after [an inconclusive activation
attempt](ime/README.md).
VoiceOver remains deferred.
