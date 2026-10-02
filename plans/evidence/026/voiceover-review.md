# Stage 03 VoiceOver review: needs correction

2026-10-02. Human review by Imran, on the isolated release window PID 77723,
profile UUID `5b98f37f-e52a-4be0-8ca9-075da76fe503`. This is a new stage 03
gate; stage 01's editor geometry/focus gate remains closed.

## Human result

Imran could not understand what Control-Alt-T or Tab was supposed to do.
He described the error as subtle, appearing like a warning at the bottom left
rather than a clear error. This is not a pass. Earlier automated AX assertions
establish exposed text/roles/focus, not intelligible spoken feedback.

## Findings

- The app binds `ctrl-alt-t` and `ctrl-alt-l`. Control-Option is a default
  VoiceOver modifier, so app shortcuts using it conflict with VoiceOver's
  command namespace. See [Apple's keyboard guide](https://support.apple.com/en-ca/guide/voiceover/vo2681/mac).
  The user's modifier configuration and exact intercepted keystroke were not
  measured. The missing visible explanation is independently apparent.
- The query footer uses Role::Status and sets its value, but supplies no live
  region setting. Pinned AccessKit macOS 0.26.3 queues spoken announcements
  only for live values; `Live::Assertive` maps to high-priority announcements.
  A status label alone is not proof of a spoken error.
- Errors share the same small footer with routine status. They need an explicit
  error surface and distinct semantics from warnings/notices.

## Correction: error placement A selected

[Static review](../../mocks/native-postgres/accessibility-review.html), served at
http://localhost:18726/accessibility-review.html.

- A: error at the result boundary.
- B: dedicated error result tab.
- C: persistent error below query controls, originally recommended.

Imran selected **A** and added an editor hover over the problematic SQL segment.
The placement decision is settled. [Compact and detailed hover mocks](../../mocks/native-postgres/error-hover-review.html)
are served at http://localhost:18726/error-hover-review.html. Imran selected
**compact** and authorized implementation. Both visual decisions are settled.

- All: keep the three selected layouts; replace Control-Option app shortcuts;
  propose F8 to the first enabled query control, a visible focus outline and
  explicit Tab/Shift-Tab/Enter/Escape guidance. Tab in SQL remains indentation.
- Name the failure, show PostgreSQL code/message/position, retain partial
  results, preserve SQL selection/undo, and announce a terminal failure once
  through an explicit assertive live node without stealing focus. Recheck
  actual VoiceOver after implementation; HTML semantics are not native proof.

The initial review stopped at static mocks as required by AGENTS.md. After
selection, native corrections were implemented. The gate still requires
verification and a repeat human listening check.

## Artifact verification

The T3 collaborative browser loaded the self-contained artifact. Its A/B/C
anchor links navigate correctly; targets land 18px below the viewport top.
Measured page width and scroll width match at 1402px. A same-origin 390px
review iframe has a 388px content viewport with matching scroll width and no
overflow in any section, toolbar, error surface or native-window example.
Both viewports have true black backgrounds. There are no scripts, animations
or failed resources. Static app controls are labelled as examples.

The collaborative browser snapshot tool failed repeatedly, including after
reopening the tab; DOM/layout inspection succeeded. No rendered screenshot
was obtained, so this follow-up does not claim screenshot-based visual review.


## Hover implementation findings

The pinned Zed revision `506beb34de3f433707b7ebe8d8ad2d80f856af6c`
exposes buffer diagnostics, but its hover path requires a semantics provider
and diagnostic renderer. The standalone host currently supplies neither.
Use a local adapter without starting a language server or expanding backend
service extraction. Preserve the stage 01 accessible editor adapter and
recheck its geometry, focus and text editing behavior.

Current SQL selection returns only a string. Retain the exact executed source
range and buffer snapshot for diagnostic mapping, including a selected range,
a caret-selected statement and scripts. Do not locate the query by searching
for its text, which could mark a duplicate statement. PostgreSQL's one-based
character position must map to editor byte offsets and safe text boundaries.
The server supplies a position rather than a complete token range. Use the
containing token only when it can be identified safely; otherwise use the
reported character or end-of-input location. Missing, invalid or stale
positions keep the persistent error without an invented underline.

Focused verification must cover Unicode, duplicate statements, selections,
later script statements, end-of-input, missing positions, edits while running,
new executions and reconnect. Clear stale diagnostics on edits/new runs and
reconnect. Verify pointer hover, keyboard hover, Escape, scroll/resize and
VoiceOver reachability. Announce terminal failure once without focus theft.
The pinned macOS editor binds Cmd-K Cmd-I to Hover and F8 to GoToDiagnostic;
the proposed F8 toolbar action must override the Editor context as well as
Workbench. Preserve Tab indentation in SQL.

## Hover artifact verification

The collaborative browser loaded both static hover variants and the small
interactive HTML example. At 1402px the page has no horizontal overflow. A
390px iframe has a 388px content viewport with no page or example overflow;
both hover variants remain inside their editor surfaces. Backgrounds are
true black. Scripted keyboard focus reveals the demo tooltip; Escape through
the browser input tool dismisses it and removes its description without
moving token focus. This is HTML behavior, not native accessibility evidence.
The browser screenshot tool remains unavailable after retries, so no
screenshot-based visual review is claimed. No native source was edited during
that design-only follow-up; selected implementation is recorded below.


## Selected correction implementation

Option A now keeps a red-bordered Query failed surface above results, with
message, PostgreSQL code, mapped source location and Return to SQL. It remains
separate from Notices and retains partial results. F8 reaches the first enabled
query control in both Editor and Workbench contexts; visible guidance explains
Tab, Enter, Escape and F6. Control-Option app shortcuts are removed.

The compact pinned Zed hover displays plain text with the database message and
PostgreSQL code. Its local semantics adapter starts no language server. Error
background is true black. The pinned editor supplies its usual rounded tooltip
border; no editor fork or new dependency version is introduced. Full error
text remains accessible above results because the pinned Markdown hover lacks
a distinct AX node. Cmd-K then Cmd-I opens the native hover at the caret.

Source mapping uses the core lexer's exact byte range plus an execution
snapshot. Edits invalidate the source even when followed by undo. New runs,
reconnect, connection loss and close clear diagnostics and pending hover work.
The persistent execution error survives edits without an obsolete location.
Terminal UI updates only for an admitted ExecutionCompleted, so retained
results cannot restore an old failure on a new session's state event.

Each failure has a fresh live-region identity and Live::Assertive value.
This handles identical fast failures even if no intermediate cleared frame is
painted. Re-rendering rows or editing SQL does not change that identity or
message. This implementation evidence is separate from the human listening
result recorded below.


## Automated correction result

The corrected release workflow passes. Native screenshots verify compact
keyboard and pointer hover plus Escape dismissal. The full AX probe verifies
F8/Tab navigation, named error details, application-alert semantics, fresh
announcement identity for fast identical errors, source location mapping,
in-flight edit invalidation and reconnect without resurrecting a prior error.
It also preserves the stage 01 geometry and focus assertions. See
[the correction evidence](./error-correction/review.md). A separate fixture
window with two result tabs, an error and compact hover was prepared for Imran.


## Human correction recheck: PASS, 2026-10-02

Imran confirmed **“Clear now”** in response to the review request covering
F8 (Fn-F8 if needed), Tab through controls and both result tabs, Escape back to
SQL, and Cmd-Shift-Enter to rerun the script and hear “Query failed”. This
closes the human listening gate for the corrected controls and error announcement.
The original negative review above remains part of the evidence.

The reviewed release window was PID 63138, isolated profile
`ae4cb4f5-a812-47db-b055-79ef240ca404`, fixture instance
`2283820d-33ec-4c4c-ae03-7051092bd410`; see
[identity](./error-correction/voiceover/identity.json). After confirmation, the
window was closed through its native close control. The owning launcher recorded
exit 0 and PostgreSQL connections 0 -> 0, then removed its marked temporary
profile; see [teardown](./error-correction/voiceover/teardown.json). The isolated
PostgreSQL fixture remains available. Plan 026 still has outstanding performance
and combined window-race evidence and no completion SHA.
