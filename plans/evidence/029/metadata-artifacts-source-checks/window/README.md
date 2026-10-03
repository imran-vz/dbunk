# Initial metadata-artifact window checks

Frozen package: `/private/tmp/dbunk-native-package-20261003-metadata-artifacts/dbunk Native Preflight.app`.
Executable SHA256: `829e534a1a06d5a1741a341ce174b71ce5f07ed8d53935c06cde23c360e5e923`.
PID 83856 was launched through `workspace_launch.py` with the existing owned
profile `/private/tmp/dbunk-native-auto-fit-20261003-review`, then quit normally
with exit 0. The original 13 documents remain; one map tab was added and saved
in workspace format 12. Both fixtures returned to zero other backends.

The owned stage03 objects in `setup.json` were retained for the corrected-package
recheck, then removed with guarded RESTRICT cleanup; see `../correction/teardown.json`. They are four marked tables in two marked schemas, with three FKs,
including a composite pair, junction and outgoing external target.

Observed behavior:

- Disconnected restore; creating the map tab requires explicit connection binding,
  Connect and Refresh. Database scope captures 7 tables / 3 FKs.
- Schema scope captures 4 / 3, including the fully described external target.
  Focal child-table scope captures 3 / 2, excluding neighbor-only edges.
- Composite detail preserves both ordered OID/attnum pairs. Return enters
  details; Cmd-C reports exact displayed-page copy. Escape/Home/Down selects
  the junction table and updates exact details. This is scoped keyboard evidence.
- Changing schema routing to Step receives an exact-save acknowledgement.
  Focal-table scope retains its separate default Curve setting. SQLite after
  normal exit contains the exact schema-scoped Step preference. Reopen is pending.
- Native SVG save publishes 6,446 database-viewport bytes. Native PNG save
  publishes a valid 24-bit-plus-alpha 2400×640 image (36,429 bytes) for the
  focal-table viewport. These are different captures, not byte-equivalent exports.
- Schema DDL captures three relations and discloses reconstruction omissions.
  The saved complete file matches all 1,930 AX preview bytes exactly. No DDL
  was executed. A physical typing attempt leaves the read-only preview unchanged.
  A nonexistent setup schema visibly refuses the read, retaining the original
  preview with historical-capture and differing-controls notices.

Actual-window failures remain open in this frozen package:

- Wide opaque FK-label backgrounds cover node content.
- Refresh/Fit changes the camera during prepaint, so paths and node elements
  can use different camera revisions; subsequent screenshots can lose content.
- Composite captions can overlap node fills in the default layout/export.

Native corrections and keyboard node movement were verified in a separate
[frozen package](../correction/README.md); the failures above remain historical
results for this initial binary. The initial screenshot named
`map-label-occlusion.png` captured the later missing-content state;
`schema-map.png` clearly shows camera mismatch and caption occlusion.
No full visual, keyboard/AX, Tool-tab IME or parity pass is claimed. VoiceOver
remains deferred. See `../ime/README.md` for the restored temporary setup.
