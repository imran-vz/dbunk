BEGIN;
CREATE SCHEMA native_metadata_20261003;
COMMENT ON SCHEMA native_metadata_20261003 IS 'Owned native metadata verification 2026-10-03';
CREATE TABLE native_metadata_20261003.parent ("first part" text UNIQUE, "second.part" bigint, PRIMARY KEY ("second.part","first part"));
CREATE TABLE native_metadata_20261003.child (id integer PRIMARY KEY,"local first" text,"local second" bigint,CONSTRAINT composite_fk FOREIGN KEY("local second","local first") REFERENCES native_metadata_20261003.parent("second.part","first part") ON UPDATE CASCADE ON DELETE SET NULL,CONSTRAINT single_fk FOREIGN KEY("local first") REFERENCES native_metadata_20261003.parent("first part"));
INSERT INTO native_metadata_20261003.parent VALUES ($v$O'Reilly\x雪$v$,9223372036854775807);
INSERT INTO native_metadata_20261003.child VALUES (1,$v$O'Reilly\x雪$v$,9223372036854775807),(2,NULL,NULL);
CREATE TABLE native_metadata_20261003.generated_table(id bigint GENERATED ALWAYS AS IDENTITY PRIMARY KEY,base integer DEFAULT 2,doubled integer GENERATED ALWAYS AS (base*2) STORED,CONSTRAINT positive CHECK(base>0));
CREATE INDEX generated_base_idx ON native_metadata_20261003.generated_table(base);
CREATE TABLE native_metadata_20261003.partitioned(id integer NOT NULL, value text) PARTITION BY RANGE(id);
CREATE TABLE native_metadata_20261003.partition_leaf PARTITION OF native_metadata_20261003.partitioned FOR VALUES FROM (0) TO (100);
CREATE TYPE native_metadata_20261003.mood AS ENUM ('one','雪','quote''value');
CREATE TYPE native_metadata_20261003.pair AS (amount bigint,label text);
CREATE TYPE native_metadata_20261003.custom_range AS RANGE (subtype=integer,subtype_diff=pg_catalog.int4range_subdiff,multirange_type_name=custom_multirange);
CREATE DOMAIN native_metadata_20261003.positive_amount AS numeric(12,2) DEFAULT 1.25 NOT NULL CONSTRAINT positive_domain CHECK(VALUE>0);
CREATE FOREIGN DATA WRAPPER native_metadata_20261003_wrapper NO HANDLER NO VALIDATOR;
COMMENT ON FOREIGN DATA WRAPPER native_metadata_20261003_wrapper IS 'Owned native metadata verification 2026-10-03';
CREATE SERVER native_metadata_20261003_server FOREIGN DATA WRAPPER native_metadata_20261003_wrapper;
COMMENT ON SERVER native_metadata_20261003_server IS 'Owned native metadata verification 2026-10-03';
CREATE FOREIGN TABLE native_metadata_20261003.foreign_metadata(id integer OPTIONS(column_name 'remote=id'),label text COLLATE "C" DEFAULT 'x' NOT NULL,CONSTRAINT present CHECK(label<>'')) SERVER native_metadata_20261003_server OPTIONS(schema_name 'remote schema',table_name 'remote=table');
COMMIT;

-- PostgreSQL resolves this explicit unqualified multirange name via search_path.
ALTER TYPE public.custom_multirange SET SCHEMA native_metadata_20261003;
