-- Plan 022 native walkthrough fixture: cap-sized pages. Applied to the primary
-- database only.
CREATE SCHEMA many_src;
CREATE SCHEMA many_tgt;
CREATE SCHEMA over_src;
CREATE SCHEMA wide_src;
CREATE SCHEMA wide_tgt;

-- 1,000 tables per side (the per-endpoint table cap): 980 shared names, 20
-- directional on each side, every seventh shared table changed. Ten pages.
DO $$
DECLARE
  i integer;
  -- 61 bytes with the suffix: inside the 63-byte identifier limit.
  long_name text := repeat('表', 17) || '_long_';
BEGIN
  FOR i IN 1..980 LOOP
    EXECUTE format('CREATE TABLE many_src.%I (v integer)',
      CASE WHEN i % 50 = 0 THEN long_name || lpad(i::text, 4, '0')
           ELSE 't_' || lpad(i::text, 4, '0') END);
    EXECUTE format('CREATE TABLE many_tgt.%I (v %s)',
      CASE WHEN i % 50 = 0 THEN long_name || lpad(i::text, 4, '0')
           ELSE 't_' || lpad(i::text, 4, '0') END,
      CASE WHEN i % 7 = 0 THEN 'bigint' ELSE 'integer' END);
  END LOOP;
  FOR i IN 1..20 LOOP
    EXECUTE format('CREATE TABLE many_src.%I (v integer)', 'src_only_' || i);
    EXECUTE format('CREATE TABLE many_tgt.%I (v integer)', 'tgt_only_' || i);
  END LOOP;
END
$$;

-- One table past the cap: no complete result may be produced.
DO $$
DECLARE
  i integer;
BEGIN
  FOR i IN 1..1001 LOOP
    EXECUTE format('CREATE TABLE over_src.%I (v integer)', 't_' || i);
  END LOOP;
END
$$;

-- One table with 400 columns per side, so its fields span many pages.
DO $$
DECLARE
  source_columns text;
  target_columns text;
BEGIN
  SELECT string_agg(format('%I integer DEFAULT %s', 'c_' || lpad(i::text, 3, '0'), i), ', '),
         string_agg(format('%I %s DEFAULT %s', 'c_' || lpad(i::text, 3, '0'),
           CASE WHEN i % 25 = 0 THEN 'bigint' ELSE 'integer' END,
           CASE WHEN i % 40 = 0 THEN i + 1 ELSE i END), ', ')
    INTO source_columns, target_columns
    FROM generate_series(1, 400) AS i;
  EXECUTE 'CREATE TABLE wide_src.wide (' || source_columns || ')';
  EXECUTE 'CREATE TABLE wide_tgt.wide (' || target_columns || ')';
END
$$;
