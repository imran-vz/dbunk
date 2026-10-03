# Plan 027 planning review

2026-10-02. Drafted against `3f987c96640d6738b48ae1113ceac3ebbdc8563f`.
No implementation or runtime verification is claimed.

[Plan](../../027-native-workspace-shell.md) · [Self-contained HTML review](../../mocks/native-workspace/index.html)

## Scope

The proposed next phase is migration stage 04's PostgreSQL workspace shell:
credentials, connection forms, multiple query tabs and restoration. It retains
the three Plan 026 query layouts. Imran selected A, Persistent Navigator, on 2026-10-02.
The reviewed shell choices are A (Persistent Navigator, selected), B (connection
strip and vertical queries), and C (connection index and focused workspace).
They share scope; B and C remain unselected alternatives. No implementation has been requested or started.

Read the migration sequence, stage 01 remaining-work budget, Plan 026 closure,
ADR-0007 and ADR-0032, current backend facade, credential/keychain code,
settings adapters, workspace/session persistence and design-system metrics.
The draft explicitly budgets the packaging preflight, credential namespace and
cache isolation, strict error handling, shared session budgets, background ACK
draining, and draft-save failures. Credential-policy changes affecting Tauri
require a separate scope decision rather than an accidental extraction change.

## Static publication and checks

Only the new artifact directory is served on a separate loopback port:

```sh
python3 -m http.server 18727 --bind 127.0.0.1 --directory plans/mocks/native-workspace
```

Review URL: http://localhost:18727/

The T3 collaborative browser loaded the artifact. At 1402 px its page width
and scroll width match. A same-origin 390 px iframe, with both detail sections
expanded, also has matching page/scroll widths. No headings, prose, plan tables,
form-state sections or decision content extend outside the narrow viewport.
The three wide static windows intentionally scroll inside their own labelled,
keyboard-focusable containers. Both viewports use a true-black background.

The section links resolve, with the layouts anchor landing 16 px below the
viewport top. The shared form-state disclosure responds to click and Enter;
the full plan disclosure opens and contains the executable-plan draft.
There are no scripts or external resource dependencies and no failed resources
in the inspected page. Mock application controls are explicitly static examples.

Snapshot capture failed twice; viewport resize also timed out. DOM/layout
inspection succeeded, and narrow layout was measured through the iframe.
No screenshot-based visual review or native accessibility pass is claimed.

`pnpm format`, `pnpm lint`, `pnpm typecheck` and `git diff --check` pass.
Only planning documents and static artifacts were changed. No native source,
credential store, database, daily-driver build, or published application channel
was touched. Prior Plan 026 closure edits were preserved. No commit or PR.
