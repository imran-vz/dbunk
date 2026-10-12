# Backend-owned, constraint-aware table seeding

**Status:** Accepted (2026-06-11, rewritten 2026-10-11)

The backend generates seed rows from the table's catalog
(`backend/src/seed.rs`, `backend/src/backend/table_seed/`): NOT NULL, foreign
keys and uniqueness are guaranteed, CHECK constraints are attempted, and a run
is all-or-nothing. A seed fails fast when a referenced parent table is empty.
The native app seeds PostgreSQL, MySQL and SQLite; ClickHouse keeps the older
path in `backend/src/clickhouse.rs`.

## Engines (2026-10-12)

One engine-neutral planner (`table_seed/engine.rs`) decides what to generate
from each engine's inspected catalog; each runner only reads parent rows and
current maxima and inserts, inside one transaction, comparing the reviewed
catalog before and after.

- **PostgreSQL:** `LOCK TABLE ... IN SHARE ROW EXCLUSIVE MODE` on the target,
  `ACCESS SHARE` on parents.
- **MySQL:** InnoDB tables only; a table without transactions (MyISAM,
  MEMORY) is refused, since a failed seed could not be rolled back. The
  transaction's metadata lock holds off DDL; unique maxima are read `FOR
  UPDATE` and parent rows `LOCK IN SHARE MODE`. Strict `sql_mode` and UTC.
  Cancel sends `KILL QUERY` from a side connection. The reviewed table is
  identified by a hash of `SHOW CREATE TABLE` without its `AUTO_INCREMENT`
  counter.
- **SQLite:** `BEGIN IMMEDIATE` holds the write lock for the whole run, and
  foreign keys are enforced (unlike the interactive session, which matches
  the sqlite3 shell). Only `main` is seeded. Cancel interrupts through the
  progress handler. The reviewed table is identified by a hash of its
  `CREATE` text with its indexes and triggers.

PostgreSQL receipts keep their saved shape; MySQL and SQLite receipts add the
engine and the table hash in place of OIDs.

## Considered options

- **Parsing CHECK expressions.** Rejected.
- **`LOCK TABLES` on MySQL.** Rejected: it ends the open transaction
  implicitly, so the seed would no longer be all-or-nothing.
- **Cascading seeds into parent tables.** Rejected.

## Seed data variety (2026-10-11)

Generated values must vary in shape and length, not only in content, so the
generators are rebuilt around these decisions:

- **Lengths are planned, never truncated.** Each text-like column has a
  length range: the user's range intersected with the column's capacity and
  the generator's own limits (an email is 8 to 254 characters). Generators
  build a value of the chosen length. An empty intersection is a review
  error. Length limits apply only to character types; numeric precision and
  scale bound numeric values.
- **Phone numbers follow libphonenumber metadata.** The `phonenumber` crate
  (pinned) supplies each region's number patterns and lengths. At first use,
  each locale gets a fixed pool of 256 number blocks: a prefix sampled from
  those patterns plus four free digits, validated with `phonenumber::is_valid`
  and pre-rendered in every output format. Rows only fill digits into a
  block's template. Per-row validation cost about 300 µs a row, which a
  million-row seed cannot afford.
- **Fictional-safe by default.** Emails use reserved domains (`example.*`,
  `.test`). Phones in the US, UK, Germany and France come only from ranges
  their regulators reserve for fiction: NANPA 555-0100 to 0199, Ofcom's drama
  numbers, Bundesnetzagentur Mitteilung 148/2021 and ARCEP Décision
  2018-0881. These ranges are used as published even where libphonenumber
  rejects them (France's, and some Ofcom blocks). Spain, Brazil, India and
  Japan publish no such ranges; by Imran's decision (2026-10-12) they get
  valid local numbers in either mode, which may belong to real people.
  Realistic values everywhere are an explicit opt-in. A unique phone column
  draws from the row's own country, so in fictional-safe mode it holds only
  as many rows as the smallest chosen country's ranges (3,200 for the US).
- **Addresses are coherent per row.** A row's street, locality, region,
  postcode, country and phone describe one place: a real postcode with its
  own locality and region, and a made-up street in the locale's style.
  Columns named `state` are not inferred, since they are as often a status.
- **Built-in data is generated in-repo and compiled in.** Names come from
  CC0 and public-domain sources (Wikidata, US Census); postcodes,
  localities and regions from GeoNames postal data (CC BY 4.0, credited in
  `apps/native/THIRD-PARTY-NOTICES.txt`), with Japanese municipality names in
  kanji joined from Wikidata; postcode patterns and address formats from
  Google libaddressinput (Apache-2.0). Only postcodes matching the country's
  pattern are kept. Japanese names
  carry a romaji form for emails and usernames. A generator version, covering
  the data and the generation algorithms, is recorded with each seed, because
  a seed reproduces its rows only under the same version.
- **First locales:** en-US, en-GB, en-IN, de, fr, es, pt-BR and ja.
- **Locales and realism are run-level, lengths per column.** Both are part
  of the reviewed setup: changing either after review disables Apply until
  the recipe is prepared again. A preview of sample values is built from the
  inspected catalog without database work and grants nothing.

### Considered options

- **The `fake` crate.** Rejected: format-string phone numbers, small word
  lists and no length control.
- **Example numbers only.** Rejected: libphonenumber has one example per
  region and type, so values would barely vary.
- **Japan Post postcode data for Japanese addresses.** Rejected: its
  download is not reachable from a script; GeoNames joined with Wikidata
  gives the same kanji municipalities.
- **`rand_regex` for pattern sampling.** Rejected: it brings a second `rand`
  major version, and any change to its sampling would change the rows a seed
  produces.
