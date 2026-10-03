# Scoped pinning AX and copy check, 2026-10-03

Package SHA256 `0ecfd79cf7db96da14130b7361a3f43b6d7473d3ee9ee3b673fd49e5d4819736`,
PID 89554. Owned profile `/private/tmp/dbunk-native-auto-fit-20261003-review`,
ID `c16f87dc-e21a-4ea2-b2ed-a0fdc9aa8a42`. Launched through workspace_launch.py
with both fixture ownership checks; connected only stage03 at
127.0.0.1:15432/dbunk_demo, UUID 2283820d-33ec-4c4c-ae03-7051092bd410.

A temporary Query 2 ran a 250-row, 25-column generate_series SELECT with two
headings named x. Pinning the second x retained selected value 1001 and moved
that exact source ahead of source zero. Pinning c2 then navigating by Right to
c24 retained the pinned sources in AX alongside the last scrolling columns.
Shift-Space copied the selected row. Pasting into the temporary SQL editor and
reading its AX value verified all 25 exact TSV values in source order
[1,2,0,3,...,24]; that paste was undone and never executed. F6 did not switch focus
in this attempt, so the paste check used an explicit editor click. No F6 pass is
claimed. Pinning c24 overflowed the frozen pane; Left twice returned selection to
1001. AX exposed the overflow guidance. Query 2 was then closed, retaining the
original eight documents.

On the existing owned native_parity_20261003.rows table, amount pinned ahead of
value only after Table preferences saved appeared. The selected exact numeric
value remained 0.10000000000000000000. Opening Edit cell addressed amount;
the edit was cancelled without staging. The full editor check is archived again
in the [reopen run](../pinning-reopen-20261003/README.md).

Normal Cmd-Q exited 0. Both stage03 and TLS activity returned 0 → 0. Query delivery
high-water was 261,343 bytes and workspace queue high-water 1,705,472, each remaining
0. These are payload queue measurements, not RSS.

Limits: wheel injection failed with noWindowsAvailable. Screenshots disagreed
with the current AX tree, became blank, or lost unchanged regions. A reversible
window zoom yielded one current frame with a header/row offset during resize;
the next input aligned the scrolling labels but other captured regions vanished.
Visual alignment and resize acceptance remain unresolved, not passed. The
[preceding package](../pinning-capture-comparison-20261003/README.md) also lost
content after input. Keyboard/AX evidence here is narrower than visual or full
accessibility acceptance. No new IME pass; VoiceOver deferred.
