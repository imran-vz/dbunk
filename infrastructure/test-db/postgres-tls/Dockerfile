FROM postgres:16-alpine
# Non-executable so the entrypoint *sources* it, which exposes
# docker_process_init_files for the shared SQL fixture mounted at
# /fixture-sql (the shared init dir cannot be mounted here as well).
COPY --chmod=644 configure-tls.sh /docker-entrypoint-initdb.d/00-configure-tls.sh
