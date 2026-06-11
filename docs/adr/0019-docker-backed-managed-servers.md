# ADR-0019 — Docker-backed Managed Servers

**Status**: Accepted (2026-06-11)

dbunk will provision local development databases as **Managed Servers**: Docker containers that dbunk creates and orchestrates but that the Docker daemon supervises. v1 supports Docker only (no Podman, no embedded binaries) and the PostgreSQL and MySQL engines, using official images with user-selected major-version tags. dbunk owns orchestration only; it never supervises a database process itself.

A Managed Server is a separate top-level entity from Connection, following the Bastion Server precedent: provisioning creates one Managed Server plus one Connection pointing at it. The data directory lives on a named volume owned by the Managed Server, so the container and the data have independent lifetimes — an externally removed container is recoverable (Recreate reattaches the volume), and only dbunk's explicit Destroy action removes both container and volume.

Lifecycle posture: containers are left running on app quit (dbunk is a client of the server, not its parent process), use Docker restart policy `no` (nothing dbunk-made auto-starts at boot), and auto-start on connect with a visible "Starting" wait. Managed Server status (`Running` / `Starting` / `Stopped` / `Orphaned`) is always derived live from Docker at observation time, never from dbunk's stored record, so external `docker stop` / `docker rm` surfaces as honest state instead of confusing errors.

Provisioning defaults: dbunk picks a free non-default port (5433+/3307+ base, user-overridable), generates credentials stored through the existing Credential Backend, and creates one empty database per server. No init-SQL hook — schema setup belongs to the user's migration tool.

## Considered Options

- **Embedded/managed server binaries (DBngin model)**: rejected — dbunk would own per-OS binary distribution, data-dir management, upgrades, and crash supervision, contradicting the reliability-first priority. Docker delegates process supervision to a runtime built for it.
- **Logical-only provisioning (`CREATE DATABASE` on an existing server)**: rejected as the core feature because the target scenario is "no server exists yet"; may return later as a cheap addition.
- **Stop containers on app quit**: rejected — other tools (dev servers, test runners) may be using the database; dbunk created the server but is still just a client of it.
- **Restart policy `unless-stopped`**: rejected — forgotten databases silently consuming resources after every reboot violates predictability; container state should change only on user action or connect intent.
- **Connection-with-container-metadata instead of a separate entity**: rejected — it overloads Connection's status vocabulary with container states and makes "delete connection" ambiguous between forgetting an endpoint and destroying data.
