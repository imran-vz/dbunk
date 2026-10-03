# Accessibility verification scope, 2026-10-03

Imran explicitly requested: “Let's remove voice over from the verification gate
it's getting too TDS. We'll come back to it later.”

VoiceOver listening and announcement verification is now deferred follow-up
work for Plans 027–030. It does not block migration acceptance. This decision is
not a VoiceOver pass and does not alter historical evidence. Preserve accessible
labels, secure-field semantics and focus behavior while implementing features.

Keyboard/AX checks and real IME composition remain required. Imran separately
instructed the agent to perform the checks because he could not test them.
Agent evidence must name the real input method and actual commit/cancel/undo/
restore behavior; pasted Unicode is not evidence of composition.

Later that day, actual Pinyin – Simplified composition was observed and verified
in SQL, connection Name and table cell controls. The exact scoped pass, binary
identities, undo behavior and restored text are recorded in
[Plan 028 window evidence](../028/table-window-verification-20261003.md#real-ime-scoped-result).
The earlier unsuccessful setup below is historical, not the current scoped result.
Future tool controls and remaining keyboard/AX acceptance still require evidence.

The agent stopped VoiceOver verification after this decision and confirmed
System Settings reported VoiceOver off, its original state. No other VoiceOver
preference was changed. Caption-panel configuration was inspected only.

Before deferral, one initial keyboard-focus anomaly was observed during screen
reader/foreground setup. Controlled new-connection tests with VoiceOver off and
on then kept SQL unchanged and routed Tab/text into Host. This observation is
not a reproducible product failure, a completed VoiceOver test or evidence to
justify a speculative source change. Any subsequent reproducible keyboard/AX
failure remains in scope.

Deferred follow-up: execute the retained VoiceOver checklist in
[human-accessibility.md](./human-accessibility.md), including form/dialog reading
order, errors, secure password speech and focus return, on the then-current
native build. No human listening result has been recorded for these controls.

Temporary IME setup was cleaned up after the attempted check: input sources
again contain only ABC, Show Input menu is off, and dictation languages contain
only English (United States). Real composition remains unverified. This does
not block implementation work or reintroduce the deferred VoiceOver gate.

## Later unlocked-desktop retry during administration checks

System Settings AX discovery and screenshots worked. A temporary built-in
Pinyin – Simplified source was added; both previous/next-source shortcuts were
observed enabled. Raising System Settings, focusing its own search field,
Control-Space and individual n/i/h/a/o keys produced plain `nihao`, without
marked text or a candidate window. This observation is outside GPUI and does not
establish an app IME failure. Direct CUA binding to the running Apple
TextInputMenuAgent and SystemUIServer timed out. No new composition pass is
claimed; the earlier scoped SQL/form/cell pass remains valid for its named builds.

Cleanup was verified through System Settings: Pinyin removed, ABC only, Show
Input menu off, dictation English (United States) only. macOS had automatically
added Mandarin dictation; it was removed. The cached download-description text
remained despite the selected language returning to English-only. VoiceOver was
not enabled. The [administration package window checks](../029/admin-control-window-20261003/README.md)
subsequently passed scoped keyboard review scrolling and activation; broader
keyboard/AX and tool-input IME gates remain open.


## Maintenance-package preparation retry

A further serialized CUA retry again found System Settings accessible. Temporary
Pinyin – Simplified was added. Raising the window, explicitly focusing the search
field and using Control-Space plus individual keys produced plain `nihao`, with
no observed marked text or candidate window. A coordinate activation retry did
not establish composition either. This is inconclusive outside GPUI; no new IME
pass or app failure is claimed. Cleanup was verified: ABC only, Show Input menu
off, dictation English (United States) only. VoiceOver was not enabled. The
existing scoped real-composition evidence remains unchanged.


Column-pinning preparation retry: System Settings AX controls and screenshots were
readable. Adding temporary Pinyin succeeded, but binding TextInputMenuAgent timed
out; trying to focus the Settings title through a screenshot coordinate failed
with `noWindowsAvailable`. No composition evidence was captured. The temporary
source was removed via AX, input menu switched off, and dictation restored to
English (United States) only. ABC-only and these restored values were observed in
AX. This retry is unverified, not an IME failure or pass. VoiceOver stayed deferred.
Restoration evidence is under `028/pinning-source-checks/restored-*-ax.txt`.


## Diagnosis and table-copy retries

The unlocked desktop supplied native AX trees and screenshots again. During
diagnosis and table-copy preparation, temporary Pinyin – Simplified plus
Control-Space and individual physical n/i/h/a/o events still produced plain
`nihao` in System Settings, with no observed composition or candidate window.
This does not establish a GPUI failure or a new IME pass. New Tool tab IME remains
open; previous named SQL/form/cell evidence is unchanged. The available app
inventory did not expose an input-menu surface. ABC-only sources, hidden input
menu and English (United States)-only dictation were restored through CUA;
restoration snapshots are under
[table-copy window evidence](../029/table-copy-source-checks/window-initial/ui/).
VoiceOver stayed off and deferred.


## XLSX preparation retry

System Settings AX and screenshots were available. The agent explicitly focused
its search field through AX value assignment, then tried Control-Space and
Control-Option-Space with individual n/i keys after temporarily adding built-in
Pinyin – Simplified. Both produced plain `ni`; no marked text or candidate window
was observed. This inconclusive OS precheck does not establish a GPUI failure or
new Tool-tab IME pass. ABC-only sources, the hidden input menu and English
(United States)-only dictation were restored through CUA. Evidence is under
[the XLSX window preparation directory](../029/xlsx-import-source-checks/window/ui/).
VoiceOver remained off and deferred.

## Seed preparation retry

The [seed precheck](../029/seed-source-checks/ime/README.md) again obtained Settings
AX and screenshots, but not marked text or candidates after physical letter keys.
Both source-switch bindings were enabled; input-menu agent binding timed out, so
explicit source selection could not be established. ABC-only sources, hidden input
menu and English (United States)-only dictation were restored. This is inconclusive
outside GPUI and does not change the earlier scoped pass. Tool-tab IME remains
pending; further retries need a different input-selection mechanism or a change
in the automation surface. VoiceOver remains deferred.

The later [Structure precheck](../029/table-structure-source-checks/ime/README.md)
tried the distinct Caps Lock input-source switch. System Settings again showed
plain Latin text without composition evidence. Temporary preferences were
restored. This does not alter the earlier scoped pass or close Tool-tab IME.

## Whole-table export and keyboard correction

Native windows are available on the unlocked desktop. The
[export checks](../029/whole-table-export-source-checks/window/README.md) record
corrected target/status AX, click-to-keyboard focus, exact local recipe reopen,
and complete/cancelled parent status. Individual Enter/Space exposed duplicated
button activation in native handlers; the
[broader correction](../029/button-activation-source-checks/README.md) verifies
single-step format/operator changes, preserved Administration arrows, and stable
Read-only/Favorite focus through repeated keys and Escape. Each result is scoped
to its recorded package. This does not pass every keyboard/AX control.

The export preparation's separate Caps Lock source-switch precheck remains
inconclusive outside GPUI; [its restored-settings evidence](../029/whole-table-export-source-checks/ime/README.md)
records ABC only, hidden input menu and English (United States)-only dictation.
Real Tool-tab IME remains pending. VoiceOver stayed off and deferred; its later
checklist is preserved.

## Existing-table DDL launch

The corrected [table DDL package](../029/table-ddl-activation-source-checks/README.md)
launched in the owned profile, but CUA subsequently failed at native pipe startup
before obtaining any app inventory or window. Resetting the automation runtime
and retrying produced the same error. This does not establish that the desktop
is locked, nor a GPUI keyboard or IME failure. No new DDL window/keyboard/AX/IME
pass is claimed. The launcher-owned process was stopped by verified PID and UID;
its SIGTERM exit is not normal-quit acceptance. No input-source or VoiceOver
settings were changed. Earlier scoped evidence remains intact.
