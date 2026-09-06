FROM postgres:16

# Disposable proof fixture only. Never install this support function in an app
# connection: its deliberately observable output is what the gate investigates.
RUN apt-get update \
    && apt-get install -y --no-install-recommends gcc libc6-dev postgresql-server-dev-16 \
    && rm -rf /var/lib/apt/lists/*
COPY output_probe.c /tmp/output_probe.c
RUN gcc -Wall -Wextra -Werror -fPIC -shared \
    -isystem "$(pg_config --includedir-server)" \
    -o /usr/lib/postgresql/16/lib/dbunk_output_probe.so /tmp/output_probe.c
