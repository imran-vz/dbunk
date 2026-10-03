# Capture symptom comparison, 2026-10-03

The preceding frozen maintenance-receipts package, SHA256
9a685ef478ebca83c8c01f2b8d32c7c8b7c52017a2eab6b94843e9c159a43fe6, was launched
through workspace_launch.py in the same owned profile, PID 94623. It contains no
column-pinning implementation. Only native_parity_20261003.rows on owned stage03
was read. The first connected screenshot showed the current 60-row table. Clicking
row 1 then pressing Down changed AX selection to row 2; the next screenshot lost
most unchanged text and selection content while retaining borders and some labels.
The connected and moved AX/pixel pairs are retained.

This reproduces the missing-content capture symptom without pinning. It does not
establish whether the cause is capture, compositor/foreground availability or a
shared pre-existing native rendering issue. No speculative rendering fix was
made. Coordinate/wheel automation also reported noWindowsAvailable during the
new-package run. Visual acceptance needs a reliable foreground/capture loop.

Normal quit exited 0; both owned fixtures returned 0 → 0. No preferences or database
rows were changed by this comparison. Queue high-water 1,705,472, remaining 0,
not an RSS measurement.
