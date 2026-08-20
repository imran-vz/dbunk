\set ON_ERROR_STOP on

CREATE SCHEMA result_mutation_fixture;

CREATE TABLE result_mutation_fixture.generated_identity (
  id bigint GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
  body text NOT NULL,
  note text DEFAULT 'fixture-default',
  body_length integer GENERATED ALWAYS AS (length(body)) STORED
);
INSERT INTO result_mutation_fixture.generated_identity (body)
VALUES ('seed');

CREATE TABLE result_mutation_fixture.keyless_dupes (
  claimed_key text NOT NULL,
  body text NOT NULL
);
INSERT INTO result_mutation_fixture.keyless_dupes (claimed_key, body) VALUES
  ('duplicate', 'first'),
  ('duplicate', 'second'),
  ('single', 'third');

CREATE TABLE result_mutation_fixture.non_unique_candidate (
  looks_like_key text NOT NULL,
  body text NOT NULL
);
INSERT INTO result_mutation_fixture.non_unique_candidate (looks_like_key, body) VALUES
  ('not-unique', 'same'),
  ('not-unique', 'same');
