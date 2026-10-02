#!/usr/bin/env python3
"""Writes the Plan 024 fixtures. Both hosts are measured on the same data:
the Tauri app reads it from PostgreSQL through `postgres.sql`, the native
spike generates the identical cells itself (see `fixture.rs`)."""
import pathlib

here = pathlib.Path(__file__).parent

# The first line is a comment so the typing run never opens a completion
# menu. The three statements below it select the grid fixtures.
lines = [
    "-- typing lands on this comment line ",
    "SELECT * FROM plan024.fixture_wide;",
    "SELECT * FROM plan024.fixture_large;",
    "SELECT * FROM plan024.fixture_many;",
    "",
]
tables = ["orders", "customers", "order_lines", "products", "shipments", "invoices", "payments"]
index = 0
while len(lines) < 2000:
    table = tables[index % len(tables)]
    other = tables[(index + 3) % len(tables)]
    lines += [
        f"-- report {index:04d}: {table} joined to {other}",
        f"SELECT t.id, t.created_at, o.name, sum(t.amount * {index % 17 + 1}) AS total_{index:04d}",
        f"FROM {table} AS t",
        f"JOIN {other} AS o ON o.id = t.{other}_id",
        f"WHERE t.created_at >= now() - interval '{index % 90 + 1} days'",
        f"  AND o.status IN ('open', 'held', 'state_{index % 11}')",
        "GROUP BY t.id, t.created_at, o.name",
        f"HAVING count(*) > {index % 5}",
        "ORDER BY total DESC",
        f"LIMIT {100 + index % 400};",
        "",
    ]
    index += 1
(here / "editor-2000.sql").write_text("\n".join(lines[:2000]) + "\n")

wide = ", ".join(f"g + {k} AS c{k:03d}" for k in range(100))
large = ", ".join(f"repeat(lpad((g * 31 + {k})::text, 16, 'x'), 512) AS t{k}" for k in range(8))
(here / "postgres.sql").write_text(f"""-- Plan 024 grid fixtures. Disposable PostgreSQL fixture only.
-- Sizes stay inside the Query Session retention limits: 10,000 rows per
-- Result Set and 32 MiB per execution.
CREATE SCHEMA IF NOT EXISTS plan024;

-- Wide rows: 10,000 rows by 100 short columns.
CREATE OR REPLACE VIEW plan024.fixture_wide AS
SELECT {wide}
FROM generate_series(1, 10000) AS g;

-- Large cells: 400 rows by 8 columns of 8,192 characters.
CREATE OR REPLACE VIEW plan024.fixture_large AS
SELECT {large}
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
""")
print("editor lines:", len(lines[:2000]))
