#!/bin/sh
set -eu

openssl req -new -x509 -days 1 -nodes -text \
  -subj "/CN=localhost" \
  -addext "subjectAltName=DNS:localhost,IP:127.0.0.1" \
  -keyout "$PGDATA/server.key" -out "$PGDATA/server.crt"
chown postgres:postgres "$PGDATA/server.key" "$PGDATA/server.crt"
chmod 600 "$PGDATA/server.key"
cat >> "$PGDATA/postgresql.conf" <<'EOF'
ssl = on
ssl_cert_file = 'server.crt'
ssl_key_file = 'server.key'
EOF
