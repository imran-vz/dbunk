# Direct PostgreSQL staged diagnosis, 2026-10-03

Source and owned-fixture verification pass; window acceptance remains pending.
This increment extends the
existing connection form's Test action; Save remains separate. It does not
activate SSH/bastions/managed endpoints or complete Plans 027–030.

The report has six ordered stages: tunnel (skipped for a direct connection),
DNS, TCP, TLS, authentication and database. It includes stage timings, bounded
failure descriptions, observed encryption/protocol/cipher, certificate/hostname
verification facts and transport warnings. Client-certificate presentation is
reported as observed only when the server observation supports it. The existing
staged SCRAM channel-binding limitation remains visible.

Each form owns one cancellation control and freezes inputs during its probe.
Cancel waits for backend completion; form destruction cancels, while Host and
Backend retain worker and socket joins. Input edits discard the report; captured
revision and cancellation checks prevent late results from becoming current.
No connection metadata, password or workspace draft is saved by Test.
Configured session options are applied in the disposable database-stage session,
preserving the previous native Test action's role/search-path validation. A
failure there is a database-stage failure, not a successful handshake result.

The shared preparation seam now applies the baseline destination check before
reading stored credentials: an unsaved host/port/user/TLS change with a blank
password is refused. Cosmetic/database edits may reuse the stored password,
and an explicit test password does not replace it. Ordinary Save semantics are
unchanged. The existing TLS fixture matrix now explicitly supplies its fixture
password when testing edited TLS settings.

Reports are checked at 64 KiB, DNS output at 32 addresses and local PEM input at
1 MiB per regular file. Native PEM loading validates the opened handle and
refuses special files/symlinks, reusing the shared TLS verifier policy. The form
reserves 256 KiB from the shared 128 MiB retained allowance; each pending report
holds 68 KiB from the shared 16 MiB delivery allowance. These are payload
allowances, not process RSS claims. At most 16 pending Host probe workers are
admitted; completed joins are consumed before new admission.

An absolute asynchronous deadline is capped at ten seconds and respects a
shorter configured connect timeout. It covers all asynchronous probe stages,
including database queries. Synchronous filesystem/platform-trust operations
and the platform DNS resolver prevent a hard wall-clock cancellation guarantee;
this increment does not claim to remove those inherited limitations. Socket
protocol tasks are explicitly aborted and joined on all terminal paths.

The credential-boundary tests pass using an encrypted temporary profile and
owned ephemeral loopback peers. An independent read-only integration review
found no concrete issues in credential ordering, cancellation, stale replies or
shared memory accounting. Required/backend/native checks, the real owned TLS
matrix and separate packaging pass; see [frozen evidence](./connection-diagnosis-source-checks/README.md).
Actual-window keyboard/AX and new-control IME checks remain pending because the
current tool connection lacks cua_repl/native desktop control. VoiceOver remains
deferred; existing scoped IME evidence is not a pass for this new report.

Later CUA access returned: [scoped diagnosis and grid keyboard window checks](./diagnosis-window-20261003/README.md) passed with clean teardown. Missing warning AX names were corrected in source; rebuilt-window verification and new real IME composition remain pending.
