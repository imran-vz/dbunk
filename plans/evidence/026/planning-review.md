# Stage 03 planning and static layout review

2026-10-02. No native UI implemented. Canonical implementation scope:
`plans/026-native-postgres-workflow.md`. Plan status lives in `plans/README.md`.
Plan 025's review and fresh repository check logs are under `../025/`.

## Local publication

Only the new static artifact directory is served. No existing published site,
application build channel, profile, keychain or database is touched.

- Plan: http://localhost:18726/index.html
- A, stacked: http://localhost:18726/mock-a.html
- B, side by side: http://localhost:18726/mock-b.html
- C, results first: http://localhost:18726/mock-c.html

Reproduce from the repository root:

```sh
python3 -m http.server 18726 --bind 127.0.0.1 --directory plans/mocks/native-postgres
```

These are local review URLs, not a remote deployment. The available Sites
publisher requires a source commit and push, prohibited for this task. Files
are self-contained and can also be opened directly without a server.

## Browser verification

Used the T3 collaborative browser on the local review server. Inspected the
plan and all three mock pages at a measured 1402px viewport, then a 388px
content viewport inside a 390px review iframe. Native preview resizing timed
out; the same-origin iframe exercised the actual narrow media queries without
switching browser tools. All four narrow documents had scroll width equal to
viewport width and true black body backgrounds. Table scrolling stays inside
its region. Wide plan and mock A width checks also passed; all three wide
mock layouts were visually inspected.

All local state anchors resolve. The full-plan disclosure opens using Enter
when focused. Its complete implementation sequence and scenario table are
included inline. No external scripts, fonts or assets are required. Browser
snapshots reported no page error. An overflowing editor region found during
inspection was constrained to its own scroll area.

Application controls are deliberately static design examples, not a simulated
database client. Each option includes connecting, running, stopping/cancelled,
SQL error, connection loss/reconnect, queue failure, zero rows, script results,
truncation, policy refusal and closing states. Illustrative rows/timings are
labelled. The complete plan contains the native accessibility verification
requirements; this HTML review is not evidence that the future GPUI UI passes AX.

## Selection decision

Imran selected A + B + C on 2026-10-02 with a user-facing layout switcher.
The selection gate is satisfied. The switchable HTML preview is available at
http://localhost:18726/workspace.html. Stacked is the initial default; the
selected layout is a presentation preference over one workspace state.
Plan 026 now requires switching during streaming/cancellation without changing
the session, execution, ACK progress, results or editor state, plus fresh AX
geometry/focus checks after reflow. Native implementation remains pending.
Plans 024/025 remain separately reviewable; no plan was marked DONE and no
completion SHA was fabricated. No commits, pushes or PRs were made.

## Switcher preview verification

Follow-up on 2026-10-02: the live HTML layout selector cycles A/B/C/A by
changing pane geometry while retaining the identical SQL/result DOM nodes and
text. Remembered choice restores on reload. Results-first editor expansion
was exercised through the browser click tool. All three layouts fit 1402px
and 390px document widths with horizontal table scrolling contained locally.
Native dropdown key selection was not established by the automation; the
control uses a standard labelled HTML select. Native keyboard/AX verification
remains an implementation gate, not a claim from this preview.

`pnpm format`, `pnpm lint`, `pnpm typecheck` and `git diff --check` pass after
the plan/mock update. No Rust or application source changed in this follow-up.
