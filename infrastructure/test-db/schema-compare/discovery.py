"""Bounded discovery/lock/snapshot proof on the disposable gate's sessions."""
import json


def identifier(value):
    return '"' + value.replace('"', '""') + '"'


def inventory(session, cap=2000):
    if session.sql("SELECT count(*) FROM pg_namespace WHERE nspname IN "
                   "('discovery_source', 'discovery_target');") != '2':
        raise ValueError('schemaUnavailable')
    # Fixed fixture namespaces; one extra bounded row detects a cap hit.
    rows = session.sql(f"""
        SELECT json_build_object('oid', c.oid::bigint, 'schema', n.nspname,
            'name', c.relname, 'kind', c.relkind,
            'eligible', c.relkind = 'r' AND NOT c.relispartition
              AND NOT EXISTS (SELECT 1 FROM pg_inherits i
                WHERE i.inhrelid = c.oid OR i.inhparent = c.oid)
              AND NOT EXISTS (SELECT 1 FROM pg_depend d
                WHERE d.classid = 'pg_class'::regclass AND d.objid = c.oid
                  AND d.deptype = 'e'))
        FROM pg_class c JOIN pg_namespace n ON n.oid = c.relnamespace
        WHERE n.nspname IN ('discovery_source', 'discovery_target')
          AND c.relkind IN ('r', 'p', 'v', 'm', 'S', 'f', 'c')
        ORDER BY n.nspname, c.relname LIMIT {cap + 1};
    """)
    facts = [json.loads(row) for row in rows.splitlines()]
    if len(facts) > cap:
        raise ValueError('inventoryLimitExceeded')
    return facts


def locked_capture(session, preflight, before_snapshot=lambda: None):
    tables = [row for row in preflight if row['eligible']]
    if len(tables) > 1000:
        raise ValueError('tableLimitExceeded')
    locks = ', '.join('ONLY ' + identifier(row['schema']) + '.' + identifier(row['name'])
                      for row in tables)
    session.begin('LOCK TABLE ' + locks + ' IN ACCESS SHARE MODE;' if locks else '')
    try:
        before_snapshot()
        facts = inventory(session)
        held = session.sql("""
            SELECT relation FROM pg_locks WHERE pid = pg_backend_pid()
              AND locktype = 'relation' AND granted AND mode = 'AccessShareLock';
        """)
        held_oids = {int(oid) for oid in held.splitlines()}
        if any(row['eligible'] and row['oid'] not in held_oids for row in facts):
            raise ValueError('captureChanged')
        return facts
    finally:
        session.sql('ROLLBACK;')


def run(a, b):
    try:
        inventory(a)
    except ValueError as error:
        if str(error) != 'schemaUnavailable':
            raise
    else:
        raise AssertionError('missing schemas must not become equal empty inventories')
    b.sql('''
        CREATE SCHEMA discovery_source; CREATE SCHEMA discovery_target;
        CREATE TABLE discovery_source.orders (id integer);
        CREATE VIEW discovery_target.orders AS SELECT 1 AS id;
        CREATE TABLE discovery_source.inherited (extra integer)
            INHERITS (discovery_source.orders);
        CREATE TABLE discovery_target.plain (id integer);
    ''')
    preflight = inventory(a)
    if len(preflight) != 4 or sum(row['eligible'] for row in preflight) != 1:
        raise AssertionError(preflight)
    baseline = locked_capture(a, preflight)
    if baseline != preflight:
        raise AssertionError('inventory lost excluded counterparts')
    try:
        inventory(a, cap=3)
    except ValueError as error:
        if str(error) != 'inventoryLimitExceeded':
            raise
    else:
        raise AssertionError('inventory cap must reject, never truncate')

    try:
        locked_capture(a, preflight, lambda: b.sql(
            'CREATE TABLE discovery_source.added (id integer);'))
    except ValueError as error:
        if str(error) != 'captureChanged':
            raise
    else:
        raise AssertionError('new unlocked candidate must refuse capture')
    # At most one retry uses a fresh discovery and pre-snapshot lock set.
    retried = locked_capture(a, inventory(a))
    if len(retried) != 5:
        raise AssertionError(retried)
    return {'completeInventory': len(baseline), 'excludedCounterpart': 'view',
            'newCandidate': 'captureChanged', 'retryInventory': len(retried)}
