CREATE SCHEMA source;
CREATE SCHEMA target;
CREATE SCHEMA external;
CREATE ROLE catalog_reader;
GRANT USAGE ON SCHEMA source, target, external TO catalog_reader;

CREATE FUNCTION external.identity_int(integer) RETURNS integer
LANGUAGE sql IMMUTABLE AS 'SELECT $1';
CREATE SEQUENCE external.serial;
CREATE TYPE external.mood AS ENUM ('calm');

CREATE TABLE source.orders (
    id integer DEFAULT 7,
    quantity integer,
    label text DEFAULT 'source.literal ''quoted''',
    serial bigint DEFAULT nextval('external.serial'::regclass),
    custom integer DEFAULT external.identity_int(7),
    mood external.mood DEFAULT 'calm',
    CONSTRAINT positive CHECK (quantity > 0)
);
CREATE INDEX orders_expression ON source.orders ((quantity + 1))
    INCLUDE (label) WHERE quantity > 0;
CREATE TABLE target.orders (LIKE source.orders INCLUDING ALL);
GRANT SELECT ON source.orders, target.orders TO catalog_reader;

-- The array is one Const node. pg_depend does not walk its regclass elements.
-- Both the column and expression types are built in; there is no user type to
-- exclude as a shortcut for proving the renderer safe.
CREATE TABLE source.hidden_dependency (
    value text DEFAULT ('{external.serial}'::regclass[])::text
);

CREATE TYPE external.probe;
CREATE FUNCTION external.probe_in(cstring) RETURNS external.probe
AS '$libdir/dbunk_output_probe', 'dbunk_probe_in' LANGUAGE C IMMUTABLE STRICT;
CREATE FUNCTION external.probe_out(external.probe) RETURNS cstring
AS '$libdir/dbunk_output_probe', 'dbunk_probe_out' LANGUAGE C IMMUTABLE STRICT;
CREATE TYPE external.probe (
    INPUT = external.probe_in, OUTPUT = external.probe_out,
    INTERNALLENGTH = 4, PASSEDBYVALUE, ALIGNMENT = int4
);
CREATE TABLE source.output_probe (value external.probe DEFAULT '7');
