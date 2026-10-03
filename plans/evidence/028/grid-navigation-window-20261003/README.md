# Retained-grid navigation window check, 2026-10-03

Serialized CUA run of executable
`086e274f2428ec0696e5f264f334f670f3a570ed251ecdc973956822b2c4a73e`
from `/private/tmp/dbunk-native-package-20261003-grid-navigation/dbunk Native Preflight.app`.
All 450 recorded source hashes matched before launch. Owned profile:
`/private/tmp/dbunk-native-auto-fit-20261003-review`, ID
`c16f87dc-e21a-4ea2-b2ed-a0fdc9aa8a42`. Launcher PID 61785. Only stage03
`127.0.0.1:15432/dbunk_demo`, UUID `2283820d-33ec-4c4c-ae03-7051092bd410`,
was used for reads. No PostgreSQL writes were performed.

- A new temporary Query 2 ran a SELECT over generate_series(1,250), with id and
  24 text columns. Keyboard navigation moved to c24. Cmd-G opened the bounded
  field. Zero stayed in the dialog with a visible error; Tab visibly focused Go.
- Jumping to row 200 preserved c24 (`col24-200`) and revealed it. Entering 24
  nines, Tab and Return clamped to `col24-250`, without overflow or a server read.
- Entering 2 then Escape preserved row 250; Up reached row 249. Shift-Space and
  Cmd-C reported one selected row. A temporary paste into the owned query draft
  verified all 25 tab-separated values, from 249 through col24-249. Undo restored
  the original query. No pasted clipboard content was executed.
- The existing table loaded 60 rows with persisted visible order value/amount
  and hidden id. Cmd-G 59, Shift-Space and copy produced exactly
  `你好\t5.9000000000000000`. The hidden id was absent. This was observed through
  a temporary paste into Query 2 and then undone.
- Temporary Query 2 was closed; the original eight documents remain. The app
  then quit normally (exit 0). Both stage03 and TLS fixture activity returned
  0 → 0. Workspace queue high-water was 8,389,120 bytes, remaining 0; this is
  delivery payload accounting, not RSS or an aggregate retained-payload peak.

## Failure found and remaining acceptance

The range and error text were visible but absent from the AX tree. Source now
adds explicit Label/Status nodes and a polite live error region. That correction
is **not in this frozen binary** and requires rebuilt-window verification. The
initial source checks and package remain frozen; subsequent maintenance work
also changes source. Do not treat this run as complete AX acceptance.

Some AX activations were processed on the next keyboard event. Coordinate
attempts and one keyboard batch reported an automation `user changed` interruption;
state was re-read before further actions. The owned temporary draft was restored
and closed. No speculative foreground/focus source fix was made.

Independent/noncontiguous checkbox selection, grid pinning, broader keyboard/AX
and real IME checks remain open. No new IME or VoiceOver verification was done;
VoiceOver remains deferred. This is scoped behavior evidence, not full parity.


The range/error AX correction was subsequently verified in the separate
[maintenance package run](../../029/maintenance-reopen-20261003/README.md).
This original binary and its evidence remain unchanged.
