# Native SQL layout formatting, 2026-10-03

Status: implemented; required and native debug/release source checks pass. The separate package and native dependency proof pass. This is a conservative,
dependency-free whitespace formatter. It preserves token spelling rather than
matching the baseline formatter's keyword capitalization. Full T01 parity and
actual-window keyboard/AX, undo/selection and real IME acceptance remain open.
VoiceOver is deferred, not passed.

## Decision

The existing transitively pinned `sqlformat` 0.2.6 and baseline JS formatter were
probed against a PostgreSQL corpus in owned temporary scratch. Both can change
newline-separated string-literal semantics; the Rust formatter also changes
some dollar bodies, nested comments and operators. No dependency was added.
The native command instead edits only bounded whitespace gaps between protected
lexemes and refuses uncertain input while retaining the original draft.

PostgreSQL explicitly distinguishes newline-concatenated strings, escape-string
continuations and immediately adjacent Unicode prefixes. Reference:
[PostgreSQL lexical structure](https://www.postgresql.org/docs/18/sql-syntax-lexical.html).

## Editor integration

Query > Format SQL and Cmd+Shift+F act on the complete current query draft when its SQL editor has focus. Other focused fields are refused.
Busy/read-only editors and actual marked-text composition refuse the command.
Admission precedes copying the bounded draft. Completion UI is dismissed before
applying whitespace edits in one ordinary editor transaction with explicit boundaries before and after it, preserving token
anchors and triggering normal draft persistence. This source behavior still
requires native-window verification; model tests cannot prove it.

## Bounds and source verification

Input and output are capped at 64 KiB, tokens at 4,096, delimiter/comment nesting
at 64 and edits at 4,097. The model measures output and edit sizes before
allocating edit text. A 4 MiB shared working allowance is admitted before copying
the source. This covers temporary formatter work, not retained Zed editor/undo
history or process RSS. Empty and whitespace-only drafts are unchanged.

Eight focused tests pass. The copied [baseline probe](./baseline-probe/summary.json)
contains 38 cases; the native formatter accepts 36 with exact repeat idempotence
and refuses two (unfinished literal and non-ASCII outside whitespace). This is a
lexical corpus, not PostgreSQL execution or full dialect acceptance. Independent
review also exercised ten lexical edge cases and found no further corruption.

Required frontend format/lint/typecheck and Rust fmt/lint/serialized tests pass:
core 677 passed/71 ignored; Tauri 694 passed/85 ignored. Native format, debug/release all-target Clippy, fixture-harness Clippy, debug build
and debug/release tests pass: 184 passed/13 ignored in each test run. All 337 frozen
source hashes match after package completion; the separate package and native dependency proof pass. Ignored
tests are not passes. Backend facade/custom-protocol behavior is unchanged from the
frozen completion evidence and those additional suites were not repeated.

## Frozen package

Package: `/private/tmp/dbunk-native-package-20261003-formatter/dbunk Native Preflight.app`.
Executable SHA256: `ac1f0e92ee530cf79f98bfc4260d2e29ef97ae77704a957e6ff0b5661fcc5579`.
Bundle size: 123,106,300 bytes. See `package-identity.json` and `package.txt`.
No window launch was attempted for this package after the preceding completion
package failed CUA discovery. This package has no actual-window acceptance.
The [completion attempt](../completion-window-20261003/failed-discovery.json) is
prior blocked verification, not a formatter-window test.
