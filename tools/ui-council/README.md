# UI council tooling

Screenshots of the native app (`apps/native`) for the design-council loop
(`comparative-ui-refresh` skill), adapted from web pages to a GPUI window.

| File | What |
| --- | --- |
| `drive.swift` | Window driver: screenshot one window, AX tree with window-relative frames, press/click by accessible name, keys and text sent to the app's pid, scroll, resize. Built to `.build/drive` on first use. |
| `capture.py` | Launches the release build against the owned PostgreSQL fixture with a fresh stage04 profile, plays `scenarios.json` and writes one PNG per `shot` step. |
| `scenarios.json` | The screens the council judges, as step lists. |
| `.council/` | Ignored. References, per-round shots, reviews, scores and plans. |

## Use

```sh
just native-fixture-up                                   # owned Postgres on 127.0.0.1:15432
(cd apps/native && cargo +1.98.1 build --release --locked)
python3 tools/ui-council/capture.py --out tools/ui-council/.council/round-0/shots
python3 tools/ui-council/capture.py --out /tmp/x --only workspace-table   # one scenario
python3 tools/ui-council/capture.py --launch            # one app left running; prints its pid
tools/ui-council/.build/drive ax --pid PID | less        # find names and frames for new steps
```

The calling terminal needs Screen Recording and Accessibility permission.
Clicks are real pointer events: leave the mouse alone while a run is going.
Every scenario starts a new app process with a new temporary profile, so no
user data or default profile is ever opened.
