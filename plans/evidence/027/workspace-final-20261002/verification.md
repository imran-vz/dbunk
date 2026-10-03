# Native workspace actual-window verification

The final release executable and exact owned launch are identified in `identity.json`. The profile is isolated at `/private/tmp/dbunk-native-workspace-final-20261002`; no daily-driver data was opened.

- Credential onboarding and connection Test/Save passed with a masked password. The secure editor's kill-ring copy did not expose the fixture password to SQL.
- Two independently connected query documents returned 42 and 2. Switching retained exact Unicode SQL, separate results, editor focus, and undo. All three layouts retained SQL/results.
- Rename, pin/unpin, ordering, and closing one document preserved the other. The draft writer acknowledged Saved.
- The TLS form rejected a wrong hostname, an untrusted CA, and a missing client key with the expected typed failures. Trusted VerifyFull with the owned client certificate/key passed Test and Save. A query of `pg_stat_ssl` confirmed TLS for the saved connection.
- Titlebar close saved drafts and joined cleanup. Both PostgreSQL fixture counts returned from zero to zero (`teardown.json`).
- The same profile reopened with exact acknowledged SQL and disconnected state, without SQL replay. Cmd-Q joined cleanup and returned both fixtures to zero. See `../workspace-final-reopen-20261002/`.

`prepare.log` ends at a foreground guard interruption after successful connection save. The probe sent no keyboard input while another app was foreground. `queries.log` resumes that same owned window and completes the query/document checks; `tls.log` records the complete TLS checks. The probe now reactivates the verified application and checks foreground identity before each keyboard event. There were no unresolved product failures in this run.

The scoped window screenshots were inspected. The connection form, workspace, and results fit without clipped controls. Long certificate paths use the field's horizontal scrolling. Passwords remain masked. Screenshots capture only the verified app PID's window.

Run the probe with `swiftc tools/measure/workspace-accessibility.swift -o /tmp/dbunk-workspace-ax`, then pass the launch `identity.json` and `prepare`, `tls` (plus the owned certificate directory), `close-window`, `reopen`, or `quit` as appropriate. The launcher verifies fixture ownership and records final backend counts.
