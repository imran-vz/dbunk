\set ON_ERROR_STOP on

CREATE SCHEMA lifecycle;
COMMENT ON SCHEMA lifecycle IS 'Fixture objects for PostgreSQL lifecycle workflows.';

CREATE TYPE lifecycle.order_status AS ENUM ('new', 'processing', 'done');
COMMENT ON TYPE lifecycle.order_status IS 'Lifecycle order state.';

CREATE TYPE lifecycle._internal_status AS ENUM ('hidden', 'visible');

CREATE TYPE lifecycle.money_pair AS (
  amount numeric(12, 2),
  currency text
);

CREATE TYPE lifecycle.order_id_range AS RANGE (
  SUBTYPE = integer,
  MULTIRANGE_TYPE_NAME = lifecycle.order_id_multirange
);

CREATE DOMAIN lifecycle.positive_int AS integer
  CHECK (VALUE > 0);

CREATE TABLE lifecycle.orders (
  id serial PRIMARY KEY,
  status lifecycle.order_status NOT NULL DEFAULT 'new',
  amount numeric(12, 2) NOT NULL DEFAULT 0,
  retry_count lifecycle.positive_int,
  created_at timestamptz NOT NULL DEFAULT now()
);
COMMENT ON TABLE lifecycle.orders IS 'Orders used to exercise object lifecycle operations.';

CREATE VIEW lifecycle.orders_view AS
SELECT id, status, amount, created_at
FROM lifecycle.orders;
COMMENT ON VIEW lifecycle.orders_view IS 'A dependency between orders and orders_mat.';

CREATE MATERIALIZED VIEW lifecycle.orders_mat AS
SELECT id, status, amount
FROM lifecycle.orders_view
WITH DATA;

CREATE SEQUENCE lifecycle.order_seq
  AS bigint
  START WITH 100
  INCREMENT BY 5
  MINVALUE 100
  MAXVALUE 1000000
  CACHE 10;
ALTER SEQUENCE lifecycle.order_seq OWNED BY lifecycle.orders.id;
SELECT nextval('lifecycle.order_seq');
COMMENT ON SEQUENCE lifecycle.order_seq IS 'Non-default sequence fixture.';

CREATE FUNCTION lifecycle.add_nums(integer, integer)
RETURNS integer
LANGUAGE sql
IMMUTABLE
AS $$ SELECT $1 + $2 $$;

CREATE FUNCTION lifecycle.add_nums(text, text)
RETURNS text
LANGUAGE sql
IMMUTABLE
AS $$ SELECT $1 || $2 $$;

CREATE PROCEDURE lifecycle.bump_orders()
LANGUAGE sql
AS $$ UPDATE lifecycle.orders SET retry_count = COALESCE(retry_count, 0) + 1 $$;

CREATE FUNCTION lifecycle.square_sum(state numeric, value numeric)
RETURNS numeric
LANGUAGE sql
IMMUTABLE
AS $$ SELECT state + value * value $$;

CREATE AGGREGATE lifecycle.sum_squares(numeric) (
  SFUNC = lifecycle.square_sum,
  STYPE = numeric,
  INITCOND = '0'
);

CREATE EXTENSION IF NOT EXISTS hstore WITH SCHEMA lifecycle;

CREATE EXTENSION IF NOT EXISTS postgres_fdw;
CREATE SERVER lifecycle_fixture_server
  FOREIGN DATA WRAPPER postgres_fdw
  OPTIONS (host '127.0.0.1', dbname 'dbunk_demo');
CREATE FOREIGN TABLE lifecycle.remote_orders (
  id integer OPTIONS (column_name 'id'),
  status text OPTIONS (column_name 'status') COLLATE "C" DEFAULT E'new\\status',
  CONSTRAINT remote_orders_status_present CHECK (status <> '')
) SERVER lifecycle_fixture_server
OPTIONS (schema_name 'lifecycle', table_name 'orders', fetch_size '100');
