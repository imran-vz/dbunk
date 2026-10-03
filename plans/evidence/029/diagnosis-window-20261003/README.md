# Scoped diagnosis and grid keyboard window checks, 2026-10-03

The ownership-checking launcher ran the frozen diagnosis package with executable
SHA256 `80f3b9962a261cf6d936bb83ffc95e7fb6e39383e05aa614b924a741578e6b2b`
and profile `/private/tmp/dbunk-native-auto-fit-20261003-review`.
[Identity](./identity.json) names the owned stage03 and stage04-TLS fixtures.
Native CUA was available again. This corrects the earlier tool-availability limit;
it does not imply the desktop was locked.

Passed scoped checks:

- Editing the saved stage03 connection and Test connection displayed all six
  stages, direct/TLS-disabled skips and PostgreSQL 17.11. Name editing removed
  the prior report. An unsaved missing default role failed at Database after
  successful authentication. The form was cancelled without saving edits.
- A temporary query generated 250 rows and three columns. Home/End selected
  first/last columns; Command-End selected `row-250`; Command-Home selected
  `1`. Page Down moved to row 7 for this viewport, Page Up returned to row 1.
  Shift-End then copy/paste into the disposable SQL editor produced exactly
  `7\t70\trow-7`. Escape kept the active cell but copy refused without a
  selected rectangle. These commands changed no database rows.
- Command-W closed a document after an earlier AX Close action. Later reopen
  showed seven original documents: the Objects Tool tab was also closed. The
  table-copy window run restored Objects and its original connection binding.
  No original SQL or table draft was discarded.
  Normal Command-Q exited with code 0 and both fixture client counts 0 → 0,
  as recorded in [teardown](./teardown.json).

Findings and limits:

- Visible unencrypted and SCRAM-channel-binding warnings were missing from AX.
  Their accessible roles/names are now source-corrected; this frozen package
  does not contain that fix. Rebuilt table-copy package AX verification subsequently passed for both warnings;
  see [the captured tree](../table-copy-source-checks/window-initial/ui/diagnosis-warnings.txt).
- AX Results clicks in the export review and a later shell Close click did not
  change state; focused-field Escape and Command-W worked. Source inspection
  found existing handlers using the same actions, so no speculative fix was made.
  These AX activation observations need a controlled rerun.
- Screenshot capture worked initially but later intermittently returned
  ScreenCaptureKit errors or `noWindowsAvailable`. Later AX snapshots confirmed
  completed keyboard actions. Visual/scroll acceptance is not complete.
- Temporary Pinyin – Simplified plus Control-Space and individual n/i/h/a/o
  keys produced plain `nihao`, with no observed marked text/candidate window.
  TextInputMenuAgent binding timed out. Real IME remains pending for these
  controls, neither passed nor failed. ABC-only sources, hidden input menu and
  English (United States)-only dictation were restored and observed in AX.
  VoiceOver was not enabled and remains deferred.

Files under ui contain captured AX snapshots/diffs and the initial screenshot.
No full parity, keyboard/AX completion or new real-composition pass is claimed.
