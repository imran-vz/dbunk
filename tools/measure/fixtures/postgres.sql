-- Plan 024 grid fixtures. Disposable PostgreSQL fixture only.
-- Sizes stay inside the Query Session retention limits: 10,000 rows per
-- Result Set and 32 MiB per execution.
CREATE SCHEMA IF NOT EXISTS plan024;

-- Wide rows: 10,000 rows by 100 short columns.
CREATE OR REPLACE VIEW plan024.fixture_wide AS
SELECT g + 0 AS c000, g + 1 AS c001, g + 2 AS c002, g + 3 AS c003, g + 4 AS c004, g + 5 AS c005, g + 6 AS c006, g + 7 AS c007, g + 8 AS c008, g + 9 AS c009, g + 10 AS c010, g + 11 AS c011, g + 12 AS c012, g + 13 AS c013, g + 14 AS c014, g + 15 AS c015, g + 16 AS c016, g + 17 AS c017, g + 18 AS c018, g + 19 AS c019, g + 20 AS c020, g + 21 AS c021, g + 22 AS c022, g + 23 AS c023, g + 24 AS c024, g + 25 AS c025, g + 26 AS c026, g + 27 AS c027, g + 28 AS c028, g + 29 AS c029, g + 30 AS c030, g + 31 AS c031, g + 32 AS c032, g + 33 AS c033, g + 34 AS c034, g + 35 AS c035, g + 36 AS c036, g + 37 AS c037, g + 38 AS c038, g + 39 AS c039, g + 40 AS c040, g + 41 AS c041, g + 42 AS c042, g + 43 AS c043, g + 44 AS c044, g + 45 AS c045, g + 46 AS c046, g + 47 AS c047, g + 48 AS c048, g + 49 AS c049, g + 50 AS c050, g + 51 AS c051, g + 52 AS c052, g + 53 AS c053, g + 54 AS c054, g + 55 AS c055, g + 56 AS c056, g + 57 AS c057, g + 58 AS c058, g + 59 AS c059, g + 60 AS c060, g + 61 AS c061, g + 62 AS c062, g + 63 AS c063, g + 64 AS c064, g + 65 AS c065, g + 66 AS c066, g + 67 AS c067, g + 68 AS c068, g + 69 AS c069, g + 70 AS c070, g + 71 AS c071, g + 72 AS c072, g + 73 AS c073, g + 74 AS c074, g + 75 AS c075, g + 76 AS c076, g + 77 AS c077, g + 78 AS c078, g + 79 AS c079, g + 80 AS c080, g + 81 AS c081, g + 82 AS c082, g + 83 AS c083, g + 84 AS c084, g + 85 AS c085, g + 86 AS c086, g + 87 AS c087, g + 88 AS c088, g + 89 AS c089, g + 90 AS c090, g + 91 AS c091, g + 92 AS c092, g + 93 AS c093, g + 94 AS c094, g + 95 AS c095, g + 96 AS c096, g + 97 AS c097, g + 98 AS c098, g + 99 AS c099
FROM generate_series(1, 10000) AS g;

-- Large cells: 400 rows by 8 columns of 8,192 characters.
CREATE OR REPLACE VIEW plan024.fixture_large AS
SELECT repeat(lpad((g * 31 + 0)::text, 16, 'x'), 512) AS t0, repeat(lpad((g * 31 + 1)::text, 16, 'x'), 512) AS t1, repeat(lpad((g * 31 + 2)::text, 16, 'x'), 512) AS t2, repeat(lpad((g * 31 + 3)::text, 16, 'x'), 512) AS t3, repeat(lpad((g * 31 + 4)::text, 16, 'x'), 512) AS t4, repeat(lpad((g * 31 + 5)::text, 16, 'x'), 512) AS t5, repeat(lpad((g * 31 + 6)::text, 16, 'x'), 512) AS t6, repeat(lpad((g * 31 + 7)::text, 16, 'x'), 512) AS t7
FROM generate_series(1, 400) AS g;

-- Many rows: 10,000 rows by 8 mixed columns.
CREATE OR REPLACE VIEW plan024.fixture_many AS
SELECT g AS id,
       g * 2 AS doubled,
       'row-' || g AS label,
       lpad(g::text, 10, '0') AS padded,
       g % 97 AS bucket,
       'value ' || (g * 7) AS note,
       '2026-10-02'::text AS day,
       'lorem ipsum dolor sit amet ' || g AS body
FROM generate_series(1, 10000) AS g;
