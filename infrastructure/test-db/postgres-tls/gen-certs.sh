#!/bin/sh
# Generate the TLS fixture material for the `postgres-tls` service
# (ADR-0025 / Plan 011): a throwaway CA, a CA-signed server certificate for
# localhost / 127.0.0.1, a client certificate whose CN is the `dbunk_cert`
# role, and a passphrase-protected copy of the client key for the
# refusal test. Output lands in ./certs (gitignored). Idempotent.
set -eu

cd "$(dirname "$0")"
mkdir -p certs
cd certs

complete=true
for artifact in \
  ca.crt ca.key \
  server.crt server.key \
  client.crt client.key client-encrypted.key
do
  if [ ! -f "$artifact" ]; then
    complete=false
  fi
done

if [ "$complete" = true ]; then
  echo "fixture certs already present in $(pwd)"
  exit 0
fi

rm -f ./*.crt ./*.key ./*.csr ./*.srl

openssl req -x509 -newkey rsa:2048 -nodes -days 3650 \
  -subj "/CN=dbunk test CA" \
  -addext "basicConstraints=critical,CA:TRUE" \
  -keyout ca.key -out ca.crt >/dev/null 2>&1

openssl req -new -newkey rsa:2048 -nodes \
  -subj "/CN=localhost" \
  -keyout server.key -out server.csr >/dev/null 2>&1
printf 'subjectAltName=DNS:localhost,IP:127.0.0.1\nextendedKeyUsage=serverAuth\n' > server.ext
openssl x509 -req -in server.csr -CA ca.crt -CAkey ca.key -CAcreateserial \
  -days 3650 -extfile server.ext -out server.crt >/dev/null 2>&1

openssl req -new -newkey rsa:2048 -nodes \
  -subj "/CN=dbunk_cert" \
  -keyout client.key -out client.csr >/dev/null 2>&1
printf 'extendedKeyUsage=clientAuth\n' > client.ext
openssl x509 -req -in client.csr -CA ca.crt -CAkey ca.key -CAcreateserial \
  -days 3650 -extfile client.ext -out client.crt >/dev/null 2>&1

openssl pkcs8 -topk8 -in client.key -out client-encrypted.key -passout pass:dbunk >/dev/null 2>&1

rm -f ./*.csr ./*.ext ./*.srl
chmod 600 ./*.key
echo "generated fixture certs in $(pwd)"
