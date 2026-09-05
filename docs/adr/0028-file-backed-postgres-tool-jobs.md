# ADR-0028: File-backed PostgreSQL tool jobs

**Status**: Accepted (Plan 018, `PAR-010`)

## Decision

A PostgreSQL Tool Job is an in-memory backup or restore owned by
`postgres::backup::PgToolJobManager`. Plan 018 delivered the backend; Plan 019
activates it through a global Backup / Restore workspace and contextual table
tabs. A table restore opens the database-target review. Native dialogs select
paths only; paths stay in transient form state. There is no event stream,
persistence, scheduler, or generic job framework.

The six typed commands are `start_pg_backup`, `start_pg_restore`,
`get_pg_tool_job`, `list_pg_tool_jobs`, `cancel_pg_tool_job`, and
`release_pg_tool_job`. Start returns admission immediately. Get/list polling is
the sole progress transport. List can filter by connection and recover jobs
when a caller loses their IDs. Snapshots contain file names, never absolute
paths or credentials. Restore has a source size but no trustworthy processed
byte count; clients must not fabricate a percentage. Preflight tool versions
are retained for client/server skew diagnosis.

Admission reserves capacity for pending validation/connection resolution as well
as running jobs: four globally and one per connection, rejecting rather than
queuing excess work. Four separate resource permits stay charged until native
child/reaper cleanup finishes. A reap timeout frees the ordinary job slot, but
exhausted reaper capacity can still refuse admission until a child exits. States
are queued, preflight, running,
finalizing, completed, cancelling, cancelled, and failed. Transitions are
centralized. An atomic per-job claim serializes accepted cancellation against
the start of non-interruptible file publication. When cancellation wins, cleanup
precedes cancelled; after publication claims the job, cancel is a no-op. The job
stays finalizing until publication and completion effects finish. Filesystem I/O
never holds the global manager mutex. Release rejects active work and otherwise is
idempotent. Terminal snapshots expire after one hour; a minute monitor and cap
of 32 terminal records bound history without evicting active work. Credentials
belong only to execution, not retained records.

## Files and native processes

Backup writes to a unique, private, create-new sibling named
`.<name>.dbunk-partial-<job-id>-<random>`. It requires an absolute destination
and existing directory parent. After successful `pg_dump`, the runner verifies
a non-empty regular archive, syncs it, and uses tempfile's non-overwriting
persistence operation. Modern macOS/Linux use atomic no-replace rename;
Windows uses a native non-replacing move. Existing files and symlinks are
refused, including destinations created after validation. Failure/cancellation
removes only this job's partial, never the final path. Local macOS publication
is covered by the execution record.

A hard crash can leave a clearly named sibling partial in an arbitrary user
directory. The final archive is protected from partial publication, but cleanup
is not guaranteed after a hard crash. Never promote or sweep old partials.
The UI explains this limitation. Network or unusual filesystems require
separate validation of atomic rename guarantees.

Native tools run through Tokio directly, without a shell, with kill-on-drop,
null operation stdout and concurrent stderr drainage. Plain restore alone uses
bounded stdin streaming; other operations have null stdin. The retained
tail is limited to 64 KiB even while the child writes more. Returned diagnostics
are normalized from that tail to recognized categories; unknown text is
withheld because PostgreSQL errors can echo passwords, SQL, rows, TLS material,
or paths. Raw arguments and environments are never logged. Version stdout is
bounded separately and must match the tool banner.

Discovery searches PATH, then `/opt/homebrew/bin`, `/usr/local/bin`, `/usr/bin`,
Postgres.app's latest bin directory, and on Linux versioned
`/usr/lib/postgresql/<version>/bin` directories newest first, before a bare-name
fallback. Keg-only clients can be selected through PATH. Preflight uses
`--version` as its sole argument under a ten-second deadline. It proves
executability, not connectivity or compatibility. All three native tools require a supported patched client: 14.24+, 15.19+,
16.15+, 17.11+, 18.6+, or a newer major release. Older clients fail before
connection/archive execution. This common security floor covers restricted-mode
[CVE-2026-18408](https://www.postgresql.org/support/security/CVE-2026-18408/)
and pg_dump's [CVE-2026-19385](https://www.postgresql.org/support/security/CVE-2026-19385/);
it is distinct from future client/server version negotiation. Operations have no
overall deadline. Hydrated credentials, resolved SSH host/port, and the shared libpq
TLS renderer remain authoritative. Database names are quoted as one conninfo
value so libpq cannot treat a name as a connection string redirecting secrets.

Backups explicitly select plain/custom and literal quoted schema/table patterns.
Mixed case, quotes, dots, and wildcard metacharacters stay literal. Plain backup
`clean` embeds `--clean --if-exists` in the archive. Custom backup rejects
`clean`, because it belongs at restore time for that format.

Restore requires an absolute, non-empty regular source. Plain SQL uses
`psql --single-transaction --set=ON_ERROR_STOP=on --no-psqlrc`, enters native
restricted mode with a private random key via `--command`, then reads bounded
stdin with `--file -`. It rejects `clean`. A 512-byte header probe and 8 KiB
stream chunks remove only canonical pg_dump outer restriction guards while
preserving the SQL/COPY body. Old dumps pass through; extra or mismatched
meta-command guards are rejected by psql's private restricted mode. Local shell
commands, program COPY, includes, and connection switching remain blocked.
A source-stream error retains the pipe until kill/reap, preventing premature
EOF from committing a valid prefix. Cancellation kills the child before
aborting the feeder.

Custom archives first pass `pg_restore --list`, then restore with
`--single-transaction` and optionally `--clean --if-exists`. The target is always
the connection's own database. Extensions never determine format. Parallel
restore is excluded because it conflicts with single-transaction correctness.

Without cleanup embedded in plain SQL or requested for custom restore,
restoring over existing objects fails. This is expected. V1 passes no
`--no-owner`, `--no-privileges`, `--create`, or `--role` switches. A different
owner/privilege environment can fail with a missing-role diagnostic. Explicit
typed ownership/privilege options and version negotiation remain follow-ups.

## Safety and teardown

Restore checks `WriteIntent::Restore` against the hydrated connection before
admission, returning typed `policyBlocked` or `policyNeedsConfirmation` errors
with shared statement summaries. A confirmed retry cannot unlock read-only.
The stored authorization disposition decides whether successful completion
records an override, followed by activity. The best-effort completion attempt has
a two-second bound and logs a warning if it times out. Public completed state,
admission release, and releasability are published together after this attempt.
Backup success touches activity without a restore override. Admission, failure,
and cancellation do not record success. Start does not use the legacy return-means-success helper.

All connection/global socket fences include these jobs before pool or tunnel
invalidation. This covers disconnect, delete, credentials, bastions, global
reset, credential configuration/migration, and every full connection save. Even
a cosmetic rename through the full save path cancels an active job. The separate
organization-only metadata update does not invalidate connection resources.
Bastion save, deletion, and host-key reset close global admission before reading
references, so concurrent connection edits cannot introduce an unfenced job.
This conservatively cancels jobs on unrelated connections during Bastion changes. The UI lists jobs before save and warns the user.
Pending resolutions belong to the manager and are cancelled before the fence
invalidates caches. The resolution future is dropped before its reservation
releases, preventing a stale SSH route from being inserted after invalidation.
Generation checks additionally reject stale credentials or policy at job start.
Nested fences retain closing status until all scopes release. Fence signatures remain infallible.

An observed successful native restore exit establishes irreversible success,
including when success is observed while reaping after a cancellation request.
Such a restore can advance from cancelling to finalizing/completed and still runs
its required success audit. Cancellation must not misreport an observed commit
as a rollback. No cancellation result promises to undo an already committed
database transaction.

Cancellation sends `start_kill` and waits at most five seconds for reaping.
If reaping times out, the job becomes cancelled with a `reap` timeout and frees
admission. A detached reaper owns the remaining child and partial cleanup until
exit; tunnel/pool invalidation proceeds after the bounded wait. The whole manager
fence and ordinary cancellation also have a five-second bound covering stalled
work outside reaping. Aborted workers are joined before normal cancelled state;
a cleanup timeout is reported if destruction cannot finish inside the bound.
Publication that already claimed success is not aborted or relabelled cancelled;
it can remain finalizing while the fence proceeds, since its database child has
exited. This preserves the rule that a cancelled job cannot later publish a file.
The bounded reap exception cannot guarantee that an unkillable child exits before
invalidation. App exit
includes `close_all` in the existing three-second join. Normal native children
are killed and cleaned within that budget; forced exit retains the hard-crash
partial limitation above.

Plan 019 owns user activation and warnings; native UI verification remains
open in its execution record. The app uses bounded polling, invalidates pending
restore reviews before lifecycle changes, and refreshes database metadata once
per observed completed restore. Staged edits remain preserved but cannot apply
after their source is invalidated. Plan 020 owns bounded table import/export
and is not implemented by this activation.
