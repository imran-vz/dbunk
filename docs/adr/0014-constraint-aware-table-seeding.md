# Backend-owned, constraint-aware table seeding

**Status:** Accepted (2026-06-11, rewritten 2026-10-11)

The backend generates seed rows from the table's catalog
(`backend/src/seed.rs`, `backend/src/backend/table_seed/`): NOT NULL, foreign
keys and uniqueness are guaranteed, CHECK constraints are attempted, and a run
is all-or-nothing. A seed fails fast when a referenced parent table is empty.
The native app seeds PostgreSQL; other engines' generators remain in
`backend/src/dispatch/seed.rs`.

## Considered options

- **Parsing CHECK expressions.** Rejected.
- **Cascading seeds into parent tables.** Rejected.

## Seed data variety (2026-10-11)

Generated values must vary in shape and length, not only in content, so the
generators are rebuilt around these decisions:

- **Lengths are planned, never truncated.** Each text-like column has a
  length range: the user's range intersected with the column's capacity and
  the generator's own limits (an email is 6 to 254 characters). Generators
  build a value of the chosen length. An empty intersection is a review
  error. Length limits apply only to character types; numeric precision and
  scale bound numeric values.
- **Phone numbers follow libphonenumber metadata.** The `phonenumber` crate
  supplies each region's number patterns and lengths. Values are sampled from
  those patterns with the run's own RNG, so a seed reproduces its rows, and
  must pass `phonenumber::is_valid`.
- **Fictional-safe by default.** Phones come from ranges regulators reserve
  for fiction and emails use reserved domains (`example.*`, `.test`).
  Realistic, possibly assigned values are an explicit opt-in.
- **Built-in data is generated in-repo and compiled in.** Names come from
  CC0 and public-domain sources (Wikidata, US SSA, US Census); cities from
  GeoNames (CC BY 4.0, credited in the legal notices); postcode patterns and
  address formats from Google libaddressinput (Apache-2.0). Japanese names
  carry a romaji form for emails and usernames. The data version is recorded
  with each seed, because a seed reproduces its rows only on the same data.
- **First locales:** en-US, en-GB, en-IN, de, fr, es, pt-BR and ja.

### Considered options

- **The `fake` crate.** Rejected: format-string phone numbers, small word
  lists and no length control.
- **Example numbers only.** Rejected: libphonenumber has one example per
  region and type, so values would barely vary.
- **`rand_regex` for pattern sampling.** Rejected: it brings a second `rand`
  major version, and any change to its sampling would change the rows a seed
  produces.
