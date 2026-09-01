# ADR-0027: Table-scoped security and routine DDL

**Status**: Accepted (2026-09-02, Plan 016, `PAR-007`)

## Context

ADR-0026 established the typed PostgreSQL object DDL workflow: operations
cross IPC, the backend renders one statement per operation, preview and apply
share one deterministic generator, and grouping is honest about
non-transactional statements. After Plans 013–015 that workflow covered
schema-level lifecycle and the table structure editor, but three table-scoped
surfaces still generated SQL strings in the frontend (grants, row-level
security, triggers), no operation could create a table, and no operation could
create or replace a function or procedure.

Those additions raise questions the earlier vocabulary did not have to answer:
a routine body is arbitrary code that must not be parsed as SQL fragments; a
`REVOKE` or a disabled trigger is harmless to the statement classifier yet
breaks running applications; and triggers and policies are identified by
table plus name rather than by an Object Ref.

## Decision

### Opaque routine bodies

`createFunction` and `createProcedure` carry `arguments`, `returns`,
`language`, and `body` as separate fields. `arguments` and `returns` are
signature fragments validated in a dedicated context: balanced delimiters, no
statement boundary, no dollar sign anywhere (a dollar quote would open a body,
and positional parameters do not exist in declarations, so the byte is refused
outright rather than classified), no top-level routine clause keyword (`AS`,
`LANGUAGE`, `RETURNS`, `SECURITY`, `SET`, `WITH`, …), and no literal in
`returns`. `language` must lex as one identifier, quoted or not, and is
rendered as typed. The body is never validated as
SQL. The renderer seals it inside a dollar quote whose tag is the first of
`$dbunk$`, `$dbunk1$`, `$dbunk2$`, … that does not occur in the body, so no
body content can terminate the string. The tag is a pure function of the body,
which keeps preview and apply rendering identical SQL. The finished statement
still has to classify as exactly one DDL statement, which is what proves the
lexer saw the body as one token. Bodies are excluded from destructiveness
scanning: creating a routine does not run it, and `OR REPLACE` is the
destructive signal because it overwrites existing code.

`CREATE OR REPLACE` is the only "alter" for routines. PostgreSQL refuses a
changed return type or argument names under `OR REPLACE`; that refusal
surfaces as the existing typed `database` error with its SQLSTATE rather than
being predicted at preview, which stays free of catalog lookups. Routine
facts now include `body` (`prosrc`), `strict`, `securityDefiner`, and
`parallel` next to the existing `arguments` and `returns` text, so an editor
round-trips the header without parsing `pg_get_functiondef` output.

### Renderer-owned destructiveness for security operations

The statement classifier marks `REVOKE`, `ALTER TABLE … DISABLE TRIGGER`, and
`ALTER TABLE … DISABLE ROW LEVEL SECURITY` as ordinary non-destructive DDL.
That is right for the query editor's mistakes-not-adversaries model and wrong
for review: each removes a guarantee an application may depend on. The
renderer therefore sets `destructive: true` for `revokePrivileges`,
`setTriggerEnabled { disable }`, `setRowLevelSecurity { enabled: false }`,
and `dropPolicy` (also destructive by classification). The preview badge and
the safety gate's typed-confirmation path engage on that flag; nothing in the
frontend re-derives it. Removing `FORCE` while leaving row security enabled is
not destructive on its own.

### Triggers and policies are structure entries, not Object Refs

`createTrigger`, `dropTrigger`, `setTriggerEnabled`, `createPolicy`, and
`dropPolicy` address their object by `schema`, `table`, and `name`, like
`dropConstraint` and `dropIndex`. The PostgreSQL `TableStructure` reports
`triggers`, `policies`, `privileges`, and `rowSecurity` behind three new
capability flags; other engines report empty sections and `false` flags.
Trigger rows carry the four-way `tgenabled` state and the verbatim
`pg_get_triggerdef` text (the `WHEN` clause and arguments are displayed, not
parsed). Policy role lists are reported as PostgreSQL stores them: a list that
named `PUBLIC` collapses to the single pseudo-role `public`; both policy
expressions are optional, as in PostgreSQL's grammar. Privilege rows are the
explicit ACL from `aclexplode(relacl)` only: a relation whose owner has never
granted anything reports an empty list, and the owner's implicit privileges
are never synthesized. Catalog rows decode into typed records, and the
`tgenabled`, policy command, and permissive/restrictive codes decode into
closed enums, so an unexpected catalog value is an error rather than a
plausible default. Promoting triggers or
policies to Object Refs later requires the ADR-0026 catalog, describe, and DDL
decisions; nothing here special-cases them.

### Closed privilege vocabulary

`grantPrivileges` and `revokePrivileges` take an Object Ref target (relations
rendered `ON TABLE`, plus sequences, schemas, functions, and procedures with
their identity arguments), a closed `PgPrivilege` set, an `allPrivileges`
flag that is mutually exclusive with the list, and a `PgGrantee` that renders
`PUBLIC` as the keyword and every role as a quoted identifier, so a role named
`public` can never be mistaken for the pseudo-role. Validation rejects a
privilege the target kind cannot carry, duplicates, and empty role names.
Whether the grantee exists is left to apply, which reports SQLSTATE `42704`.

### Create table

`createTable` renders one `CREATE TABLE` with column definitions (tagged
defaults, nullability, `GENERATED … AS IDENTITY`) and table constraints
(primary key, unique, check, foreign key). Identity columns must be
non-nullable and default-free; every constraint column must be declared;
foreign-key column counts must match. Comments and indexes are separate
operations the designer appends, so a designer batch is one atomic group
unless it ends in a concurrent index, which the existing grouping runs as its
own standalone statement after the table has committed.

### Module layout

`PgObjectOp` stays the serde dispatcher (an internally tagged enum whose
variants are newtypes over payload structs, which serde flattens so the wire
format is unchanged). Each payload lives in a domain module under
`postgres/object_ddl/` (`object`, `column`, `constraint`, `index`, `table`,
`view`, `sequence`, `enum_type`, `routine`, `trigger`, `policy`, `privilege`)
and implements one `ObjectOperation` trait: `validate`, `fragments`, and
`render` for one operation live together. Dispatch is an exhaustive match
with no wildcard, so a new SQL-bearing operation cannot compile without
declaring what it scans for destructiveness.

## Consequences

- Plan 017 can activate a table designer, a routine editor, and typed
  trigger, policy, and privilege affordances without a second generation
  path or a frontend destructiveness classifier.
- The Specialized tab's remaining string generators can be retired on
  PostgreSQL; ClickHouse keeps its frontend generator.
- Database lifecycle, aggregates, `ALTER POLICY`, attribute-only
  `ALTER FUNCTION`, roles and ownership, default privileges, partitions,
  rules, event triggers, extensions, non-enum types and domains, and
  tablespaces remain deferred; each needs its own catalog or scope decision
  first.

## Related

- ADR-0026: the object DDL workflow this extends.
- ADR-0024: the safety gate that consumes the destructive flag.
