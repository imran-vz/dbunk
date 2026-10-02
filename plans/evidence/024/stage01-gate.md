# Stage 01 gate review

2026-10-02. Review requested by Imran: “Finish text geometry for accessibility
highlighting and verify keyboard focus between editor and results. Review and
close the stage 01 gate.” This authorizes the gate decision recorded here.

## Decision

**CLOSED: PASS / continue on path Z.** All four criteria are met. This
record concerns the synthetic macOS spike and the migration viability gate.
It does not approve daily-driver cutover,
production access, a licence change to the current application, or mark Plan
025 reviewed. No commit, push or PR is part of this task.

## Review of the four criteria

1. **Architecture reason: met.** Stage 00 explicitly requires no performance
   gain. Core extraction alone removes host dependencies below the command
   layer; a second host additionally removes the WebView/IPC boundary and
   duplicated frontend types at cutover. That is the chosen reason to accept
   the larger rewrite, rather than claim a performance need.
2. **No measured regression above 10%: met in the recorded baseline.**
   Recomputed all 18 rows from the saved JSON with `tools/measure/summarize.py`;
   the worst is +1%. The spike has less chrome than Tauri. The original
   measurements predate the adapter and are not a prediction for the full app.
   Follow-up accessibility-active measurements are recorded below.
3. **No capability gap without a costed fix: met.** The SQL and cell
   editors now expose shaped character bounds, visible soft-wrap segments,
   primary selection and focus using the pinned public APIs. The external AX
   probe exercises scrolling, resizing and keyboard-only transitions through
   SQL, results and the cell editor. Imran's earlier VoiceOver confirmation
   remains the human evidence; this session adds automated evidence, not a
   new listening check. Remaining gaps have explicit work allowances below.
4. **Dependency path: met.** The lockfile and Zed revision are unchanged.
   The existing clean-build, licence, single-GPUI and network-denied startup
   evidence still applies. Fresh release build, lint and focused tests pass.
   Rust 1.98.1 is required; this spike uses runtime shaders, while a packaged
   build should install Xcode's Metal Toolchain.

Plan 024 remains `READY FOR REVIEW` solely because no authorized completion
commit exists. Keep its plan body until a completion SHA is recorded. Plan 025
retains its separate review status. The stage 00 backend-first rule now
applies: React receives correctness, safety and data-loss fixes; new capability
lands dark in the core and its UI is built natively. The parity baseline
remains `102568b`; no React feature was added by this work.

## Accessibility-active performance follow-up

Fresh final release binary, 2,000-line SQL fixture and synthetic `many`
results, 1440 × 900 points, same M4 Pro / 120 Hz display on AC power. AX was
activated before measurement. Three runs of 300 keys at 120 ms, followed by
30 seconds idle. Capture resolution is 8.3 ms. No keys were missed.

| Metric | Final native, AX active | Recorded Tauri baseline |
| --- | ---: | ---: |
| Typing p95, median of runs | 32.8 ms (32.8, 33.9, 32.7) | 40.7 ms |
| Typing p50, median of runs | 27.9 ms | 32.8 ms |
| Idle CPU, one core (foreground interrupted, diagnostic only) | 3.4% | 5.5% |
| Idle footprint | 260.0 MiB | 523.2 MiB |

The repeat typing runs agree within 4% at p95, with no missed updates.
Typing and footprint stay below the Tauri baseline and its +10% limit.
Accessibility has a real cost: the earlier native p95 was 24.7 ms and idle CPU 1.3%. This follow-up is an AX-active
stress check against the saved baseline, not a new matched VoiceOver
benchmark; it does not replace the original 18-metric comparison. Startup
and grid scrolling were not rerun for this text/focus change. The original
synthetic-spike limitations still apply.

One earlier run on this same final binary reported 57.4 ms p95 and six
missed captured updates, between two runs at 32.7 and 33.0 ms with none
missed. Its cause was not established. All three runs and their idle sample
are retained under `accessibility/performance/attempt-1/`; they are not
silently discarded or labelled a proven foreground interruption. The bounded
repeat passed all three runs. Its foreground-state log stayed true throughout
typing. The window lost foreground at 06:56:47Z, about four seconds before
the idle sample ended at 06:56:51Z. The idle CPU result is therefore
diagnostic only, not a clean foreground comparison; the original stage 01
idle evidence remains the gate evidence. No further repeat was run. The first
anomaly remains an unexplained outlier, not evidence of a diagnosed and
repaired runtime regression.

Raw repeat files: `accessibility/performance/latency-{1,2,3}.json`, `idle.json`
and `foreground.txt`.
Build identity and verification counts: `accessibility/geometry-build.txt`.
Final AX assertions: `accessibility/editor-geometry-focus.txt`.

## Remaining work and cost

These are engineering estimates in person-days, not measurements or promises.
They cover the gaps found in stage 01, not the entire 75-item feature port.
Each later implementation plan must budget and verify its applicable rows.
An unsuccessful feasibility probe reopens the affected decision before that
surface ships; it does not silently waive current behavior.

| Gap or unverified behavior | Fix / verification route | Allowance and deadline |
| --- | --- | --- |
| Full forms, focus order, validation announcements and dialog focus return | Apply roles, explicit focus order and the editor adapter to each control; verify keyboard and AX flows against Tauri | 5–8 days in stage 04, plus 1–2 days per later complex form |
| Secondary cursors, folded/inlay text and offscreen visual-line semantics | Extend the display adapter; preserve primary selection as the platform selection and expose extra state through supported semantics; add focused display-map fixtures | 3–5 days before full editor parity in stage 05 |
| Bidirectional text | Arabic/Hebrew shaping and hit-test probe first; budget a GPUI text-layout fix or a dedicated bidi renderer if needed | 2 days investigation plus 5–10 days repair allowance before stage 05; reopen dependency choice if neither route preserves text |
| IME in SQL and cell editors | Human composition checks with a real input method, including selection replacement and focus return; repair the embedding boundary if needed | 1 day verification plus 2–3 days repair allowance before editor parity |
| Embedded find, SQL formatter, diagnostics and statement boundaries | Embed search independently of the workspace toolbar, compare a Rust formatter on dialect fixtures, map server offsets, reuse core SQL lexing | 5–8 days in stages 03/05 |
| Grid keyboard navigation, active-cell announcements, copy/staged-value parity, scrollbars and header lag | Complete the selection model and active descendant; use the staged value consistently; share horizontal scrolling; add the existing scrollbar primitive | 4–6 days before native result-grid parity |
| Specialized cell editors beyond the JSON probe | Position at the cell, validate types, provide date/enum/boolean/array editors and wire staged mutation review | 4–7 days in stage 05 |
| Schema-map layout, routing, selection, saved positions and minimap; pan cache and zoom integration | Cull edges, simplify retained paths, scale canvas-owned geometry, add layout/routing and persistence; verify zoom/drag | 7–12 days before schema-map parity |
| Browser-supplied selectable labels, radio/number controls, toasts and split panes | Explicit copy/read-only text surfaces and reusable GPUI controls | 6–10 days across stages 04/05 |
| Inherited editor and grid interactions not covered by stage 01 automation | Focused undo/redo, find, multi-cursor, clipboard, selection and resize scenarios against Tauri | 2–3 days before editor/grid parity |
| Packaging and long-term dependency maintenance | Probe an `.app`/DMG build, signing and update path; pin Zed and rerun the AX suite on upgrades | 2–3 days packaging probe before stage 04, 5–8 days release integration in stage 07; 1–2 days per routine Zed update, larger breaks estimated separately |

The dependency cost remains substantial: 811 resolved packages, 102 Zed
crates, no stable embedding API, and slower iteration than Vite. The scope
remains macOS Apple Silicon. Other platforms have no validation from this gate.

## Verification

- External macOS AX: highlight containment in the actual SQL/cell-editor
  screen frames, Unicode text and glyph advances, multiline ranges,
  empty/trailing carets, vertical/horizontal autoscroll, soft wrapping,
  resize/reflow, stale offscreen bounds, selection and edit notifications.
- Keyboard focus: F6 and Shift-F6 between editor/results; Tab and Shift-Tab
  from results; Tab/Shift-Tab retain indentation in both editors; Enter into cell editor; F6 away and back; Escape and Cmd-S
  return to results; SQL selection and undo survive the round trip.
- `pnpm format`, `pnpm lint`, `pnpm typecheck`, `just fmt`, `just lint`,
  `just test`: pass. Backend tests: 633 passed / 70 ignored without Tauri;
  657 passed / 85 ignored with Tauri. Ignored tests need external fixtures.
- Native `cargo fmt --check`, release Clippy, release tests: pass, four
  focused tests. Release build passes. The existing transitive `block`
  future-compatibility warning remains.
- All input targets only the owned synthetic spike. No database, dbunk
  profile, keychain, Tauri build or daily-driver preview is touched.

Detailed adapter evidence: [editor-accessibility.md](./editor-accessibility.md).
Original evidence: [measurements](./measurements.md), [dependency
path](./step1-dependency-path.md), [probes](./probes.md), [browser
behavior](./browser-provided-behavior.md), [iteration cost](./iteration-speed.md)
and [parity checklist](./parity-checklist.md).
