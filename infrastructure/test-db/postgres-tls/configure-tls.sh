#!/bin/sh
# Sourced by the postgres entrypoint during initdb (ADR-0025 fixture).
# Installs the host-generated material (see gen-certs.sh, mounted at
# /certs), requires a client certificate for the `dbunk_cert` role, then
# runs the shared SQL fixture from /fixture-sql. The password rule for
# `dbunk` stays, so every existing TLS test keeps working.
set -eu

for file in server.crt server.key ca.crt; do
  cp "/certs/$file" "$PGDATA/$file"
done
chown postgres:postgres "$PGDATA/server.key" "$PGDATA/server.crt" "$PGDATA/ca.crt"
chmod 600 "$PGDATA/server.key"

cat >> "$PGDATA/postgresql.conf" <<'CONF'
ssl = on
ssl_cert_file = 'server.crt'
ssl_key_file = 'server.key'
ssl_ca_file = 'ca.crt'
CONF

{
  echo "hostssl all dbunk_cert 0.0.0.0/0 cert clientcert=verify-full"
  echo "hostssl all dbunk_cert ::/0 cert clientcert=verify-full"
  cat "$PGDATA/pg_hba.conf"
} > "$PGDATA/pg_hba.conf.new"
mv "$PGDATA/pg_hba.conf.new" "$PGDATA/pg_hba.conf"
chown postgres:postgres "$PGDATA/pg_hba.conf"

docker_process_init_files /fixture-sql/*
