# ADR-0024: Backend-enforced production safety policy

**Status**: Accepted (Plan 007, `PAR-004`)

## Decision

dbunk enforces relational connection safety in the backend. Frontend warnings
and confirmation dialogs explain a decision and collect user intent, but they
are not an enforcement boundary. Every write-capable backend command evaluates
one shared policy against the hydrated stored Connection before dispatching any
database work or recording successful activity.

### Threat model

Safety Policy prevents mistakes by an authorized user, such as running a
destructive statement against the wrong Connection. It is not an adversarial
security boundary against a user who owns the database credentials. PostgreSQL
volatile functions and stored procedures can hide writes that statement-text
classification cannot prove. The classifier therefore fails closed where it
cannot establish intent, while acknowledging that server privileges remain the
actual security boundary.

### Policy model

Every stored Connection has three policy inputs:

- Environment is `development`, `test`, `staging`, or `production`, defaulting
  to `development`.
- Safe Mode is `inherit`, `disabled`, `protected`, or `strict`, defaulting to
  `inherit`. Inheritance resolves development and test to `disabled`, staging
  to `protected`, and production to `strict`.
- Relational Connections have a read-only flag, defaulting to false. Redis
  retains its existing independently enforced read-only contract.

`disabled` admits all operations. `protected` requires a confirmed override
for destructive or unknown writes. `strict` requires one for every write.
Read-only refuses every gated write regardless of confirmation and admits only
statements proven to be reads on both arbitrary-SQL paths. Cancelling a backend
is always allowed because it stops work rather than creating it.

Defaults keep the feature dark. Existing Connections deserialize as
development, inherit, and not read-only, which resolves to no new restriction.

### PostgreSQL statement classification

The PostgreSQL classifier uses the shared token lexer and returns an ordered
Statement Class list without retaining statement text. It splits scripts only
at top-level semicolons and recognizes read, DML, DDL, transaction, session,
and unknown statements.

Classification fails closed. Lex failures, `DO`, `CALL`, and unrecognized
heads are `unknown`, which policy treats as a destructive write. `WITH`
statements are scanned for write keywords. `COPY` combines its direction with
the same write-keyword scan, so a data-modifying CTE wrapped by `COPY ... TO`
is still DML. Only `COPY ... TO STDOUT` can retain the read class. Server-file
output and `COPY ... PROGRAM` are destructive DML because they write outside
the client connection or execute a server-side command. Top-level parenthesis
depth determines whether `UPDATE` or `DELETE` has a bounding `WHERE`.

`EXPLAIN` is a read only when execution is provably disabled. Both the
parenthesized options form and the legacy prefix form are parsed. Consequently,
`EXPLAIN UPDATE` and `EXPLAIN VERBOSE SELECT` are reads, while
`EXPLAIN (ANALYZE) UPDATE` and `EXPLAIN ANALYZE UPDATE` take the wrapped
statement's class.

A read-class escalation denylist catches token-exact, unquoted references to
known write-capable functions such as `setval`, `nextval`, `set_config`, and
backend-control functions. This deliberately has conservative false positives:
a column named `nextval` escalates, while an identifier such as `nextval_log`
does not. It limits common accidental bypasses but does not change the threat
model for arbitrary volatile functions. Escalations normally become bounded,
non-destructive DML. `pg_terminate_backend`, `lo_unlink`, and `dblink_exec`
instead become destructive DML because they terminate a session, delete server
state, or execute an opaque remote command; protected mode therefore requires
confirmation for those three read-shaped calls.

Top-level `SELECT ... INTO` is classified as non-destructive DDL because it
creates a relation. Empty and whitespace-only scripts produce no Statement
Classes. A read-only Connection refuses such a script because the admission
rule requires at least one statement proven to be a read. These conservative
rules are stricter than the initial Plan 007 classifier table and are recorded
here explicitly because safety correctness takes priority over permissiveness.

Non-PostgreSQL engines have no statement classifier yet. Arbitrary text sent
through their shared `run_query` path is fully fail closed as one `unknown`
statement. A strict non-PostgreSQL Connection therefore requires confirmation
even for text that the user knows is a read. Command-kind gates remain usable
without engine-specific text parsing.

### Read-only layering

Read-only arbitrary-SQL admission allows only the `read` Statement Class. This
blocks transaction control, session `SET`, DML, DDL, and unknown statements on
both query-session execution and `run_query`. The escalation denylist also
blocks common read-shaped state changes such as `SELECT set_config(...)`.

In-process PostgreSQL connections additionally set
`default_transaction_read_only = on`. This GUC is belt-and-braces protection
for driver paths and opaque volatile-function writes; it is not the enforcement
boundary because SQL can change session state. PostgreSQL restore runs in a
subprocess that cannot inherit this GUC and is refused solely at the command
layer.

### Confirmation and transport

`confirmed: true` is a deliberate-unlock token following the Redis CLI trust
model. The backend can verify the token but cannot verify that a particular UI
dialog was displayed. It accepts the override only where the resolved policy
requires confirmation; it never unlocks read-only.

Actor-owned query-session and result-mutation surfaces return typed
`policyBlocked` and `policyNeedsConfirmation` variants. Legacy string-error
commands return human-readable refusals prefixed by stable
`[policy:read-only]` or `[policy:confirm]` tags. Clients must use a strict
prefix match, never a substring match, because database error text can contain
user-controlled content. This tagged string transport is transitional until
those commands adopt typed error unions.

Every confirmed override that was required by `protected` or `strict` is
recorded best-effort in a bounded local audit. The audit contains only the
Connection, command kind, Statement Class labels, and occurrence time. It
never contains SQL, row values, parameter values, or structured database error
detail. An audit failure is warned about without failing already-authorized
database work.

## Consequences

- One backend policy decision covers every current relational write surface,
  including destination-side copy and subprocess restore.
- Policy refusal occurs before dispatch and does not update Connection
  activity.
- Environment and Safe Mode can be activated later without changing the
  backend contract.
- Unknown procedural or engine-specific SQL may require more confirmation
  than necessary, which is the intentional cost of failing closed.
- The confirmed override audit explains deliberate unlocks without persisting
  sensitive query content.

## Non-goals

- Emulating or continuously re-verifying database privileges.
- Changing transaction defaults based on Environment.
- Maintaining per-statement allowlists.
- Treating UI confirmation as a security boundary.
- Adding MySQL, SQLite, or ClickHouse statement classifiers in this change.
