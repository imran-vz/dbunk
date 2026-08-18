# Test Database Fixtures

Ephemeral Docker fixtures for exercising Dbunk against PostgreSQL, ClickHouse,
and Redis/Redis Stack. Each service starts with a clean in-memory data directory
and reseeds from its own fixture files on container creation.

## Run

```sh
pnpm run db:postgres
pnpm run db:postgres-tls
pnpm run db:clickhouse
pnpm run db:redis
pnpm run db:all
```

Or run from the infrastructure folder:

```sh
make -C infrastructure/test-db postgres
make -C infrastructure/test-db postgres-tls
make -C infrastructure/test-db clickhouse
make -C infrastructure/test-db redis
make -C infrastructure/test-db postgres-redis
make -C infrastructure/test-db all
```

Stop and remove containers with:

```sh
pnpm run db:down
make -C infrastructure/test-db down
```

The service profiles can also be combined directly:

```sh
docker compose -f infrastructure/test-db/compose.yml --profile postgres --profile redis up --force-recreate --renew-anon-volumes postgres redis
```

## Connection Details

| Engine | Host | Port | Database | User | Password |
| --- | --- | ---: | --- | --- | --- |
| PostgreSQL | `localhost` | `15432` | `dbunk_demo` | `dbunk` | `dbunk` |
| PostgreSQL TLS | `localhost` | `15433` | `dbunk_demo` | `dbunk` | `dbunk` |
| ClickHouse HTTP | `localhost` | `18123` | `dbunk_demo` | `dbunk` | `dbunk` |
| ClickHouse native | `localhost` | `19000` | `dbunk_demo` | `dbunk` | `dbunk` |
| Redis | `localhost` | `16379` | `0`, `1`, `2` | | |

## Fixture Coverage

PostgreSQL covers schemas, primary keys, foreign keys, composite keys, enums,
domains, arrays, JSONB, generated columns, identity columns, check constraints,
nullable fields, views, materialized views, partitions, indexes, comments, and
edge-case data.

ClickHouse covers MergeTree-family engines, partitions, ordering keys,
low-cardinality and enum columns, decimals, arrays, maps, nullable values,
materialized views, aggregate rollups, logical relationships, and intentionally
awkward text/JSON payloads.

Redis covers strings, counters, binary-ish values, hashes, lists, sets, sorted
sets, streams, expirations, multiple logical databases, and RedisJSON keys when
using the Redis Stack image.
