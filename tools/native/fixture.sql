-- Loaded only through the identity-checked stage03 fixture container.
CREATE SCHEMA plan026;
CREATE TABLE plan026.fixture_identity (instance uuid PRIMARY KEY);
CREATE VIEW plan026.exact_values AS
SELECT NULL::text AS null_value, ''::text AS empty_value,
       9223372036854775807::bigint AS large_integer,
       1234567890.12345678901234567890::numeric AS precise_decimal,
       'é😀é'::text AS unicode_value,
       'quoted ''value''; still one string'::text AS quoted_value;
CREATE TABLE plan026.policy_probe (id integer PRIMARY KEY, value text NOT NULL);
INSERT INTO plan026.policy_probe VALUES (1, 'unchanged');
