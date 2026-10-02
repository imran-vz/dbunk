# Plan 024 Steps 2 to 4: measurements

2026-10-02. Apple M4 Pro (Mac16,7), macOS 27.0.1, built-in 120 Hz display at
1728 by 1117 points, AC power, Low Power Mode off, thermal state nominal for
every run. Both windows 1440 by 900 points for typing, scrolling, idle and
footprint. Raw output, one JSON file per run with every sample, is in
`tauri/` and `native/`.

## What was compared

| | Tauri | Native |
| --- | --- | --- |
| Binary | Release build of the application at `102568b` plus the Plan 025 working tree, `isolated-profile` feature, identifier `codes.imran.dbunk.plan024` | Release build of `spikes/gpui-path-z` |
| Frontend | Production bundle. Typing, scrolling, idle and footprint used the bundle with Plan 022's evaluation bridge, served on localhost; startup used the bundle embedded through the custom protocol, with no bridge | Zed's editor and the spike's grid |
| Data | The disposable PostgreSQL 16 fixture through a Query Session | A synthetic stream producing the same cells with the same retention limits |
| Profile | `/tmp/dbunk-plan024/config`, plain SQLite credentials | none |

The native side is a spike, not the application. It has no storage, no
connection, no navigator, no toolbar and no cell formatting, so it draws less
chrome and does less work per launch. Its grid is also larger on screen
(about 17 rows across the full window, against 7 rows in the Tauri layout).
Read the table as "the hard parts are not slower", not as a forecast for the
finished app.

## Result

Each cell is the median across runs. "Native vs Tauri" above +10% would fail
criterion 2.

| Metric | Tauri | Native | Native vs Tauri | Runs (Tauri / native) |
| --- | ---: | ---: | ---: | --- |
| Typing latency p95 (ms) | 40.7 | 24.7 | -39%  | 40.6 40.7 41.6 / 25.1 24.7 24.6 |
| Typing latency p50 (ms) | 32.8 | 16.1 | -51%  | 32.6 32.8 33.1 / 16.1 16.2 15.9 |
| Idle CPU (% of one core) | 5.5 | 1.3 | -77%  | 5.5 / 1.3 |
| Idle footprint (MiB) | 523.2 | 230.9 | -56%  | 523.2 / 230.9 |
| Scroll wide rows: frame interval p95 (ms) | 9.8 | 9.9 | +1%  | 9.8 9.8 9.8 / 10.0 9.9 9.9 |
| Scroll wide rows: long-frame share (%) | 0.4 | 0.0 | -100%  | 0.0 0.4 0.4 / 0.0 0.0 0.0 |
| Footprint with wide rows loaded (MiB) | 563.2 | 287.3 | -49%  | 563.2 / 287.3 |
| Scroll large cells: frame interval p95 (ms) | 10.8 | 10.0 | -8%  | 10.1 10.8 16.9 / 10.0 10.0 10.0 |
| Scroll large cells: long-frame share (%) | 4.6 | 0.0 | -100%  | 3.5 4.6 6.1 / 0.0 0.0 0.0 |
| Footprint with large cells loaded (MiB) | 564.3 | 294.7 | -48%  | 564.3 / 294.7 |
| Scroll many rows: frame interval p95 (ms) | 9.9 | 9.9 | +1%  | 9.9 9.9 9.8 / 9.9 10.0 9.9 |
| Scroll many rows: long-frame share (%) | 0.2 | 0.0 | -100%  | 0.2 0.2 0.2 / 0.0 0.0 0.0 |
| Footprint with many rows loaded (MiB) | 514.7 | 294.8 | -43%  | 514.7 / 294.8 |
| Scroll wide rows sideways: frame interval p95 (ms) | 17.4 | 9.9 | -43%  | 17.4 17.4 17.3 / 9.9 9.8 9.9 |
| Scroll wide rows sideways: long-frame share (%) | 7.7 | 0.0 | -100%  | 7.7 7.8 6.1 / 0.0 0.0 0.0 |
| Startup: first paint (ms) | 232.0 | 213.5 | -8%  | 231.6 256.9 232.0 / 178.5 237.1 213.5 |
| Startup: main content, last paint of half the window or more (ms) | 940.2 | 213.5 | -77%  | 939.9 940.2 940.3 / 178.5 237.1 213.5 |
| Startup: settled, last paint of 5% of the window or more (ms) | 3073.6 | 686.8 | -78%  | 3006.6 3073.6 3098.6 / 686.8 712.1 646.8 |

**Criterion 2 (no measured metric more than 10% worse than Tauri): met.**
The two positive rows are +1% on a frame interval of 9.8 to 9.9 ms, which is
inside the capture resolution of 8.3 ms.

Repeatability: the three typing runs agree within 3% at p95 on each host
(40.6 to 41.6 ms and 24.6 to 25.1 ms), inside the 10% the plan requires of
the harness.

## How each number was taken

- **Typing latency.** 300 key presses at 120 ms intervals, posted to the
  process, at the end of a comment line so no completion menu opens. The
  value is the time from posting the key-down to the display time of the
  first captured frame whose pixels differ from the previous frame. One
  capture interval is 8.3 ms.
- **Scrolling.** Scroll steps at about 195 per second (twice the refresh
  rate, less timer overhead) for 5 seconds; 4,000 points a second down the
  two long fixtures, 1,000 down the 400-row one, 2,000 sideways. Frame
  interval is the gap between consecutive changed frames. A long frame is a
  gap over 1.5 refresh intervals. The first 250 ms and last 450 ms are
  dropped.
- **Idle.** 30 seconds with the editor focused and nothing but the caret
  blinking. CPU is user plus system time over the process tree.
- **Footprint.** `phys_footprint` summed over the process tree. For Tauri
  that is the app plus WebKit's WebContent, GPU and Networking processes
  (about 50, 310 to 1,040, 145 and 7 MiB).
- **Startup.** The whole display is captured before the process is spawned.
  "First paint" is the first frame that changes at least 5% of the window;
  "main content" is the last frame that changes at least half of it;
  "settled" is the last frame that changes at least 5%. Startup windows were
  each host's default size (1200 by 800, and 1440 by 932 with its title bar).

## Calibration

A target window in the harness repaints on input after a delay the harness
does not choose. With no delay the measured latency was 10.8 ms p50 and 16.3
ms p95; with a 50 ms delay it was 63.9 and 69.4. The difference, 53 ms, is
the delay to within one capture interval.

## What went wrong on the way, and what it changed

- **ScreenCaptureKit's frame status is not a change signal.** For a window
  capture it delivers `complete` frames whether or not the window repainted
  and reports the whole window as dirty. The first calibration measured a 50
  ms delay as 14 ms. The harness now compares every frame's pixels with the
  previous frame in 32-pixel tiles.
- **Scroll steps posted to a process do not reach a WebView's content.** The
  first Tauri scroll runs showed two frames a second, which was the caret.
  Scroll steps now go through the session event tap with the pointer parked
  over the target and a guard that discards the run if it leaves.
- **Scroll steps at the refresh rate manufacture long frames.** At a nominal
  120 steps a second the loop delivered 99, so one refresh in five had no new
  position, and the native grid showed 19% long frames where it now shows
  none. WebKit showed 3% under the same input because it smooths scrolling.
  Steps are now posted at twice the refresh rate.
- **WebKit's helper processes were not counted.** They are started by
  launchd, and when the app is launched from a shell their responsible
  process is the terminal, not the app. The first Tauri footprint was 48 MiB.
  The harness now also counts WebKit services that share the app's
  responsible process and started after it.
- **Two system alerts sat over the centre of the screen for most of the
  session**: a crash report for the spike's first launch, and a macOS prompt
  asking whether T3 Code may bypass the window picker for screen capture.
  They swallowed scroll events in the upper half of the screen, which is why
  the schema-map probe scrolls from the lower half. They do not appear in
  window captures and did not sit over any measured scroll point. They are
  inside the startup capture's window area but do not change, so they only
  lower the measured coverage slightly. Neither was dismissed or answered.

## Limits

- One machine, one display mode, one power state, one macOS version.
- Synthetic input, not a physical keyboard or trackpad. Key events are
  posted to the process and skip part of the HID path; both hosts are
  measured the same way.
- WebKit's memory moves with garbage collection. Across two passes the Tauri
  footprint with a fixture loaded ranged from 515 to 1,247 MiB; the table
  shows the second pass. The native range was 170 to 295 MiB.
- Tauri's "settled" time of 3 s was the same on an empty profile, so it is
  not session restoration. What paints at 2 and 3 seconds was not identified.
- IME, undo and redo, search and multi-cursor editing were not exercised by
  the harness. They are Zed editor features and work in Zed; they were not
  checked in the spike.
