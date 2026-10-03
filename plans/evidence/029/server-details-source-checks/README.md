# Server facts, settings and extensions, 2026-10-03

Status: implementation, focused/live checks, frozen repository/native verification and separate packaging pass. Native-window discovery failed before interaction. Actual-window acceptance remains pending. This is the read-only server-inspection portion of Plan 029 T12, not complete administration or overview parity.

Baseline authority and deliberate differences are in [the source reconnaissance](../overview-recon.md). The baseline has a server-details backend/store loader but no active component consumer was found. The native sections implement the planned target within the approved Administration Tool tab.

## Native behavior

Server facts, Settings and Extensions share the Administration document's explicit Connect, Refresh, Cancel read and Clear captures actions. Refresh reads the selected family; section switching and typing send no SQL. Activity and server captures retain independently, with one serialized read lane and monotonic request IDs. Cancellation drops late replies and preserves the last capture. Disconnect marks retained readings stale. Settings and extension selection follows stable names across refresh/filter changes; a disappeared selection does not silently become a different row.

Settings search is applied explicitly to captured names, categories, sources and descriptions, excluding setting/boot/reset values. Unicode lowercase matching is bounded. **Non-default source** means source is not `default`, excluding the two inspection timeout overrides. It is not a comparison against boot values. NULL, omitted cells, restricted/unavailable sections and partial row/byte-limited captures remain distinct, including when filtering leaves no visible rows. Copy selected details verifies immediate exact clipboard readback.

The readings describe the inspection connection, not the SQL editor session or an unmodified server-wide configuration. Details include reader PID, current/session user, search path and collection interval. Statement and lock timeouts set locally by inspection are flagged. Database `LC_COLLATE` comes from `pg_database.datcollate`; it is not presented as an ICU locale.

## Bounds and ownership

The facade uses the existing admitted DataDocument/object-read lane, endpoint capability, connection generation, policy/credential fences, cancellation epoch and joined driver ownership. No legacy global connection path is activated. The read-only repeatable-read transaction applies a statement timeout of at most 10 seconds, lock timeout of 2 seconds and the existing 30-second cooperative operation deadline. These do not claim hard wall-clock limits for synchronous TLS-file work or joined cleanup.

SQL guards text at 8 KiB per cell and identities at 256 bytes before materialization. Oversized values are explicitly omitted with original byte counts; invalid/oversized identities refuse. Settings cap at 1,024 rows and extensions at 256, with cap-plus-one detection and streamed rows. Header/settings/extensions have independent 64/768/192 KiB allowances under a 1 MiB encoded and checked-capacity limit. Savepoint recovery classifies only SQLSTATE 42501 as restricted; unrelated SQL and decode errors remain failures.

The native capture reserves 2 MiB under the shared 128 MiB retained-payload budget before allocating search indices. Replacement reserves old and new simultaneously and retains the old capture on refusal. The filter has a separate lifetime 256 KiB allowance, a 1 KiB query limit and bounded editor history. Normal marked composition survives; oversized input/history restores committed text. No composition or keyboard claim follows from source checks. Delivery remains within the shared 16 MiB queue budget. These are payload/work allowances, not process RSS claims.

## Focused evidence and reproduced failure

Seven backend model/bounds groups pass, plus seven native capture/search tests and two filter/history/reservation tests. Independent source review found two issues before freeze: filter history needed a shared allowance before any capture, and validation needed to be identical before and after the first capture. Both are corrected. Activity replies also leave server-section scroll position alone.

The first owned stage03 live probe failed with Catalog(Database). Its log and identity are preserved in `owned-live.txt` and `owned-live-identity.json`. Running each static query read-only isolated FACTS: PostgreSQL 17.11 returned `unrecognized configuration parameter "lc_collate"`. Other static queries passed. See `static-query-diagnosis.json`. The reader now uses the current database catalog column, and the original full live probe passes in `owned-live-locale-fix.txt`. It verifies facts/settings/extensions, inspection timeout flags, idle cancellation followed by reuse, retired-owner refusal and joined cleanup. It does not verify cancellation during an in-flight query.

Target: owned `dbunk-native-stage03`, `127.0.0.1:15432/dbunk_demo`, UUID `2283820d-33ec-4c4c-ae03-7051092bd410`. Both failed and corrected runs returned activity from 0 to 0. Test profiles were private marked temporary directories removed by their test scope. No persistent database objects/settings were changed and no setting values were logged during diagnosis.

## Frozen verification

Source manifest: `source-sha256.json`, 361 files. Frontend format/lint/typecheck and required Rust fmt/lint/serialized tests pass: core 677 passed/71 ignored, Tauri 694/85. Isolated backend passes 847/83 and two doctests; Tauri facade passes 96/12. Native format, debug/release all-target Clippy, fixture-harness Clippy, debug build and debug/release tests pass: 196 passed/13 ignored. The Tauri custom-protocol build passes. Ignored tests are not passes; only the named owned live test was explicitly executed.

The first broad native test run exposed one stale assertion for the renamed locale label (195 passed/1 failed/13 ignored). The assertion now expects Database LC_COLLATE, preserving its NULL-vs-empty check. `native-test.txt` retains the failure, `native-test-final.txt` the passing rerun. The before-label source manifest is retained separately. Python tooling was unchanged and was not repeated; its earlier scoped evidence is separate. The guarded package launch ran, but CUA discovery failed before interaction; see the exact package/window scope below. Keyboard/AX and real IME remain required; VoiceOver remains deferred, not passed.

## Remaining scope

Database/relation/schema statistics, recent-query overview, read-only connection Settings mirror/Edit navigation, audit list, backend control actions and maintenance remain open. This read-only settings catalogue does not edit server configuration. Complete PostgreSQL parity and Plans 027–030 remain open.

## Frozen package and blocked window acceptance

Package: `/private/tmp/dbunk-native-package-20261003-server-details/dbunk Native Preflight.app`.
Executable SHA256: `8d5a87a5862ad14767f5c49d959afc0128f6964fd159c29ab0e4a47843bf9c4a`.
Bundle size: 123,411,468 bytes. Packaging and native dependency proof pass. All 361 frozen source hashes match after packaging (`post-package-source-proof.json`). Later DDL work is a separate source increment.

The guarded workspace launcher created the isolated stage04 profile `/private/tmp/dbunk-native-server-details-20261003-review`, PID 30652, bound only to the checked stage03 fixture. CUA failed with `-10005 cgWindowNotFound` for both exact bundle path and observed bundle ID; inventory reported the app running. No click, key, screenshot or application state verification followed. See [window identity/failure/cleanup](../server-details-window-20261003/failed-discovery.json). This does not establish Ready, focus, AX, IME, server-section rendering, filter/copy behavior or normal quit.

Exact process command, profile and executable hash were checked before SIGTERM. The process stopped, fixture backend activity remained zero and the isolated profile was retained. Forced cleanup is not a passed quit gate. VoiceOver remains deferred; its existing checklist is preserved.
