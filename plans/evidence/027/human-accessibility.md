# Plan 027 accessibility and IME checks

Current status: **VoiceOver deferred, non-blocking; scoped real IME passed**.
Agent-driven Pinyin composition in SQL, connection Name and table cell editor,
including commit/cancel/selection/undo and restored acknowledged SQL/cell draft,
passed on the named table/column packages on 2026-10-03. See
[the exact actions and limits](../028/table-window-verification-20261003.md#real-ime-scoped-result).
This is not complete keyboard/AX acceptance or verification of future tools. On
2026-10-03 Imran asked the agent to perform the checks, then explicitly removed
VoiceOver from the verification gate. See [the scope decision](./accessibility-scope-20261003.md).
No VoiceOver pass or human sign-off is claimed. Automated keyboard/AX evidence
and agent-driven real composition are recorded separately.

Use only the isolated native window and profile identified in the handoff. It
contains disposable fixture credentials and drafts. No daily-driver app or
database is part of this check. Foreground automation must be serialized. The earlier human handoff ended when
Imran asked the agent to run the checks on 2026-10-03.

## VoiceOver (deferred follow-up)

1. Navigate the connection Navigator, query tabs, credential settings and New
   connection form. Check names, selected/disabled states and reading order.
2. In the connection form, traverse text fields, password, TLS options and the
   certificate picker. The password must remain a secure field with no spoken
   plaintext. Cancel the picker and form; focus should return predictably.
3. Submit an empty connection name or invalid port. Hear the validation failure
   and find the affected field without losing the form. Correct it or cancel.
4. Open Rename tab and the credential-reset confirmation. Check their titles,
   actions and Escape/Cancel focus return. Cancel reset to preserve this profile.
5. Change SQL, hear the draft-save state, switch tabs and return. Check that the
   active document and query controls remain identifiable. Existing query error
   announcements may also be rechecked; they do not substitute for new controls.

## Real input method

1. Use a real composition-based input method already available on this Mac.
   Compose text in a connection name, commit part, cancel another composition,
   move the caret and undo. Do not save a connection with a different endpoint.
2. Compose text inside a SQL comment. Commit and cancel compositions, select
   and replace text, then switch tabs and return. Composition must not trigger
   Run, corrupt text or lose the caret unexpectedly.
3. Wait for Saved, quit and reopen the same isolated profile. Confirm the exact
   committed SQL comment is restored and the tab stays disconnected.

Report the input method/language and either a pass or the control/action that
failed. Record the actual checked build/profile, tester and observed result
below; do not infer composition from pasted Unicode or willingness to test.

## Earlier attempt (superseded by the scoped result above)

VoiceOver: deferred by Imran, not passed. IME: pending. The agent temporarily added built-in Pinyin – Simplified, but
did not obtain observed composition/candidate evidence through the available
UI controls. No pass is claimed. The temporary input source was removed; ABC,
the hidden input menu and English (United States) dictation were restored.

## Current handoff

The unlocked-window check passed on the frozen package with executable SHA256
`df2cb9a3141cd5c9ae83c38d28333e40fc843e56bbcd36c4f4bb8a1506803a02`.
The window is `dbunk Native Workspace`, using
`/private/tmp/dbunk-native-workspace-final-20261002`.
The agent stopped foreground automation after leaving SQL focused and result
`42` visible. This is automated handoff evidence, not a human pass.

For the final IME restore check, wait for Saved and quit normally. After the
existing launcher exits, reopen the same frozen package without rebuilding:

```sh
python3 tools/native/workspace_launch.py /private/tmp/dbunk-native-workspace-final-20261002 --bundle '/private/tmp/dbunk-native-package-20261002/dbunk Native Preflight.app' --tls --out /Users/imran/projects/Code/dbunk/plans/evidence/027/workspace-package-20261002/human-reopen-20261002
```

The output directory must be new. The restored tab must be disconnected and
contain the exact acknowledged SQL. The profile includes both owned fixture
manifests, so retain `--tls` when reopening it.

Imran subsequently said “continue”; no VoiceOver/IME outcome or input method
was reported at that time. The later scope decision above supersedes the
VoiceOver gate and delegates the remaining real IME check to the agent.
