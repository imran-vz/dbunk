# Stage 03 owned PostgreSQL fixture

Recorded 2026-10-02T08:04:25Z. Automated fixture setup and ownership checks;
this file is not native UI or accessibility completion evidence.

## Target and isolation

The user authorized creating an isolated fixture after the Docker daemon
blocker. The installed Docker CLI points to an unavailable OrbStack socket.
No Docker daemon, existing container, system service or Homebrew database was
started. No production database, daily-driver profile or Keychain was used.

- Host: macOS 27.0.1, build 26A434, Apple Silicon.
- Python: 3.14.7.
- PostgreSQL: 17.11, built from the official source archive.
- Endpoint: `127.0.0.1:15432/dbunk_demo`.
- Fixture project: `dbunk-native-stage03`.
- Instance: `2283820d-33ec-4c4c-ae03-7051092bd410`.
- Server PID at setup: `3519`.
- Runtime: `tools/native/.state/runtime/installed/bin/postgres`.
- Data: `tools/native/.state/local/2283820d-33ec-4c4c-ae03-7051092bd410/data`.
- Cluster directory is private and UUID-marked. TCP binds only loopback;
  Unix sockets are disabled to avoid macOS's socket-path length limit.
- Credentials are the disposable fixture-only `dbunk` credentials.

`fixture.py check` verifies the state/cluster markers, exact private executable,
PID, postmaster data directory, port, listen address and SQL instance sentinel
before admitting the fixture. Up/down operations share an exclusive lock.
Cleanup rejects changed process identity and removes only the marked cluster.
It does not issue the repository's broad `db:down` command.

The Docker path remains available and preferred when its daemon is already
running: a labelled project, loopback port binding and tmpfs PostgreSQL data.
The local fallback uses disposable files under the marked task directory;
it does not claim tmpfs behavior.

## Reproduction and source identity

```sh
just native-fixture-up
python3 tools/native/fixture.py check
python3 tools/native/fixture.py count
# After all native/core/AX test clients have exited:
just native-fixture-down
```

The fallback downloads
[PostgreSQL 17.11 source](https://ftp.postgresql.org/pub/source/v17.11/postgresql-17.11.tar.bz2)
and verifies SHA-256 before extraction. It configures a private install prefix,
with optional ICU/readline/zlib disabled, then runs `make -j4` and
`make install`. Build output stays in
`tools/native/.state/runtime/source-build.log`. It invokes no package install
or service manager. The runtime is reusable after the cluster is removed.

| Artifact | SHA-256 |
| --- | --- |
| Official source archive | `dd27f2b3c59e73ed14aa3324901242bf69a032a6347805f274e6260322d42979` |
| Built PostgreSQL executable | `a0b34047f3547b667f580797227054924affe6a73f3d35b3cb95c7b00394f405` |
| `tools/native/runtime.py` | `3180eb13349a947a8a8cb7779049a52dcb733fd0db4ab2a3197a0417c26518ce` |
| `tools/native/local_fixture.py` | `3479e455d6f9e423581e96b675f04e112587f4d3d0003845a13bb3dab5a54642` |
| `tools/native/fixture.py` | `61d611e21241ff4da305a7d5d8a4a4bac2df951d4f4bf4ec8529d57c7a80e3ad` |

An initial checksum-verified Homebrew bottle trial remained entirely under
`.state/runtime`. Its embedded system share paths prevented isolated cluster
startup. It was replaced by the private source build, with no global symlinks
or Homebrew installation changes. The unused bottle is not the running server.

## Observed results

`just native-fixture-up` and `fixture.py check` passed. The fixture loaded the
existing stage01 views plus stage03 identity, exact-value and policy fixtures.

| Observation | Result |
| --- | --- |
| `plan024.fixture_wide` count | 10,000 |
| `plan024.fixture_large` count | 400 |
| `plan024.fixture_many` count | 10,000 |
| Null text | SQL NULL |
| Empty text | Empty string |
| Bigint text | `9223372036854775807` |
| Decimal text | `1234567890.12345678901234567890` |
| Unicode text | `é😀é` |
| Quoted text | `quoted 'value'; still one string` |
| Backend count before application tests | 0, excluding the counting connection |
| Fixture/profile ownership and fault-identity unit tests | 10 passed |

The core actor/safety run subsequently passed 17 + 2 live tests and returned
activity to baseline; see [core-live.txt](./core-live.txt). The fixture is
being retained for subsequent native and AX runs. An active fixture here is
intentional, not an assertion that native shutdown tests have passed.

The native AX fault harness now has a narrow `terminate-query` helper. It
requires the launch fixture UUID and the exact query backend PID/start time,
revalidates the owned server and guards database/user identity in SQL. It
rejects stale or mismatched identities. This external fixture operation does
not add a service-policy override to the native host.
