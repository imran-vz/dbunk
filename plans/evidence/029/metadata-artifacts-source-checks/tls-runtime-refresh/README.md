# Owned TLS runtime refresh

The ownership guard refused further checks when Homebrew changed the OpenSSL alias from 3.6.4 to 3.6.5. The old PostgreSQL runtime linked through that alias. No guard was bypassed for database access.

The one-time `refresh.py` checked the recorded PostgreSQL binaries, prior OpenSSL inputs, fixture marker, certificates/configuration, exact PID/command and postmaster identity before stopping only stage04 PID 35510. It preserved the old runtime at `tools/native/.state/tls-runtime-retired-3.6.4-20261003`, built the same pinned PostgreSQL source against the current OpenSSL, and restarted the existing data directory. It did not initialize a database, replace certificates, install packages or alter global trust.

`before.json`, `after.json` and `refresh.log` record the operation. UUID 15151cfc-5885-4066-831f-2717ed9b4587, port 15433 and database dbunk_tls_demo are unchanged. The SQL sentinel, seven-case owned libpq TLS matrix and zero other backends passed. Native TLS/window acceptance remains a separate scope. This script is a historical, exact-instance receipt, not a general restart command.
