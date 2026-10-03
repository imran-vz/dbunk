# Tool tabs window attempts, 2026-10-03

Status: **no Tool tabs/copy/IME acceptance pass**. The frozen tools package passed
[source checks](./tool-tabs-source-checks/README.md), but CUA could not discover a
window. This does not establish an application defect.

Package: `/private/tmp/dbunk-native-package-20261003-tools/dbunk Native Preflight.app`.
Executable SHA256: `6abd0abebf535d980d8c8f6340d9edba123d6e8cba56525340d800f91f5a28ca`.
Profile: `/private/tmp/dbunk-native-tables-20261003-review`.
Both attempts used guarded `tools/native/workspace_launch.py`; stage03 fixture
UUID `2283820d-33ec-4c4c-ae03-7051092bd410`, endpoint
`127.0.0.1:15432/dbunk_demo`. No daily-driver or production resource was opened.

- First attempt: [identity](./tools-window-20261003/identity.json), PID 37381.
  CUA app discovery listed the process, but `getApp` returned `cgWindowNotFound`
  and the prior app handle's AX read timed out. An owned process sample showed
  AppKit's main event loop idle; its short stack excerpt is `startup-sample.txt`.
- Controlled retry: [identity](./tools-window-retry-20261003/identity.json),
  PID 38888. The same native discovery error occurred. Attempting to open
  System Settings for input-method verification produced the same error;
  inventory listed both apps running. No settings were changed, and no IME
  composition was attempted in this variant.
- Each native process was terminated with SIGTERM only after its PID command
  matched the recorded executable and profile. Each fixture check verified
  ownership and returned PostgreSQL activity to zero. Launcher exit `-15`
  is failure cleanup, **not** a normal quit pass. Details are in
  `failed-startup.json` and `failed-discovery.json` in the respective directories.

The agent asked whether the Mac desktop is unlocked/available and continues
independent implementation. Pending scenarios: history success/error/counts and
cancellation exclusion; connection/search/outcome filters; save/edit/favorite/
delete and exact draft opening; clipboard formats/projection/refusal; keyboard/AX;
real Pinyin composition in Tool controls; normal close/reopen and cleanup.
Earlier [SQL/form/cell Pinyin evidence](../028/table-window-verification-20261003.md)
remains scoped to its named packages. VoiceOver stays deferred, not passed.
