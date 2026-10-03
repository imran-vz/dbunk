# Persisted table pin reopen, 2026-10-03

Same frozen pinning package and owned profile as the
[first run](../pinning-window-20261003/README.md), PID 93498. Workspace restored
all eight original documents disconnected. After an explicit stage03 connection,
AX showed 60 rows, two visible columns, one pin, headers amount then value.
The exact selected numeric source opened Value for amount with
0.10000000000000000000. Cancel edit staged nothing.

Unpinning acknowledged Table preferences saved, restored headers value then
amount, and preserved the selected amount source. Normal Cmd-Q exited 0, both
owned fixtures 0 → 0, workspace queue high-water 1,705,472 with remaining 0.
The final read-only profile check, after the comparison run, found workspace v9,
eight documents, 2,198 bytes, hidden id, order [value,id,amount], widths 193/179 and
empty pins. The temporary query is absent; no data mutation was issued.

This verifies scoped AX, exact source editing and persistence, not visual wheel
scrolling, all pin/visibility combinations, new-tool IME or complete PostgreSQL
parity. VoiceOver remains deferred.
