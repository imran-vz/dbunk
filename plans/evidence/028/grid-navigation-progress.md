# Retained-grid navigation, 2026-10-03

Go to row (Cmd-G) and Select current row (Shift-Space) are implemented in
source. Navigation preserves the displayed column and clamps positive whole
numbers to the captured retained result or table page. It does not fetch rows.
Opening the bounded editor first admits 1 MiB against the shared 128 MiB
payload allowance; closing or replacing the result releases that lease.
Keyboard and AX actions share the same validation. Escape cancels, Tab cycles
the field and buttons, and marked composition is left to the editor.

Select current row creates a rectangle across visible columns in display
order. It is not independent or noncontiguous checkbox selection. That baseline
workflow remains open, along with grid pinning and broader data parity.

[Required/native checks and separate packaging](./grid-navigation-source-checks/README.md)
pass (257 native tests, 13 ignored). [Initial window checks](./grid-navigation-window-20261003/README.md)
pass scoped navigation and projected copy, with normal quit and fixture activity
0 → 0. They found missing AX range/error labels; the source correction awaits a
rebuilt-window check. VoiceOver remains deferred; real IME and
remaining keyboard/AX checks retain their existing scope. Plans 027–030 remain
IN PROGRESS.


The [maintenance package window run](../029/maintenance-reopen-20261003/README.md)
verified the corrected range and invalid-value AX nodes. Its exact binary and
limits are recorded there; broader grid/keyboard/IME acceptance remains open.
