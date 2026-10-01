-- Plan 022 native walkthrough fixture: small comparison schemas. Applied to
-- every disposable database. Invented definitions only; no row data.
CREATE SCHEMA src;
CREATE SCHEMA tgt;
CREATE SCHEMA same_a;
CREATE SCHEMA same_b;
CREATE SCHEMA views_a;
CREATE SCHEMA views_b;
CREATE SCHEMA empty_a;
CREATE SCHEMA empty_b;

-- Equal within scope.
CREATE TABLE src.equal_table (
  id integer PRIMARY KEY,
  name text NOT NULL DEFAULT 'x',
  qty integer CHECK (qty > 0)
);
CREATE TABLE tgt.equal_table (
  id integer PRIMARY KEY,
  name text NOT NULL DEFAULT 'x',
  qty integer CHECK (qty > 0)
);

-- Known changes next to fields that stay not comparable (now() default).
CREATE TABLE src.orders (
  id bigint GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
  amount numeric(10, 2) NOT NULL DEFAULT 0,
  status text DEFAULT 'new',
  note text,
  created timestamptz DEFAULT now(),
  CONSTRAINT amount_positive CHECK (amount >= 0)
);
COMMENT ON COLUMN src.orders.note IS 'source note';
CREATE INDEX orders_status_idx ON src.orders (status);
CREATE TABLE tgt.orders (
  id bigint GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
  amount numeric(12, 2) NOT NULL DEFAULT 1,
  status text DEFAULT 'pending',
  note text NOT NULL,
  created timestamptz DEFAULT now(),
  extra integer,
  CONSTRAINT amount_positive CHECK (amount > 0)
);
COMMENT ON COLUMN tgt.orders.note IS 'target note';
CREATE INDEX orders_status_idx ON tgt.orders (status, amount);

-- Directional absence.
CREATE TABLE src.only_in_source (id integer);
CREATE TABLE tgt.only_in_target (id integer);

-- Excluded counterpart (table against view) and excluded definitions.
CREATE TABLE src.mixed_kind (id integer);
CREATE VIEW tgt.mixed_kind AS SELECT 1 AS id;
CREATE VIEW src.a_view AS SELECT 1 AS x;
CREATE VIEW tgt.a_view AS SELECT 1 AS x;
CREATE TABLE src.parted (id integer, d date) PARTITION BY RANGE (d);
CREATE TABLE tgt.parted (id integer, d date) PARTITION BY RANGE (d);

-- Lock target for the cancel and concurrent DDL scenarios.
CREATE TABLE src.lock_me (id integer, renamed_later integer);
CREATE TABLE tgt.lock_me (id integer, renamed_later integer);

-- Large escaped and multibyte values: a comment at the 256 KiB field limit on
-- the source and a shorter, different one on the target. Markup must render
-- as text.
CREATE TABLE src.big_values (id integer, markup text);
CREATE TABLE tgt.big_values (id integer, markup text);
COMMENT ON COLUMN src.big_values.markup IS
  '<img src=x onerror="window.__dbunkInjected=1"><b>bold</b> &amp; </pre><script>window.__dbunkInjected=2</script>';
COMMENT ON COLUMN tgt.big_values.markup IS
  '<img src=x onerror="window.__dbunkInjected=3"><i>italic</i>';
DO $$
DECLARE
  unit text := '日本語 😀 "quoted" \back/slash <tag> ' || chr(9) || 'tab' || chr(10)
    || chr(1) || ' é ß ' || chr(8232) || '|';
  source_value text;
  target_value text;
BEGIN
  source_value := repeat(unit, 262144 / octet_length(unit));
  source_value := source_value
    || repeat('x', 262144 - octet_length(source_value));
  target_value := repeat(unit, 200000 / octet_length(unit)) || '终';
  EXECUTE format('COMMENT ON TABLE src.big_values IS %L', source_value);
  EXECUTE format('COMMENT ON TABLE tgt.big_values IS %L', target_value);
  -- Pure three- and four-byte sequences: 64 KiB chunk cuts land inside a
  -- code point unless the reader follows the returned byte offsets.
  EXECUTE format('COMMENT ON COLUMN src.big_values.id IS %L',
    repeat('語', 30000) || repeat('😀', 30000));
  EXECUTE format('COMMENT ON COLUMN tgt.big_values.id IS %L',
    repeat('😀', 25000) || repeat('語', 20000));
END
$$;

-- Identical pair.
CREATE TABLE same_a.item (id integer PRIMARY KEY, label text DEFAULT 'a');
CREATE TABLE same_b.item (id integer PRIMARY KEY, label text DEFAULT 'a');

-- Nothing inside the supported projection.
CREATE VIEW views_a.only_view AS SELECT 1 AS x;
CREATE VIEW views_b.only_view AS SELECT 2 AS x;
