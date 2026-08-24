\set ON_ERROR_STOP on

-- Client-certificate role for the postgres-tls fixture (ADR-0025). On the
-- plain fixture the role simply exists and cannot log in over TLS.
CREATE ROLE dbunk_cert LOGIN;
GRANT CONNECT ON DATABASE dbunk_demo TO dbunk_cert;
