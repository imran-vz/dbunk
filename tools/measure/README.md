# measure

External measurement harness for Plan 024 (GPUI migration stage 01). One tool
for both desktop hosts: it posts synthetic input and reads frame timing from
the window server, so neither host's own timers are involved. macOS only.

```sh
swift build -c release --package-path tools/measure
tools/measure/.build/release/measure help
```

| Command | What it reports |
| --- | --- |
| `latency --pid P` | Key press to the first changed frame, p50 and p95 |
| `scroll --pid P --at fx,fy` | Interval between changed frames while scrolling, and long frames |
| `footprint --pid P` | Memory footprint and CPU over the process tree |
| `startup -- <executable>` | Spawn to first paint, main content and settled |
| `ax --pid P --tree` | The accessibility tree as an assistive client sees it |
| `place`, `chord`, `click` | Window size, a key with modifiers, a click |
| `target` | A calibration window whose response to input is known |

`editor-accessibility.swift` is a separate focused GPUI editor check. Compile
it with `swiftc tools/measure/editor-accessibility.swift -o /tmp/dbunk-editor-ax`
and run `/tmp/dbunk-editor-ax <spike-pid>` from the repository root. It asserts
text, focus, Unicode ranges, line reading, AX-driven selection, keyboard edits,
undo, empty-buffer recovery, shaped text bounds, scrolling, soft wraps,
window resize and keyboard focus between SQL, results and the cell editor.
Run with `DBUNK_SPIKE_FIXTURE=many` so the cell-editor checks have a row. It accepts only a process named
`dbunk-gpui-spike`, replaces that spike's synthetic SQL text, and does not touch
the clipboard. It complements the tree dump; it does not replace a human
VoiceOver check.

`suite.sh <pid> <out-dir> <grid-point>` runs the Plan 024 sequence against a
running app. `summarize.py <tauri-dir> <native-dir>` prints the comparison
table. `fixtures/` holds the shared document and the PostgreSQL views;
`tauri/` sets up the isolated Tauri build.

## Before running it

- The shell needs Screen Recording and Accessibility permission.
- Keys are posted to the target process only. Scroll steps and clicks go
  where the pointer is: the pointer is parked over the target window, and a
  run stops if the pointer leaves or another window covers the point. Leave
  the machine alone for the run.
- A run brings the target window to the front.
- Check that no system dialog is sitting over the window. One swallowed
  scroll events for part of Plan 024.

For foreground latency, scroll and footprint diagnostics, pass `--foreground`.
The capture is discarded if the target loses foreground ownership or another
normal window covers its center. The native performance runner enables this
guard. A process can receive PID-targeted keys while in the background, so
successful input delivery alone does not establish a valid foreground sample.
`python3 tools/measure/verify_foreground.py --out NEW_DIRECTORY` verifies the
guard using two owned calibration windows; it deliberately interrupts a run.

How each number is defined, the calibration result and the mistakes found on
the way are in `plans/evidence/024/measurements.md`.
