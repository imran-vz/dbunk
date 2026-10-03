# PostgreSQL URI import and copy, 2026-10-03

Status: source checks and separate package pass; actual-window acceptance is pending. This is the URI
portion of Plan 029 T14, not staged diagnosis, SSH or managed-server parity.
Baseline: [source reconnaissance](../connection-uri-recon.md), Tauri `102568b`.

## Native behavior

A new connection form exposes **Import URI from clipboard**. It reads one plain
text clipboard entry, performs a bounded complete parse, then prefills host,
port, user and database. Absent/empty password and absent sslmode preserve the
current password and TLS choice. Name, policy, organization, certificate paths
and driver fields remain unchanged. Import neither saves nor tests/connects.
Marked text in any form field refuses import until composition finishes.
Independent source review also found the form capture handler consumed Escape
and Tab during composition. It now leaves those keys to the focused editor
while marked text exists; actual IME verification is still pending.

The explicit clipboard action replaces the baseline URI editor. Raw URIs are
not retained in a form, workspace or editor undo history. Changed fields receive
fresh buffers; passwords remain masked with secure accessibility semantics.
The operating system clipboard allocation precedes the parser and is outside
its byte bound. This is not a clipboard, heap-zeroization or process RSS bound.

General-profile new forms now default to TLS Prefer, matching the baseline.
Fixture defaults remain Disable. Existing records retain their stored mode.

Supported saved PostgreSQL records expose **Copy URI** separately from Duplicate.
The builder accepts secret-free metadata, never retrieves a credential, and
omits password, saved policy, TLS files/server name and driver options. The UI
discloses omissions. GPUI writes have no Result, so the action verifies immediate
exact clipboard readback and reports failure if it cannot confirm the copy.
That is not a guarantee against a later clipboard change by another application.

Existing profile admission remains authoritative at Test/Save/Connect. Import
cannot widen an owned fixture manifest. No network, certificate reads or secret
lookups are part of the pure parser/builder.

## Parser contract and deliberate corrections

The pure facade uses the existing `reqwest::Url` reexport, with no manifest or
lock changes. Input is capped at 16 KiB before decoding, host/user/database at
256 bytes each, password at 4 KiB, query pairs at 32 and decoded query keys at
128 bytes. Export is premeasured and capped at 4 KiB. Parsed Debug and errors
never expose the source, password or ignored option values. Only the five exact
supported sslmode values apply. Other option names are disclosed without their
values; certificate paths from URIs are never trusted or applied.

The baseline silently drops some ambiguous input. Native deliberately refuses
malformed percent/UTF-8, controls (including outer CR/LF), zero/empty ports,
encoded/socket/multiple-host/zone-ID authorities, extra path segments, fragments
and conflicting duplicate sslmode values. Identical duplicate sslmode is allowed.
Raw database `.` and `..` names remain exact instead of undergoing URL path
normalization. Encoded database slashes, Unicode and IPv6 round-trip. These are
correctness differences, not claims of byte-for-byte baseline parser behavior.

## Verification

Six focused backend test groups pass under backend `url 2.5.8`; isolated-backend
all-target Clippy, backend formatting and native all-target integration Clippy
pass. The independent source review found no remaining parser/integration issue
after the host-character and marked-text keyboard fixes. The source is frozen at
349 hashes. Frontend format/lint/typecheck and required Rust fmt/lint/serialized
tests pass (core 677 passed/71 ignored; Tauri 694 passed/85 ignored). Native
format, debug/release all-target Clippy, fixture-harness Clippy, debug build and
debug/release tests pass (187 passed/13 ignored). Isolated backend passes
840 tests/82 ignored and two doctests; Tauri facade passes 96/11 ignored. The
custom-protocol build passes. Ignored tests are not passes. The same six URI tests pass against the exact backend
rlib emitted by the native Cargo build, using native `url 2.5.7`. Cargo refused
testing a dependency package with dev dependencies directly; a temporary rustc
harness links the recorded native artifact instead. Its initial missing-parent-
imports error was corrected without repository source changes. Both failed setup
logs are retained; see `native-graph-uri-proof.json` and the final corpus log.
All 349 source hashes match after package completion; package and native dependency
proof pass. Python tooling and live database suites were not repeated for this
pure URI increment; their unchanged scoped evidence remains separate. The preceding
[general-profile window attempt](../../030/general-profile-window-20261003/failed-discovery.json)
failed CUA discovery before any UI interaction; it does not validate this URI
increment. Actual-window, keyboard/AX and real IME checks remain pending.
VoiceOver remains deferred, not passed.

## Frozen package

Package: `/private/tmp/dbunk-native-package-20261003-uri/dbunk Native Preflight.app`.
Executable SHA256: `1c4a0e6f6fc7ee88843e858ed075e8cc39b603f5b7ecf300b554521c05347789`.
Bundle size: 123,171,292 bytes. See `package-identity.json`, `package.txt`
and `post-package-source-proof.json`. No window launch was attempted after the
preceding general-profile package failed CUA discovery. That earlier failure is
not a URI-window test. Import/copy, form Escape/Tab composition behavior, focus,
clipboard error reporting and normal close/reopen still need actual-window checks.
