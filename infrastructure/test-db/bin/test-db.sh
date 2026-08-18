#!/usr/bin/env sh
set -eu

cd "$(dirname "$0")/.."

case "${1:-}" in
  postgres)
    exec make postgres
    ;;
  postgres-tls)
    exec make postgres-tls
    ;;
  clickhouse)
    exec make clickhouse
    ;;
  redis)
    exec make redis
    ;;
  postgres-redis)
    exec make postgres-redis
    ;;
  all)
    exec make all
    ;;
  down)
    exec make down
    ;;
  ps)
    exec make ps
    ;;
  logs)
    exec make logs
    ;;
  *)
    cat >&2 <<'USAGE'
Usage: infrastructure/test-db/bin/test-db.sh <command>

Commands:
  postgres        Start only PostgreSQL
  postgres-tls    Start only TLS PostgreSQL
  clickhouse      Start only ClickHouse
  redis           Start only Redis
  postgres-redis  Start PostgreSQL and Redis together
  all             Start PostgreSQL, ClickHouse, and Redis
  down            Stop and remove containers/anonymous volumes
  ps              Show fixture containers
  logs            Follow fixture logs
USAGE
    exit 64
    ;;
esac
