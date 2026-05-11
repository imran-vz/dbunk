#!/usr/bin/env sh
set -eu

redis-server \
  --loadmodule /opt/redis-stack/lib/rediscompat.so \
  --loadmodule /opt/redis-stack/lib/redisearch.so \
  --loadmodule /opt/redis-stack/lib/redistimeseries.so \
  --loadmodule /opt/redis-stack/lib/rejson.so \
  --dir /data \
  --save "" \
  --appendonly no &

pid="$!"

until redis-cli ping >/dev/null 2>&1; do
  sleep 0.1
done

redis-cli --pipe < /seed.redis

wait "$pid"
