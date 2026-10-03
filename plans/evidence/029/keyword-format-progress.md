# Native keyword formatting progress, 2026-10-03

The existing Format SQL command now uppercases PostgreSQL keywords while preserving protected text, functions, types and exact bound parameters. [Source and baseline evidence](./keyword-format-source-checks/README.md) records eleven focused passes and 1,079 matching token-spelling cases, with explicit refusal and parameter differences. Required checks pass, including 246 native tests/13 ignored in debug and release and 36 Python tooling tests. The separate package matches 439 source hashes and includes the vocabulary license notice. Native-window selection/undo, keyboard/AX and real IME are still required; VoiceOver is deferred. Plans 027–030 remain IN PROGRESS.

A later [auto-fit package window check](../028/auto-fit-window-20261003/README.md) exercised keyword formatting, protected literals, one-step undo/redo and query execution. Full formatter selection/IME acceptance remains open.
