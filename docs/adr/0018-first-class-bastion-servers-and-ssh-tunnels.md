# ADR-0018 — First-class Bastion Servers and SSH Tunnels

**Status**: Accepted (2026-06-02)

dbunk will model SSH access as two separate concepts: a reusable **Bastion Server** and a per-Connection **SSH Tunnel** that references one bastion. The Connection's `host` and `port` remain the database endpoint as seen from the bastion; the tunnel layer chooses the local forwarding endpoint unless the Connection stores explicit `localBindHost` / `localPort` options.

SSH tunneling applies to every network-backed engine: PostgreSQL, MySQL, ClickHouse, and Redis. SQLite is excluded because it has no network transport. When a tunnel is enabled, all operations for that Connection must route through it, including health checks, Test Connection, Redis Pub/Sub, scan sessions, and ClickHouse HTTP requests. dbunk must fail immediately on tunnel setup failure and must not silently fall back to direct database connectivity.

Bastion Servers are managed as first-class Settings records. The Connection form may create one inline, but the saved result is still a normal Bastion Server. Deleting a Bastion Server is blocked while any Connection references it; editing one invalidates the shared SSH session and active forwards, and referencing Connections must reconnect.

Each Bastion Server owns exactly one active SSH authentication method: password, private key file path, or private key content. Key passphrases, password auth secrets, and stored private key contents use the existing credential-storage backends under a separate bastion-secret namespace, not the database-password namespace. Host-key verification uses trust on first use: dbunk stores the first accepted host-key fingerprint and rejects later mismatches until the user explicitly resets trust.

At runtime, dbunk shares one SSH session per Bastion Server and creates one local forward per active Connection. Forwards are reference-counted and are cleaned up on disconnect, delete, save, or bastion edit. This avoids repeated SSH handshakes while keeping database endpoint routing isolated per Connection.

## First Implementation Slice

The first slice should ship the persisted model and working tunnel runtime for all network-backed engines, with minimal UI: Settings > Bastion Servers CRUD, Connection Advanced tunnel selector/options, inline bastion creation from the Connection form, Test Bastion, and Test Connection through the tunnel.

## Deferred

- Bastion search/filter and richer management polish.
- Import-key wizard or guided private-key capture flow.
- Host-key reset UX beyond a simple explicit reset action.
- Advanced SSH options such as compression, SSH keepalive tuning, jump chains, and proxy commands.
- Broader Settings redesign beyond the Bastion Servers section needed for this feature.

## Considered Options

- **PG-only tunnel inside `PgDriverOptions`**: rejected because the user need is transport routing for databases, not a PostgreSQL driver option.
- **Per-operation SSH tunnels**: rejected because query/schema/Redis fan-out would repeatedly pay SSH handshake cost and increase failure rate under load.
- **Silent direct fallback when SSH fails**: rejected because it can leak traffic outside the configured route and makes failures misleading.
- **One SSH session per Connection**: rejected because multiple Connections can reuse the same bastion without repeating authentication.
