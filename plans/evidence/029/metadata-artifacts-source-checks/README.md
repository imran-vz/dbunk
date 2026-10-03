# DDL export and schema map, 2026-10-03

Status: implementation and verification in progress. These are read-only
capabilities toward full PostgreSQL parity, not activation of the legacy
object-write service. No plan is complete.

The DDL artifact uses one owned read-only snapshot, exact database/schema/relation
identity checks and bounded reconstruction. It retains at most 1,024 relations,
4 MiB of SQL, 8 MiB actual payload and 16 MiB encoded data. UTF-8 ranges identify
each relation within one SQL string. It explicitly lists omissions, including
dependency ordering, non-relation objects, data, privileges and triggers. It is
not a canonical backup or execution authority. Native previews are paged at
32 KiB / 128 lines; complete SQL is saved through an owned, cancellable,
no-clobber file job.

Independent review found two shared-description defects: standalone unique
indexes used as foreign-key targets were incorrectly treated as constraint-owned
indexes, and quoted whitespace-only view names were rejected. The shared reader
now excludes only indexes owned by a primary/unique/exclusion constraint on the
same table and preserves nonempty quoted identifiers exactly. Both owned DDL
fixture probes pass, including mixed quoted/view/partition/materialized-view
coverage. Recorded temporary objects were removed and independently checked.

The schema-map reader captures complete typed table/column/constraint identities,
composite column pairs, cardinality reasons, uniqueness, nullable keys, junction
participation and compact trigger metadata. Database, schema/outgoing-external
target and relation/direct-neighbor scopes preserve their separate semantics.
The reader refuses partial graphs at its component/text/4 MiB bounds. Its first
live probe exposed a SQL row-alias collision: a foreign-key source column caused
`to_jsonb(source)` to produce a scalar. An explicit row alias fixes the reader.
The corrected owned probe passes; `corrected-map-teardown.json` independently
confirms all recorded objects absent and both fixtures at zero other backends.

Native scene geometry, Tool-tab lifecycle, scoped preference CAS storage and
SVG/PNG file jobs are integrated and under verification. Workspace format 12
stores the tab identity; loading versions 1–11 remains read-only. All 41 focused
workspace tests pass after updating previous-version/future-version expectations.
Preference storage has six passing focused tests, with exact revisions, explicit
reset tombstones and refusal of corrupt/future/oversized values. Existing Tauri
map preference/position import remains a Plan 030 gap.

The canvas and exports share bounded geometry. SVG saves the current viewport
on white at 1×; PNG renders that prepared SVG at 2×, with checked source, layer,
raster and encoded-output bounds. PNG uses bundled fonts and visibly refuses
missing glyphs. Full Unicode PNG coverage is therefore still a parity gap.
Parser/font working reservations are conservative estimates, not proof of every
allocator byte or process RSS. Shared retained payload admission remains 128 MiB
and the delivery queue remains 16 MiB. Details pages are 32 KiB / 128 lines.

Initial isolated-profile all-target Clippy passes. Seven DDL and seven map pure
tests, the map facade ownership test, eight shared-description tests and nine
native DDL model/view tests pass. Required `pnpm format`, `pnpm lint`, `pnpm typecheck`, `just fmt`,
`just lint` and serialized `just test` pass. The isolated backend run passes
1,061 tests (97 ignored) and two documentation tests. Native debug and release
Clippy/tests pass with 351 tests (13 ignored); dependency proof and all 36
Python tooling tests pass. The initial release package passes. [Initial window checks](window/README.md)
verify scoped DDL save behavior and expose map rendering failures. The
[corrected package and window recheck](correction/README.md) pass the rendering
corrections, keyboard/pointer movement, scoped persistence/reopen, stale refusal,
table opening and bounded exports. Its native debug/release matrices pass
355 tests (13 ignored). Ignored fixture probes are not counted as passes. VoiceOver remains deferred; keyboard/AX and
real Tool-tab IME remain required. The [single-source preparation attempt](ime/README.md)
was blocked by macOS retaining ABC; temporary settings were restored.

During teardown verification, the TLS ownership guard detected an external
OpenSSL alias update. The [owned runtime refresh](tls-runtime-refresh/README.md)
preserved the fixture UUID, data and certificates and passed the seven-case TLS
transport matrix. This is fixture maintenance, not native-window acceptance.
