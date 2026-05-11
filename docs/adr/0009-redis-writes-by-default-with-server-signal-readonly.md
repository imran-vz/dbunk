# ADR-0009 — Redis allows writes by default; server-signal drives auto-read-only

**Status**: Accepted (2026-05-11)

## Context

ADR-0006 settled ClickHouse on a read-only-by-default posture. Mutations
land in Tier 2 behind capability flags; the connection ships safe-by-
default and earns write access through an explicit, server-aware
mechanism. That stance was load-bearing for ClickHouse and is not
controversial.

The natural reflex when adding Redis is to copy that posture: ship Redis
read-only-by-default, require an opt-in for writes. This ADR exists to
say no, and to explain why the symmetry is misleading.

The relevant differences:

1. **ClickHouse mutations are asynchronous; Redis mutations are not.**
   ADR-0006's first argument was that `ALTER TABLE … UPDATE/DELETE`
   queues a background mutation; the HTTP call returns when the mutation
   is *accepted*, not when it is *applied*. The synchronous-looking
   `commit_cell_edits` UX would lie to the user. Redis writes are
   synchronous and atomic per-command: `SET`, `HSET`, `LPUSH`, `DEL`,
   `EXPIRE` return when the operation has been applied, in-memory, on
   the receiving server. The UX cost of getting writes wrong on Redis
   is one extra round trip, not "the mutation is somewhere in the
   middle of being applied across the cluster."
2. **ClickHouse mutations gate on table engine; Redis writes gate on
   nothing.** ADR-0006 also rested on Distributed/View/Kafka/Buffer
   tables being read-only by their nature on ClickHouse. The user
   needed a per-table capability flag. Redis has nothing analogous —
   every key in a MergeTree-shaped (or any other) sense is writable
   unless ACL says otherwise.
3. **The 80% case for Redis is local/dev.** Our Q3 grilling settled
   that standalone Redis (including local Redis, dev Redis, Upstash,
   small managed instances) is the v1 audience. Production cluster
   users are deferred. The audience that benefits most from a read-
   only default is the small minority who connect to production
   masters; the audience that pays the cost is everyone else. The
   trade is bad.
4. **Redis has its own primary safety signal: the `INFO replication`
   `role` field.** A connected replica reports `role:replica`; writes
   sent to it fail at the server. A master reports `role:master` and
   optionally a `connected_slaves` count. The server already tells us
   the safety-relevant fact; we don't need to ask the user to
   re-declare it on the connection form.

The choice is between:

- **(A) Mirror ADR-0006.** Default new Redis connections to read-only;
  require explicit toggle to enable writes. Every editor and key-op
  starts disabled until the user opts in.
- **(B) Server-signal-driven auto-read-only.** Default to writes
  enabled; at connect time, parse `INFO replication`. If
  `role: replica`, mark the connection auto-read-only and surface a
  non-dismissable banner. If `role: master` with replicas attached,
  surface a soft notice. If `role: master` alone, no surface at all.
- **(C) Belt-and-braces.** Both — read-only by default *and*
  server-signal-driven gating.

## Decision

**Redis allows writes by default. Server-signal drives auto-read-only.**
Option (B).

Concretely:

- The connection form has **no `Read only` toggle** in Tier 1. Tier 2
  adds an explicit toggle as a Tier-2 deferral — useful for users on
  masters who want belt-and-braces. Its absence in v1 is deliberate.
- At connect time, after `PING` succeeds, the capabilities pipeline
  fires `INFO replication` (and `INFO server`). Parse `role:`,
  `connected_slaves:`, `master_link_status:`.
- If `role:replica`: the connection is **auto-marked read-only**. The
  workspace renders a non-dismissable banner: "Server reports role =
  replica. Writes will fail on the server; dbunk has disabled
  mutations for this connection." Editors render in read-only mode.
  The CLI rejects any command on the
  `redis-destructive-commands.toml` hard list before sending. The
  banner cannot be dismissed because the underlying fact does not
  go away during the session.
- If `role:master` and `connected_slaves > 0`: soft notice — "This
  server has N replicas attached. It may be a production master."
  Notice is **dismissable per-connection** (state lives in the
  connection record's `dismissed_replica_warning_at` field, populated
  by the dismiss action).
- If `role:master` and `connected_slaves == 0`: no surface at all.
  The expected local/dev case; we do not nudge the user.
- **Destructive commands are guarded regardless of read-only state.**
  `FLUSHDB`, `FLUSHALL`, `DEBUG`, `SHUTDOWN`, `CONFIG SET`,
  `CONFIG RESETSTAT`, `SCRIPT FLUSH`, `SCRIPT KILL`, `CLIENT KILL`,
  `KEYS` require a typed-confirmation in the CLI; the editor surfaces
  (`DEL` confirmation modal, `FLUSHDB` per-DB modal) are also gated.
  The destructive-command list is **enforced at the backend** so it
  cannot be bypassed by frontend modifications, scripted commands, or
  EVAL scripts that wrap destructive operations.
- **The destructive-command list is shared between Rust and TypeScript
  via codegen** (`src-tauri/src/redis/destructive-commands.toml` →
  `destructive_commands.rs` + `destructive-commands.ts`). The TOML is
  the single source; both languages compile-time-read it. A CI check
  asserts the generated files match the TOML.

This decision applies to Redis only. It does not amend ADR-0006 for
ClickHouse; it does not extend to any future engine. Future engines
that share Redis's shape (synchronous atomic writes, server-reported
replica role) may follow this pattern; engines that share ClickHouse's
shape (async mutations, table-engine-dependent write capability) should
follow ADR-0006.

## Consequences

- **Dev experience matches expectation.** A user pointing dbunk at a
  local `redis-server` can edit a string immediately. No "enable writes"
  click, no opt-in modal. The local-dev case is the 80% case.
- **Production safety lives in the server signal.** Users who connect
  to a replica get the read-only outcome they would have wanted; users
  on a master with replicas attached get a soft notice asking them to
  think twice. The auto-detection costs us one `INFO replication` per
  connection (already fired as part of the capabilities pipeline) and
  one banner-shaped component.
- **The destructive-command list is the hard floor.** Even if the
  server-signal heuristic missed a production case (a master with
  zero replicas in some setups), accidental `FLUSHDB` is still blocked
  by typed-confirmation. The defense is layered, not single-point.
- **The CLI's destructive guard is the same plumbing as the editors'.**
  Both consult the same `DESTRUCTIVE_COMMANDS` list at the backend, so
  the user's mental model is consistent: "if this command can destroy
  a lot, dbunk asks me to type the name." The frontend modal is the
  UX; the backend check is the safety.
- **The Tier 2 read-only toggle is genuinely additive.** Users who want
  belt-and-braces — connect to a master, but never accidentally
  mutate — will be served by the deferral. Its absence in Tier 1 is
  not a permanent stance; it is a "we don't believe the friction tax on
  the 80% case is worth the safety win on a small audience that the
  destructive-command guard already mostly serves."
- **The banner on replica connections is non-dismissable by design.**
  The user cannot turn off the safety signal because the underlying
  fact (the server *is* a replica) doesn't change during the session.
  If we made the banner dismissable, the next person who opens the
  app and sees no banner would assume "this is a master." Banner state
  is session-scoped, never persisted.
- **The soft notice on multi-replica masters is dismissable** because
  the underlying fact ("this *might* be production") is the user's
  judgment call, not ours. Dismissing is the user's stated assertion
  "I know, I checked, I'm OK."
- **Divergence from ADR-0006 is the expected outcome, not a violation.**
  Future ADRs that propose "let's harmonize the read-only posture
  across engines" should be rejected with this ADR as the citation.
  The two engines look similar at first glance and they are not.
  Harmonizing them would either (a) saddle Redis with a friction tax it
  doesn't need or (b) lift ClickHouse's safety floor in ways that
  recreate the async-mutation footgun ADR-0006 closed.
- **The Tauri command for Redis writes always sees the connection's
  auto-read-only state.** `dispatch/keyvalue.rs` checks it before
  issuing any write command. The check is in the dispatcher, not the
  editor, because a future surface (e.g. the bulk-edit Tier 2 tab)
  needs the same gate and shouldn't have to re-implement it.
