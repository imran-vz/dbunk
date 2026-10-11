# Docker-backed managed servers

**Status:** Accepted (2026-06-11, rewritten 2026-10-11)

Local PostgreSQL servers the app creates are a separate Managed Server
entity, run in Docker with a named volume and restart policy `no`
(`backend/src/managed.rs`, `backend/src/docker.rs`). dbunk is a client of the
server, not its parent: containers keep running when the app quits, start
again only on user action or connect intent, and their status is always read
live from Docker rather than from the stored record, so an external
`docker stop` or `docker rm` shows up as honest state. Provisioning picks a
free non-default port, stores generated credentials through the credential
backend (ADR-0003) and creates one empty database; schema setup belongs to
the user's migration tool.

## Considered options

- **Embedded server binaries (the DBngin model).** Rejected: dbunk would own
  per-OS binaries, data directories, upgrades and crash supervision. Docker
  already supervises processes.
- **Stopping containers on quit.** Rejected: other tools may be using the
  database.
- **Restart policy `unless-stopped`.** Rejected: forgotten databases would
  start after every reboot.
- **Container metadata on a Connection instead of a separate entity.**
  Rejected: it mixes container states into connection status and makes
  "delete connection" ambiguous between forgetting an endpoint and destroying
  data.
