# Native PostgreSQL CSV transfers, 2026-10-03

Source implemented in the approved Tool tabs and table-context entry points. Required repository checks, native debug/release checks, the guarded real CSV probe and the separate package pass. Actual-window acceptance remains pending after CUA discovery failed. This does not close Plans 027–030 or establish full PostgreSQL parity.

## Behavior and ownership

CSV setup uses explicit native file selection, bounded dialect fields, indexed mapping, preview, exact immutable review and explicit start/confirmation. Duplicate and blank source headers remain distinct. Generated/identity targets cannot be mapped; omitted required targets without defaults refuse review. NULL, empty text and quoted NULL tokens remain distinct. Import appends in one transaction. Whole-relation export streams committed data, independently of grid filters, selections or staged changes, and refuses existing destinations.

The app-owned observer survives setup-tab closure. Abandoned unused inspections are cancelled, joined and released without starving other cleanup. Opaque reviews carry exact owner/revision identity, checked under the consuming transition lock. Reused inspection or attempt IDs cannot reuse old execution approval. Predispatch cancellation terminates without an orphan owner. Unknown outcomes require explicit reconciliation and are never automatically retried.

Native filesystem operations, PostgreSQL drivers and core workers retain joined ownership. Source CSV files use filesystem fingerprints and final revalidation through completion; they are not immutable snapshots. Private export publication does not overwrite an existing destination. A known publication remains successful if private-name cleanup fails, while cleanup ownership/admission remains retained. Connection/credential mutation cancels and joins CSV work under a deadline while holding an admission guard. Failed cleanup refuses mutation. Backup/restore mutation now conservatively refuses while matching work remains unsettled, avoiding a wait on a monitor that needs the same profile gate.

Import dispatch retires old data handles and fences new data handles and restore jobs until cleanup settles. Successful and unknown imports advance a lifetime revision; the native workspace invalidates affected metadata/editing sources while preserving drafts and SQL sessions. Revision gaps invalidate conservatively. No query or write is automatically rerun.

Backend bounds are eight inspections with five-minute expiry; four job slots and 32 retained terminal jobs; 4 MiB per immutable inspection within a 32 MiB inspection pool; and two 64 MiB execution reservations within a separate 128 MiB execution pool. Parser limits remain 1 MiB per field and 8 MiB per record. Preview scans at most 256 KiB/50 records with 64 KiB sample values. Native setup/capture/editor/catalog allowances share the application’s 128 MiB retained payload budget and 16 MiB delivery budget. These are not process RSS claims. Workspace format 6 persists only CSV tab identity/binding, refusing paths, samples and attempts.

## Focused and owned-fixture evidence

Focused backend checks pass: 15 CSV facade tests, 36 transfer/core tests and three backup/restore retirement tests. Native initial Clippy and 12 focused tests pass. Earlier failures remain in the focused directories, including a corrected fixture-profile policy assumption. Ignored tests are not passes.

The explicitly invoked CSV probe passed against stage03 `127.0.0.1:15432/dbunk_demo`, UUID `2283820d-33ec-4c4c-ae03-7051092bd410`. Unique schema `native_csv_64f2ddf8dcbc4e07b7186dfe63ec4a63` had captured schema OID 16659 and table OID 16661. Its temporary profile explicitly used Strict mode. Two rows exercised exact bigint limits, fixed-scale numeric, Unicode, quoted newlines/quotes, NULL/quoted NULL/empty values, generated/identity columns and database defaults. The 214-byte export was parsed and compared to exact database values. Replacing the source pathname was refused. A malformed record beyond preview scope rolled back preceding imported rows. Exactly one successful audit event and one import revision were observed.

Backend and helper joins preceded original-OID/owner/comment-guarded RESTRICT cleanup. The probe schema was absent afterward and activity returned from zero to zero. This probe does not establish TLS, window, keyboard/AX or real IME acceptance. VoiceOver remains deferred, not passed.

## Final source checks and corrections

`pnpm format`, `pnpm lint`, `pnpm typecheck`, `just fmt`, `just lint` and serialized `just test` pass. Core tests: 677 passed/71 ignored; Tauri tests: 694 passed/85 ignored. Native debug/release all-target Clippy, fixture-harness Clippy, debug build and both test suites pass: 230 passed/13 ignored each. Dependency proof and Tauri custom-protocol build pass. Corrected isolated all-target Clippy/tests pass: 910 passed/86 ignored plus two doctests. Tauri facade tests pass: 151 passed/15 ignored. Python tooling is unchanged and its prior checks were not repeated.

The initial native format failure required one string conversion on one line. Default Tauri Clippy then found helpers left unused by extraction: test/native-only helpers are now scoped to their actual callers and an unused test wrapper was removed. The backup/restore footer now describes mutation refusal until cleanup settles. Initial manifests/logs remain intact; verification-revision.json and verification-revision-2.json identify these corrections. The native-verified matrix covers the final production sources.

The first isolated suite passed 908 tests but failed two stale expectations: fixtures already supplied future version 7 but still expected UnsupportedVersion(6). Only those assertions changed. verification-revision-3.json records that test-only correction; backend-corrected contains the passing suite and remaining facade/build checks. The final 419-file manifest is source-sha256-final-tests.json. Runtime CSV semantics did not change after the live probe. Inactive generated Rust incremental caches were removed before the matrix; CARGO_INCREMENTAL=0 avoided further cache growth. Source, profiles, packages and older evidence were preserved.

## Package and window attempt

Package: `/private/tmp/dbunk-native-package-20261003-csv/dbunk Native Preflight.app`.
Executable SHA256: `6743917f18e4c9580b5a183b8f2198f3da05963acf613cd3754c4fd27bf104b9`.
Bundle size: 127,554,444 bytes. package-identity.json and post-package-source-proof.json bind the package to all 419 final hashes; the last two corrected assertions are test-only.

The owned launcher created `/private/tmp/dbunk-native-csv-20261003-review` and launched PID 16684 outside the repository. CUA discovery by exact package path and observed bundle ID both returned `cgWindowNotFound`; inventory reported the app running. No UI interaction followed. Exact executable hash/process command/profile/fixture-manifest checks preceded SIGTERM cleanup. Process exit was -15 and fixture activity was zero. The profile remains preserved. This is not a normal-quit pass or a source/global-outage diagnosis. See [window evidence](../csv-transfer-window-20261003/failed-discovery.json) and forced-cleanup.json.

Keyboard/AX and real IME acceptance remain required for the new controls. VoiceOver is deferred, not passed. No production, daily-driver cutover, commit, push or PR action occurred.
