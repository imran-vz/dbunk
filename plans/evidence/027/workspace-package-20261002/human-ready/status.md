# Human acceptance handoff pending

The frozen preflight package launched the existing plain-SQLite profile `/private/tmp/dbunk-native-workspace-final-20261002`. PID 16493 remains alive under the verified launch identity. Both fixtures were idle at launch. No human acceptance has been claimed.

The first probe failed its exact AX window title check. A scoped diagnostic found the actual CoreGraphics window titled `dbunk Native Workspace`, visible at 1296×840. macOS supplied a placeholder AX application node as the window and denied window image capture. A subsequent read-only session check identified `CGSSessionScreenIsLocked=1`. This explains the unavailable accessibility tree; it does not establish a native application startup defect.

The probe now explicitly rejects a locked macOS session before activation or UI events. Its traversal also detects repeated AX nodes instead of recursing indefinitely through the locked-session placeholder. The exact AX workspace title guard remains unchanged. The failed diagnostic attempts and startup sample are retained alongside the final explicit `locked-session.log`.

Foreground automation is paused until the user unlocks macOS. After unlock, run the `human-ready` step against this identity once, inspect the owned-window screenshot, then leave the window open without further foreground automation for VoiceOver and IME checks.

## Handoff completed after unlock

The single `human-ready` pass in `check-unlocked.log` succeeded after macOS was unlocked. The probe reverified the running package's hash, canonical executable/bundle, PID, exact launch arguments, profile/fixture identity and exact accessibility window title. It restored the acknowledged Unicode SQL, explicitly connected the fixture connection, observed completed result `42`, focused the SQL editor, and captured `screenshot-human-ready.png`. Visual inspection found the navigator, query controls, SQL editor, results and saved-draft status visible without clipping.

PID 16493 is left open as `dbunk Native Workspace`, with profile `/private/tmp/dbunk-native-workspace-final-20261002` and the same frozen package binary. All foreground automation is now stopped. The launcher remains active; this handoff does not claim a teardown/baseline check or human VoiceOver/IME acceptance. A separately coordinated backend test was using its own fixture schema during this pass, so no fixture-count acceptance was inferred.
