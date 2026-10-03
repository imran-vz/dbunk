# Direct connection diagnosis checks, 2026-10-03

The separate package is
`/private/tmp/dbunk-native-package-20261003-diagnosis/dbunk Native Preflight.app`.
Executable SHA256 `80f3b9962a261cf6d936bb83ffc95e7fb6e39383e05aa614b924a741578e6b2b`,
bundle 129,968,319 bytes. All 476 source hashes and all package-file hashes match
after packaging. The manifest identifies dirty source, not a commit or cutover.

Required pnpm format/lint/typecheck and just fmt/lint/serialized test pass.
Final native debug/release all-target Clippy, fixture-harness Clippy and tests
pass: **275 passed, 13 ignored** in each test profile. Isolated backend Clippy
and tests pass: **965 passed, 89 ignored**, plus two compile-fail documentation
tests. Combined facade tests pass: **181 passed, 18 ignored**. Tauri's
custom-protocol build, native package and dependency proof pass. Ignored tests
are not passes. Python tooling did not change and its earlier evidence was not
rerun for this increment.

Early logs preceded the [session-option correction](./review.md). The final-*
native/isolated-Clippy logs cover it; isolated-test.txt includes its missing-role
failure test. The manifest retains both initial and corrected source hashes.
Initial visibility/test-type and borrowed-reference compiler corrections were
resolved before the final checks; initial logs are not final acceptance evidence.

The [live matrix](./live/native-tls.txt) passed on the separately verified owned
stage03 and stage04-TLS fixtures. It covers trusted verify-full, untrusted CA,
hostname mismatch, verify-CA, require/prefer TLS, observed protocol/cipher and
verification facts, a saved encrypted query session, and disabled-encryption
disclosure on the plaintext fixture. [Identity](./live/identity.json) binds the
helper executable and endpoints; [teardown](./live/teardown.json) records both
fixtures at 0 clients before and after joined shutdown. Only the new disposable
profile `/private/tmp/dbunk-native-diagnosis-20261003-live` was created.

[Progress and limits](../connection-diagnosis-progress.md) describe direct-only
scope, cancellation, credential reuse, bounded payloads and inherited platform
I/O/channel-binding limitations. No native window was launched for this package:
the current tool connection lacks cua_repl/native desktop control. Keyboard/AX,
new-control real IME and visual acceptance remain pending. VoiceOver remains
deferred. Plans 027–030 and full PostgreSQL parity remain IN PROGRESS.
