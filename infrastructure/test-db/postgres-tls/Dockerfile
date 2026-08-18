FROM postgres:16-alpine
RUN apk add --no-cache openssl
COPY configure-tls.sh /docker-entrypoint-initdb.d/00-configure-tls.sh
