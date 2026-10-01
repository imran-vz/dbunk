#!/usr/bin/env python3
"""Plan 022 native WebView walkthrough: drives the real desktop app.

Prerequisites, all owned by this harness (see README.md, "Plan 022 walkthrough"):

    python3 infrastructure/test-db/schema-compare/webview-driver/fixtures.py up
    DBUNK_DEV_CONFIG_DIR=/tmp/dbunk-plan022-gate/config \
    CARGO_TARGET_DIR=/tmp/dbunk-plan022-gate/target \
      pnpm tauri dev --config infrastructure/test-db/schema-compare/webview-driver/tauri.walkthrough.json

Usage: walkthrough.py [scenario ...]   (no arguments: every scenario in order)

The app window must stay visible: a hidden WebView stops observing by design
and macOS suspends its content process. The script refuses to run hidden.
Results are written to /tmp/dbunk-plan022-gate/report.json. No DSN is
accepted; endpoints come only from the fixture state file.
"""
import ctypes
import hashlib
import json
from pathlib import Path
import re
import subprocess
import sys
import time
import urllib.request

HERE = Path(__file__).resolve().parent
WORK = Path('/tmp/dbunk-plan022-gate')
DRIVER = 'http://localhost:3000/__dbunk'
BINARY = '^/tmp/dbunk-plan022-gate/target/debug/dbunk'
FIXTURES = json.loads((WORK / 'fixtures.json').read_text())
PRIMARY = FIXTURES['primary']['name']
MAIN = ['gate-main', 'src', 'gate-main', 'tgt']
report = {}
current = {}


# --- plumbing ---------------------------------------------------------------

def ev(body, timeout=90, **params):
    """Runs an async function body in the WebView with `A` (params) and `h`."""
    expr = ('(async () => { const A = %s; const h = window.__h; %s })()'
            % (json.dumps(params), body))
    request = urllib.request.Request(DRIVER + '/eval', data=expr.encode(),
                                     headers={'x-timeout-ms': str(timeout * 1000)})
    out = json.load(urllib.request.urlopen(request, timeout=timeout + 15))
    if not out.get('ok'):
        raise RuntimeError(out.get('error'))
    return out.get('value')


def raw(expr, timeout=20):
    request = urllib.request.Request(DRIVER + '/eval', data=expr.encode(),
                                     headers={'x-timeout-ms': str(timeout * 1000)})
    return json.load(urllib.request.urlopen(request, timeout=timeout + 15))


def ensure():
    """Helpers installed and the page visible, or stop."""
    out = raw((HERE / 'helpers.js').read_text() + ';document.visibilityState')
    if not out.get('ok'):
        raise SystemExit(f'driver error: {out.get("error")}')
    if out['value'] != 'visible':
        # Focusing switches the active desktop Space to the fixture window.
        raw("(async () => { for (const c of ['show', 'unminimize', 'set_focus']) await window.__h.window(c); })()")
        time.sleep(1.2)
        if raw('document.visibilityState')['value'] != 'visible':
            raise SystemExit('The fixture window is hidden. Bring it to the visible desktop and rerun.')


def sh(*args, stdin=None, check=True):
    return subprocess.run(args, input=stdin, text=True, capture_output=True, check=check).stdout.strip()


def psql(sql, database='cmp_main', name=PRIMARY):
    return sh('docker', 'exec', '-i', name, 'psql', '-X', '-q', '-v', 'ON_ERROR_STOP=1',
              '-U', 'postgres', '-d', database, '-Atc', sql)


def background_sql(sql, database='cmp_main'):
    """Runs one implicit-transaction script in a detached session tagged gate-hold."""
    sh('docker', 'exec', '-d', '--env', 'PGAPPNAME=gate-hold', PRIMARY, 'psql', '-X', '-q',
       '-U', 'postgres', '-d', database, '-c', sql)


def hold_lock(seconds=45, extra=''):
    background_sql(f'BEGIN; LOCK TABLE src.lock_me IN ACCESS EXCLUSIVE MODE; {extra} '
                   f'SELECT pg_sleep({seconds}); COMMIT;')
    for _ in range(100):
        if psql("SELECT count(*) FROM pg_locks WHERE relation = 'src.lock_me'::regclass "
                "AND mode = 'AccessExclusiveLock' AND granted") == '1':
            return
        time.sleep(0.05)
    raise RuntimeError('lock holder did not start')


def release_lock():
    psql("SELECT pg_terminate_backend(pid) FROM pg_stat_activity WHERE application_name = 'gate-hold'")


def busy(database='cmp_main'):
    """Non-idle backends on a fixture database, excluding this probe and lock holders.

    A comparison backend is in a transaction or waiting on a lock; the app's
    pooled sessions sit idle. An empty string means nothing is left behind.
    """
    sql = ("SELECT coalesce(string_agg(state || ':' || coalesce(wait_event_type, '-'), ',' ORDER BY pid), '') "
           "FROM pg_stat_activity WHERE datname = current_database() AND pid <> pg_backend_pid() "
           "AND application_name <> 'gate-hold' AND state <> 'idle'")
    found = psql(sql, database)
    if found:  # A pool health check can be mid-statement; look twice.
        time.sleep(0.5)
        found = psql(sql, database)
    return found


def check(name, condition, detail=None):
    current.setdefault('checks', []).append({'check': name, 'pass': bool(condition), 'detail': detail})
    print(f"   {'ok  ' if condition else 'FAIL'} {name}" + (f' :: {detail}' if detail is not None and not condition else ''),
          flush=True)
    return bool(condition)


def note(key, value):
    current[key] = value


def clip(text, limit=700):
    return text if len(text) <= limit else text[:limit] + f'…[{len(text)} chars]'


# --- memory -----------------------------------------------------------------

_lib = ctypes.CDLL('/usr/lib/libSystem.B.dylib')
_lib.responsibility_get_pid_responsible_for_pid.restype = ctypes.c_int
_responsible = _lib.responsibility_get_pid_responsible_for_pid


def memory(label):
    """RSS and physical footprint of the app process and its own WebKit helpers."""
    app = int(sh('pgrep', '-f', BINARY).split()[0])
    def started(text):
        return time.mktime(time.strptime(text, '%a %b %d %H:%M:%S %Y'))
    processes = []
    for line in sh('ps', '-axo', 'pid=,rss=,lstart=,comm=').splitlines():
        parts = line.split(None, 7)
        processes.append((int(parts[0]), int(parts[1]), started(' '.join(parts[2:7])), parts[7]))
    app_start = next(p[2] for p in processes if p[0] == app)
    row = {'label': label, 'at': time.strftime('%H:%M:%S')}
    for pid, rss, start, command in processes:
        helper = 'WebKit' in command and _responsible(pid) == _responsible(app) and start >= app_start
        if pid != app and not helper:
            continue
        summary = sh('vmmap', '--summary', str(pid), check=False)
        footprint = re.search(r'Physical footprint:\s+([\d.]+)([KMG])', summary)
        scale = {'K': 1 / 1024, 'M': 1, 'G': 1024}
        name = 'native' if pid == app else command.rsplit('.', 1)[-1]
        row[name] = {'rssMiB': round(rss / 1024, 1),
                     'footprintMiB': round(float(footprint.group(1)) * scale[footprint.group(2)], 1)
                     if footprint else None}
    return row


def mem_line(row):
    return ' '.join(f"{k}={v['rssMiB']}/{v['footprintMiB']}" for k, v in row.items() if isinstance(v, dict))


# --- setup ------------------------------------------------------------------

def setup():
    """Onboarding, fixture connections and the workspace; idempotent."""
    settings = ev("const { store } = await h.mods(); await h.until(() => store.getState().appSettings);"
                  "return store.getState().appSettings;")
    if not settings['onboardingCompleted']:
        # Onboarding clears the shared `dbunk` keychain entry: never do that to a real one.
        present = subprocess.run(['security', 'find-generic-password', '-s', 'dbunk', '-a',
                                  'connection-credentials'], capture_output=True).returncode == 0
        if present:
            raise SystemExit('A dbunk keychain entry exists; onboarding here would delete it. Stop.')
        ev("const { store } = await h.mods();"
           "if (!(await store.getState().configureCredentialStorage({ mode: 'plain-sqlite' }))) throw new Error('onboarding failed');")
    connections = [
        ['gate-main', 'cmp_main', '127.0.0.1', FIXTURES['primary']['port'], 'development', None],
        ['gate-other', 'cmp_other', '127.0.0.1', FIXTURES['primary']['port'], 'staging', None],
        ['gate-minor', 'cmp_minor', '127.0.0.1', FIXTURES['minor']['port'], 'test', None],
        ['gate-pg17', 'cmp_pg17', '127.0.0.1', FIXTURES['other']['port'], 'production', None],
        ['gate-tunnel', 'cmp_main', FIXTURES['primary']['bridgeAddress'], 5432, 'development', 'gate-bastion'],
    ]
    state = ev(r"""
      const { store, forms } = await h.mods();
      await store.getState().loadBastionServers();
      const saved = await store.getState().saveBastionServer({ id: 'gate-bastion', name: 'gate-bastion',
        host: '127.0.0.1', port: A.bastionPort, user: 'tunnel', authMethod: 'password',
        password: { action: 'set', value: 'tunnel' }, privateKeyContent: { action: 'keep' }, passphrase: { action: 'keep' } });
      if (!saved) throw new Error('bastion save failed: ' + JSON.stringify(store.getState().bastionStatus));
      for (const [name, database, host, port, environment, bastion] of A.connections) {
        const parsed = forms.connectionSchema.parse({ ...forms.EMPTY_NEW_DEFAULTS, name, engine: 'PostgreSQL',
          host, port, database, user: 'postgres', password: 'x', environment,
          sshTunnelEnabled: bastion !== null, sshTunnelBastionServerId: bastion ?? '' });
        const existing = store.getState().connections.find((c) => c.id === name);
        const next = forms.buildConnectionFromForm(parsed, name, { status: 'disconnected', latency: '' });
        if (!existing) await store.getState().addConnection(next);
        else if (existing.port !== port || existing.host !== host) {
          if ((await store.getState().updateConnection(next)) !== 'saved') throw new Error('update failed ' + name);
        }
      }
      await h.open('gate-main');
      const left = await h.clearJobs();
      return { left, connections: store.getState().connections.map((c) => `${c.id}@${c.host}:${c.port}/${c.database}:${c.status}`) };
    """, connections=connections, bastionPort=FIXTURES['bastion']['port'])
    print('   setup:', state, flush=True)
    return state


def fresh():
    ev("await h.open('gate-main'); await h.clearJobs(); window.__trace.reset();")


COMPARE = "return await h.compare(A.endpoints, { wait: A.wait ?? true });"


def compare(endpoints, wait=True):
    return ev(COMPARE, endpoints=endpoints, wait=wait)


def coverage():
    return ev(r"""
      const details = [...h.root().querySelectorAll('details')].find((d) => d.innerText.includes('Coverage'));
      if (!details.open) details.querySelector('summary').click();
      await h.sleep(150);
      const text = details.innerText;
      details.querySelector('summary').click();
      return text;
    """)


# --- scenarios --------------------------------------------------------------

def same_connection():
    fresh()
    result = compare(MAIN)
    note('ms', result['ms']); note('phases', result['phases'])
    check('completes', result['job']['phase'] == 'completed', result['job'])
    text = coverage()
    note('coverage', clip(text, 1500))
    check('one shared transaction is stated', 'Both schemas were read in one transaction' in text)
    check('supported scope and exclusions are reachable', 'Not compared:' in text and 'PostgreSQL 16.15' in text)
    objects = ev("return h.objects();")
    note('objects', objects)
    expected = {'equal_table': 'Equal within scope', 'orders': 'Changed', 'only_in_source': 'Source only',
                'only_in_target': 'Target only', 'mixed_kind': 'Not comparable', 'a_view': 'Not comparable',
                'big_values': 'Changed', 'lock_me': 'Equal within scope'}
    check('difference kinds match the fixture', all(objects.get(k) == v for k, v in expected.items()), objects)
    check('page position is an explicit server page', re.search(r'1–\d+ of \d+', result['text']) is not None)
    orders = ev("return await h.object('orders');")
    check('known changes coexist with incomparable fields', re.search(r'\d+ changed · [1-9]\d* not comparable', orders) is not None,
          orders[:80])
    check('field page is capped at 100', ev("return h.fieldRows().length;") == 100 and 'Fields 1–100 of' in orders)
    amount = ev("return await h.field('column amount default');")
    check('changed value shows both sides', 'Changed' in amount['inspector'] and re.search(r'SOURCE.*\n0\n', amount['inspector'], re.S)
          and amount['inspector'].rstrip().endswith('1'), amount['inspector'])
    created = ev("return await h.field('column created default');")
    check('incomparable field carries its reason', 'Expression outside the supported scalar grammar' in created['row'][3]
          and 'Expression outside' in created['inspector'], created)
    extra = ev("return await h.field('column extra type');")
    check('missing side reads Absent', extra['row'][1] == 'Absent' and 'SOURCE\nAbsent' in extra['inspector'], extra)
    null_row = ev("return h.fieldRows().find((r) => r[0] === 'table comment');")
    check('observed NULL is distinct from Absent', null_row[1] == 'NULL', null_row)
    mixed = ev("return await h.object('mixed_kind');")
    check('excluded counterpart is not directional absence',
          'Target: excluded, not an ordinary table (view)' in mixed and 'Source: eligible ordinary table' in mixed
          and 'Absent' not in mixed, mixed)
    only = ev("await h.object('only_in_source'); return h.fieldRows()[0];")
    check('directional absence is labelled', only[2] == 'Absent' and only[3] == 'Source only', only)
    ev("await h.object('big_values');")
    markup = ev("return await h.field('column markup comment');")
    injected = ev("return { flag: window.__dbunkInjected ?? null, nodes: h.root().querySelectorAll('img, script, b, i').length };")
    check('captured markup renders as text', '<img src=x onerror=' in markup['inspector'] and injected == {'flag': None, 'nodes': 0},
          injected)


def chunks():
    fresh()
    compare(MAIN)
    ev("await h.object('big_values');")
    hashes = {}
    for field, column in (('table comment', 0), ('column id comment', 1)):
        expected = psql("SELECT encode(sha256(convert_to(d, 'UTF8')), 'hex') || ':' || octet_length(d) FROM "
                        f"(SELECT col_description('src.big_values'::regclass, {column}) d UNION ALL "
                        f"SELECT col_description('tgt.big_values'::regclass, {column})) s").split('\n')
        steps = ev(r"""
          await h.field(A.field);
          const steps = [];
          const snap = (label) => steps.push(`${label} | ${h.paneHead('Source')} | ${h.paneHead('Target')}`);
          snap('first');
          const bad = () => /�/.test(h.pane('Source').innerText + h.pane('Target').innerText);
          let replacement = bad();
          for (const side of ['Source', 'Target']) {
            while (await h.chunk(side, 'Next')) { snap(`${side} next`); replacement ||= bad(); }
            while (await h.chunk(side, 'Previous')) { replacement ||= bad(); }
            snap(`${side} back at start`);
          }
          // Integrity only: reassemble outside the view through the typed client.
          const { client, observer, form } = await h.mods();
          const job = observer.store.getState().jobs.find((j) => j.jobId === form.getState().selectedJobId);
          const request = { identity: { jobId: job.jobId, resultId: job.resultId }, source: job.source, target: job.target };
          const page = await client.fields(request, { kind: 'table', name: 'big_values' }, 0);
          const wanted = A.field.replace(/^column /, '').split(' ');
          const item = page.items.find((i) => { const p = JSON.stringify(i.path); return wanted.every((w) => p.includes(w)) && (A.field.startsWith('table') ? i.path.kind === 'table' : p.includes('"id"')); });
          const out = { steps, replacement, sides: {} };
          for (const side of ['source', 'target']) {
            let offset = 0, text = ''; const sizes = [];
            for (;;) { const c = await client.value(request, item[side], offset); text += c.text; sizes.push(c.nextOffset - c.offset); if (c.complete) break; offset = c.nextOffset; }
            const bytes = new TextEncoder().encode(text);
            const digest = await crypto.subtle.digest('SHA-256', bytes);
            out.sides[side] = { sizes, bytes: bytes.length, sha256: [...new Uint8Array(digest)].map((b) => b.toString(16).padStart(2, '0')).join('') };
          }
          return out;
        """, field=field)
        hashes[field] = steps
        for side, want in zip(('source', 'target'), expected):
            got = steps['sides'][side]
            check(f'{field} {side}: bytes match the database', f"{got['sha256']}:{got['bytes']}" == want, (got, want))
            check(f'{field} {side}: chunks stay within 64 KiB', max(got['sizes']) <= 65536, got['sizes'])
        check(f'{field}: no replacement characters while paging', not steps['replacement'])
        check(f'{field}: partial ranges are labelled in bytes', 'Partial · bytes 0–' in steps['steps'][0], steps['steps'][0])
    note('chunks', hashes)
    trace = ev("return h.traceSummary();")
    note('trace', trace)
    check('one outstanding read, each acknowledged', trace['maxInflightReads'] == 1 and trace['unackedNow'] == 0
          and trace['maxUnacked'] <= 1, trace)


def independent_databases():
    fresh()
    result = compare(['gate-main', 'src', 'gate-other', 'src'])
    note('ms', result['ms']); note('phases', result['phases'])
    check('completes', result['job']['phase'] == 'completed', result['job'])
    text = coverage()
    check('independent captures are stated', 'Independent captures. This is not a single cross-database snapshot.' in text, clip(text))
    ev("await h.object('orders');")
    status = ev("return await h.field('column status default');")
    note('statusDefault', status)
    check('cross-database change is found', status['row'][3] == 'Changed' and 'other-db' in status['inspector'], status)
    check('both identities stay visible', re.search(r'gate-main\s+src\s+→\s+gate-other\s+STAGE\s+src', result['text']) is not None,
          clip(result['text'], 300))


def minor_versions():
    fresh()
    result = compare(['gate-main', 'src', 'gate-minor', 'src'])
    note('ms', result['ms']); note('phases', result['phases'])
    check('completes across PG16 minors', result['job']['phase'] == 'completed', result['job'])
    text = coverage()
    check('both server versions are shown', '16.15' in text and '16.14' in text, clip(text))
    ev("await h.object('equal_table');")
    rendered = ev("return await h.field('column name default');")
    structured = ev("return h.fieldRows().find((r) => r[0] === 'column name type');")
    note('rendered', rendered); note('structured', structured)
    check('rendered expressions are not comparable across minors',
          'Rendered on different server versions' in rendered['row'][3], rendered['row'])
    check('structured facts still compare', structured[3] == 'Equal within scope', structured)


def refusals():
    fresh()
    for label, endpoints, needles in (
        ('PG17 target', ['gate-main', 'src', 'gate-pg17', 'src'], ['17.11', 'arget', '16']),
        ('PG17 source', ['gate-pg17', 'src', 'gate-main', 'src'], ['17.11', 'ource', '16']),
        ('table cap', ['gate-main', 'over_src', 'gate-main', 'many_tgt'], ['No complete result was produced']),
        ('missing schema', ['gate-main', 'nope', 'gate-main', 'tgt'], ['may not exist']),
    ):
        result = compare(endpoints)
        failure = ev("return h.root().querySelector('[role=alert]')?.innerText ?? h.text();")
        note(label, {'phases': result.get('phases'), 'failure': result.get('job', {}).get('failure'), 'text': clip(failure, 400)})
        check(f'{label}: fails without a result', result.get('job', {}).get('phase') == 'failed', result.get('job'))
        check(f'{label}: message is specific', all(n in result['text'] for n in needles), clip(result['text'], 500))
        ev("await h.clearJobs();")
    result = compare(['gate-main', 'same_a', 'gate-main', 'same_b'])
    check('identical schemas read Equal within scope', ev("return h.objects();") == {'item': 'Equal within scope'}, result['text'][-300:])
    result = compare(['gate-main', 'empty_a', 'gate-main', 'empty_b'])
    note('empty', clip(result['text'], 600))
    check('empty projection is not called equal', 'No ordinary tables to compare' in result['text'] and 'does not mean the schemas are equal' in result['text'])
    result = compare(['gate-main', 'views_a', 'gate-main', 'views_b'])
    check('views-only schemas list excluded objects', ev("return h.objects();") == {'only_view': 'Not comparable'}, result['text'][-300:])
    ev("await h.clearJobs(); window.__trace.reset();")
    blank = compare(['gate-main', 'src', 'gate-main', ''])
    starts = ev("return window.__trace.calls.filter((c) => c.command === 'start').length;")
    check('missing schema name is refused before native admission', blank.get('formError') == 'Enter the target schema name.' and starts == 0,
          (blank.get('formError'), starts))
    double = ev(r"""
      await h.endpoints('gate-main', 'same_a', 'gate-main', 'same_b');
      window.__trace.reset();
      const button = h.compareButton();
      button.click(); button.click(); button.click();
      await h.sleep(1500);
      return window.__trace.calls.filter((c) => c.command === 'start').length;
    """)
    check('repeated Compare presses admit one job', double == 1, double)


def cancel_and_busy():
    fresh()
    hold_lock()
    try:
        started = compare(MAIN, wait=False)
        waiting_backends = None
        waiting = ev("await h.sleep(1500); const { observer } = await h.mods(); return { text: h.text(), job: observer.store.getState().jobs.at(-1) };")
        waiting_backends = busy()
        note('waiting', {'phase': waiting['job']['phase'], 'backends': waiting_backends, 'text': clip(waiting['text'], 400)})
        check('one dedicated backend waits on the lock', 'Lock' in waiting_backends, waiting_backends)
        check('lock wait shows phase text and counts, no percentage',
              'objects read' in waiting['text'] and '%' not in waiting['text'] and waiting['job']['phase'] != 'completed', waiting['job'])
        second = compare(['gate-main', 'src', 'gate-main', 'same_a'], wait=False)
        third = compare(['gate-other', 'src', 'gate-other', 'tgt'], wait=False)
        note('second', second); note('third', third)
        refused = (second.get('formError') or '') + (third.get('formError') or '')
        check('busy limit is reported with a pointer to active jobs', 'ctive' in refused or 'busy' in refused.lower(), refused)
        cancelled = ev(r"""
          const { observer } = await h.mods();
          await h.selectJob('src', 'tgt');
          const seen = [];
          const watch = setInterval(() => { const t = h.text(); if (/Cancelling…/.test(t) && !seen.includes('cancelling')) seen.push('cancelling'); }, 5);
          const output = h.root().querySelector('output');
          h.click(h.byText('button', 'Cancel', output), 'workspace Cancel');
          const done = await h.waitTerminal(A.jobId, 15000);
          clearInterval(watch);
          return { seen, phases: done.phases, phase: done.job?.phase, text: done.text };
        """, jobId=started['jobId'])
        note('cancelled', {**cancelled, 'text': clip(cancelled['text'], 400)})
        check('cancel passes through cancelling to cancelled', cancelled['phase'] == 'cancelled' and
              ('cancelling' in cancelled['phases'] or 'cancelling' in cancelled['seen']), cancelled['phases'])
        check('cancelled state offers no result', 'Comparison cancelled' in cancelled['text'])
        left = ev("return await h.clearJobs();")
        check('remaining jobs cancel and dismiss', left == 0, left)
        time.sleep(1.0)
        after = busy()
        note('backendsAfterCancel', after)
        check('no comparison backend is left behind while the lock is still held', after == '', after)
    finally:
        release_lock()


def concurrent_ddl():
    fresh()
    # DDL commits inside the two-second lock wait: the capture must not mix states.
    hold_lock(0.6, extra="SELECT pg_sleep(0.4); ALTER TABLE src.lock_me RENAME COLUMN renamed_later TO renamed_now; "
                         "CREATE TABLE src.added_during_wait (id integer); DROP TABLE src.only_in_source;")
    try:
        result = compare(MAIN)
        note('ddl', {'ms': result['ms'], 'phases': result['phases'], 'failure': result['job'].get('failure')})
        check('comparison completes after the DDL commits', result['job']['phase'] == 'completed', result['job'])
        objects = ev("return h.objects();")
        columns = ev("await h.object('lock_me'); return [...new Set(h.fieldRows().filter((r) => r[0].startsWith('column ')).map((r) => r[0].split(' ')[1]))];")
        note('objects', objects); note('lockMeColumns', columns)
        check('result reflects the committed DDL, not a mixed capture',
              'added_during_wait' in objects and 'only_in_source' not in objects and 'renamed_now' in columns
              and objects.get('lock_me') == 'Changed', (objects, columns))
    finally:
        release_lock()
        psql("ALTER TABLE src.lock_me RENAME COLUMN renamed_now TO renamed_later; DROP TABLE src.added_during_wait; "
             "CREATE TABLE src.only_in_source (id integer);")
    # A table that stays locked past both lock waits: no result, deliberate rerun.
    ev("await h.clearJobs();")
    hold_lock(6)
    try:
        locked = compare(MAIN)
        note('locked', {'ms': locked['ms'], 'phases': locked['phases'], 'failure': locked['job'].get('failure'), 'text': clip(locked['text'], 500)})
        check('a lock held past the wait is reported, not presented as a result',
              locked['job']['phase'] == 'failed' and locked['job']['failure']['kind'] == 'captureChanged', locked['job'])
        check('the message covers a lock and offers a deliberate rerun', 'stayed locked' in locked['text'] and 'Run again' in locked['text'],
              clip(locked['text']))
    finally:
        release_lock()
    time.sleep(0.5)
    rerun = ev(r"""
      const { observer } = await h.mods();
      const before = new Set(observer.store.getState().jobs.map((j) => j.jobId));
      h.click(h.byText('button', 'Run again', h.root()), 'Run again');
      const job = await h.until(() => observer.store.getState().jobs.find((j) => !before.has(j.jobId)));
      const done = await h.waitTerminal(job.jobId);
      await h.until(() => h.byLabel('Compared objects'), 15000);
      return { phase: done.job.phase, source: done.job.source, target: done.job.target, objects: Object.keys(h.objects()).length };
    """)
    note('rerun', rerun)
    check('rerun uses the failed job endpoints and completes', rerun['phase'] == 'completed' and rerun['source']['schema'] == 'src'
          and rerun['target']['schema'] == 'tgt', rerun)


def disconnect_reconnect():
    fresh()
    ev("const { store } = await h.mods(); if (store.getState().connections.find((c) => c.id === 'gate-other').status !== 'Connected') await store.getState().connectConnection('gate-other');")
    result = compare(['gate-main', 'src', 'gate-other', 'src'])
    check('result open before disconnect', result['job']['phase'] == 'completed')
    ev("await h.object('orders');")
    gone = ev(r"""
      const { store, observer } = await h.mods();
      window.__trace.reset();
      await store.getState().disconnectConnection('gate-other');
      await h.sleep(1500);
      const text = h.text();
      return { text, census: h.census(), jobs: observer.store.getState().jobs.map((j) => j.phase), trace: h.traceSummary() };
    """)
    note('afterDisconnect', {**gone, 'text': clip(gone['text'], 500)})
    check('disconnecting an endpoint drops the retained pages',
          'unavailable' in gone['text'] and gone['census']['fieldRows'] == 0 and gone['census']['objectRows'] == 0, gone['census'])
    again = ev("const { store } = await h.mods(); await store.getState().connectConnection('gate-other'); await h.clearJobs(); return await h.compare(A.endpoints);",
               endpoints=['gate-main', 'src', 'gate-other', 'src'])
    check('a fresh comparison works after reconnect', again['job']['phase'] == 'completed', again.get('job'))

    # Disconnect while a job waits on a lock.
    ev("await h.clearJobs();")
    hold_lock()
    try:
        started = compare(['gate-other', 'src', 'gate-main', 'src'], wait=False)
        ev("await h.sleep(1200);")
        active = ev(r"""
          const { store } = await h.mods();
          await store.getState().disconnectConnection('gate-other');
          const done = await h.waitTerminal(A.jobId, 20000);
          return { phases: done.phases, job: done.job, mounted: !!h.root(), text: h.text() };
        """, jobId=started['jobId'])
        note('activeDisconnect', {**active, 'text': clip(active['text'], 500)})
        check('disconnect ends the active job', active['job'] is None or active['job']['phase'] in ('cancelled', 'failed'), active['job'])
        time.sleep(1.0)
        left = {'cmp_other': busy('cmp_other'), 'cmp_main': busy('cmp_main')}
        check('no comparison backend is left on either endpoint', left == {'cmp_other': '', 'cmp_main': ''}, left)
    finally:
        release_lock()
    ev("const { store } = await h.mods(); await store.getState().connectConnection('gate-other'); await h.clearJobs();")

    # Edit, then delete, a connection used by the selected result.
    edited = ev(r"""
      const { store } = await h.mods();
      await h.compare(['gate-main', 'src', 'gate-other', 'src']);
      await h.object('orders');
      const other = store.getState().connections.find((c) => c.id === 'gate-other');
      const outcome = await store.getState().updateConnection({ ...other, name: 'gate-other-edited' });
      await h.sleep(1500);
      return { outcome, text: h.text(), census: h.census() };
    """)
    note('afterEdit', {**edited, 'text': clip(edited['text'], 500)})
    check('editing an endpoint connection invalidates the result', edited['outcome'] == 'saved' and 'unavailable' in edited['text']
          and edited['census']['fieldRows'] == 0, edited['census'])
    deleted = ev(r"""
      const { store } = await h.mods();
      const other = store.getState().connections.find((c) => c.id === 'gate-other');
      await store.getState().updateConnection({ ...other, name: 'gate-other' });
      await store.getState().addConnection({ ...other, id: 'gate-temp', name: 'gate-temp', status: 'Disconnected' });
      store.getState().setActiveConnectionId('gate-main');
      await h.open('gate-main');
      await h.clearJobs();
      const result = await h.compare(['gate-main', 'src', 'gate-temp', 'src']);
      await h.object('orders');
      await store.getState().deleteConnection('gate-temp');
      await h.sleep(1500);
      return { phase: result.job?.phase, formError: result.formError, text: h.text(), census: h.census() };
    """)
    note('afterDelete', {**deleted, 'text': clip(deleted['text'], 500)})
    check('deleting an endpoint connection drops its result at once',
          deleted['phase'] == 'completed' and 'unavailable' in deleted['text'] and deleted['census']['fieldRows'] == 0, deleted['census'])
    ev("await h.open('gate-main'); await h.clearJobs();")


def reload_document():
    fresh()
    first = compare(MAIN)
    check('result exists before reload', first['job']['phase'] == 'completed')
    hold_lock()
    try:
        waiting = compare(['gate-main', 'src', 'gate-main', 'same_a'], wait=False)
        ev("await h.sleep(800);")
        hello = json.load(urllib.request.urlopen(DRIVER + '/status'))['hello']['at']
        try:
            raw('location.reload()', timeout=3)
        except Exception:
            pass
        for _ in range(200):
            time.sleep(0.25)
            status = json.load(urllib.request.urlopen(DRIVER + '/status'))
            if status['hello'] and status['hello']['at'] != hello:
                break
        else:
            raise RuntimeError('page did not come back after reload')
        time.sleep(1.0)
        ensure()
        after = ev(r"""
          const { store, observer } = await h.mods();
          await h.until(() => store.getState().connections.length >= 4);
          const statuses = store.getState().connections.map((c) => `${c.id}:${c.status}`);
          await h.until(() => observer.store.getState().observedAt !== null);
          const beforeOpen = observer.store.getState().jobs.map((j) => `${j.source.schema}->${j.target.schema}:${j.phase}`);
          await h.open('gate-main');
          await h.sleep(900);
          return { statuses, beforeOpen, jobs: observer.store.getState().jobs.map((j) => `${j.source.schema}->${j.target.schema}:${j.phase}`), text: h.text() };
        """)
        note('afterReload', {**after, 'text': clip(after['text'], 600)})
        check('native jobs are reconciled after reload',
              any(j.startswith('src->tgt:completed') for j in after['jobs']) and any(j.startswith('src->same_a:') for j in after['jobs']), after['jobs'])
        check('no job or result id was persisted to select automatically', 'Compare two schemas' in after['text'] or 'Session comparisons · 2' in after['text'])
    finally:
        release_lock()
    done = ev(r"""
      const { observer } = await h.mods();
      const job = observer.store.getState().jobs.find((j) => j.target.schema === 'same_a');
      const finished = await h.waitTerminal(job.jobId, 70000);
      window.__trace.reset();
      await h.openResult('src', 'tgt');
      await h.object('orders');
      const field = await h.field('column amount default');
      return { phase: finished.job?.phase, failure: finished.job?.failure, objects: Object.keys(h.objects()).length, field: field.row, trace: h.traceSummary() };
    """, jobId=waiting['jobId'])
    note('afterRelease', done)
    check('the job that was active across reload still finishes', done['phase'] == 'completed', done)
    check('the earlier result is readable with a new transport token',
          done['objects'] >= 9 and done['field'][3] == 'Changed' and done['trace']['unackedNow'] == 0
          and all(v['failed'] == 0 for v in done['trace']['by'].values()), done['trace'])


def view_switching():
    fresh()
    result = compare(['gate-main', 'many_src', 'gate-main', 'many_tgt'])
    note('ms', result['ms']); note('phases', result['phases'])
    check('1,000 tables per side complete', result['job']['phase'] == 'completed', result['job'])
    check('first page is the 100-item cap', '1–100 of' in result['text'], clip(result['text'], 300))
    out = ev(r"""
      window.__trace.reset();
      const errors = [];
      for (let round = 0; round < 20; round++) {
        // Start a read, leave the view before it lands, and come back.
        h.byLabel('Next object page', h.root())?.click();
        if (round % 2) await h.sleep(round);
        await h.rail('Tables');
        await h.until(() => !h.root());
        await h.sleep(round % 3 ? 0 : 40);
        await h.rail('Schema compare');
        await h.until(() => h.byLabel('Compared objects'), 15000);
        await h.settled();
        if (/unavailable|could not|failed/i.test(h.byLabel('Compared objects').innerText)) errors.push(round);
        if (!h.text().includes('1–100 of')) errors.push(`page ${round}`);
      }
      // Rapid in-view switching: only the latest intent may apply.
      const names = Object.keys(h.objects());
      const buttons = [...h.byLabel('Compared objects').querySelectorAll('li button')];
      for (const button of buttons.slice(0, 30)) button.click();
      await h.until(() => h.byLabel('Selected object fields')?.innerText.includes(names[29]), 15000);
      await h.settled();
      const header = h.byLabel('Selected object fields').innerText.split('\n')[0];
      await h.sleep(300);
      return { errors, header, expected: names[29], census: h.census(), trace: h.traceSummary() };
    """, timeout=180)
    note('result', out)
    check('leaving and returning during reads never shows an error or a stale page', out['errors'] == [], out['errors'])
    check('only the latest selection is shown', out['expected'] in out['header'], (out['header'], out['expected']))
    check('every delivered read was acknowledged', out['trace']['unackedNow'] == 0 and
          all(v['failed'] == 0 for v in out['trace']['by'].values()), out['trace'])
    check('skipped selections were not all read', out['trace']['by'].get('read:fields', {}).get('n', 0) < 30, out['trace']['by'].get('read:fields'))


def tunnel():
    fresh()
    bastion = FIXTURES['bastion']['name']
    connected = ev(r"""
      const { store } = await h.mods();
      await store.getState().connectConnection('gate-tunnel');
      const c = store.getState().connections.find((x) => x.id === 'gate-tunnel');
      return { status: c.status, error: c.errorMessage ?? null };
    """)
    note('connect', connected)
    if not check('tunnel connection connects through the bastion', connected['status'] == 'Connected', connected):
        return
    result = compare(['gate-tunnel', 'src', 'gate-main', 'tgt'])
    check('comparison through the tunnel completes', result.get('job', {}).get('phase') == 'completed', result.get('job') or result.get('formError'))
    ev("await h.object('orders');")
    hold_lock()
    try:
        waiting = compare(['gate-tunnel', 'src', 'gate-main', 'same_a'], wait=False)
        ev("await h.sleep(1200);")
        # End the forwarding sessions; the bastion itself keeps its port.
        sh('docker', 'exec', bastion, 'sh', '-c', 'pkill -u tunnel; pkill -f "[s]shd-session"; pkill -f "[s]shd: tunnel"; true')
        ended = ev("return await h.waitTerminal(A.jobId, 75000);", jobId=waiting['jobId'], timeout=100)
        note('teardown', {'phases': ended['phases'], 'job': ended['job'], 'text': clip(ended['text'], 500)})
        check('tunnel loss ends the active job with a failure, not a result',
              ended['job'] is not None and ended['job']['phase'] in ('failed', 'cancelled'), ended['job'])
    finally:
        release_lock()
    time.sleep(1.5)
    check('no comparison backend is left behind after tunnel loss', busy() == '', busy())
    retained = ev(r"""
      await h.selectJob('src', 'tgt');
      await h.sleep(500);
      const text = h.text();
      let field = null;
      if (h.byLabel('Compared objects')) { await h.object('orders'); field = (await h.field('column amount default')).row; }
      return { text, field };
    """)
    note('retainedAfterTeardown', {'field': retained['field'], 'text': clip(retained['text'], 400)})
    check('the earlier capture is either readable as captured or plainly unavailable',
          (retained['field'] is not None and retained['field'][3] == 'Changed') or 'unavailable' in retained['text'], retained['field'])
    recovered = ev(r"""
      const { store } = await h.mods();
      await store.getState().disconnectConnection('gate-tunnel');
      await store.getState().connectConnection('gate-tunnel');
      await h.clearJobs();
      const result = await h.compare(['gate-tunnel', 'src', 'gate-main', 'tgt']);
      await store.getState().disconnectConnection('gate-tunnel');
      await h.sleep(1200);
      return { phase: result.job?.phase, formError: result.formError ?? null, afterDisconnect: h.text() };
    """, timeout=120)
    note('recovered', {**recovered, 'afterDisconnect': clip(recovered['afterDisconnect'], 400)})
    check('a new tunnel session compares again', recovered['phase'] == 'completed', recovered)
    check('closing the tunnel connection invalidates its result', 'unavailable' in recovered['afterDisconnect'])
    ev("await h.clearJobs();")


def visibility():
    fresh()
    hidden = ev(r"""
      const { observer, client } = await h.mods();
      const lists = () => window.__trace.calls.filter((c) => c.command === 'lists').length;
      await h.window('minimize');
      await h.until(() => document.visibilityState === 'hidden', 8000);
      window.__trace.reset();
      const started = await h.compare(A.endpoints, { wait: false });
      await h.sleep(5000);
      return { jobId: started.jobId, listsWhileHidden: lists(), observed: observer.store.getState().jobs.at(-1).phase,
               native: (await client.get(started.jobId)).phase, text: h.text() };
    """, endpoints=MAIN)
    resumed = ev(r"""
      const lists = () => window.__trace.calls.filter((c) => c.command === 'lists').length;
      const before = lists();
      const started = performance.now();
      await h.window('unminimize');
      await h.window('set_focus');
      await h.until(() => document.visibilityState === 'visible', 8000);
      const visibleAt = performance.now();
      await h.until(() => lists() > before, 5000, 5);
      const listAfterVisibleMs = Math.round(performance.now() - visibleAt);
      await h.until(() => h.byLabel('Compared objects'), 15000);
      return { listAfterVisibleMs, resultAfterVisibleMs: Math.round(performance.now() - visibleAt), objects: Object.keys(h.objects()).length };
    """)
    note('hidden', {**hidden, 'text': clip(hidden['text'], 300)}); note('resumed', resumed)
    check('a hidden window stops observing after the start reconciliation', hidden['listsWhileHidden'] <= 1, hidden['listsWhileHidden'])
    check('the native job finishes without the view', hidden['native'] == 'completed' and hidden['observed'] != 'completed',
          (hidden['native'], hidden['observed']))
    check('returning to the window reconciles at once and opens the result',
          resumed['listAfterVisibleMs'] < 500 and resumed['objects'] >= 9, resumed)


def expiry():
    """Opt-in: waits out the native ten-minute result lifetime."""
    fresh()
    compare(MAIN)
    ev("await h.object('orders');")
    waited = 0
    while waited < 660:
        time.sleep(30)
        waited += 30
        ensure()
    after = ev(r"""
      const { observer } = await h.mods();
      const jobs = observer.store.getState().jobs.map((j) => j.phase);
      const before = h.text();
      let clicked = null;
      if (h.byLabel('Compared objects')) {
        [...h.byLabel('Compared objects').querySelectorAll('li button')].find((b) => b.firstElementChild.textContent === 'equal_table').click();
        await h.sleep(1500);
        clicked = h.text();
      }
      return { jobs, before, clicked, census: h.census() };
    """)
    note('afterTtl', {'jobs': after['jobs'], 'before': clip(after['before'], 400), 'clicked': clip(after['clicked'] or '', 400), 'census': after['census']})
    text = after['clicked'] or after['before']
    check('an expired result reads as unavailable and keeps no pages', 'unavailable' in text and after['census']['fieldRows'] == 0
          and after['census']['objectRows'] == 0, after['census'])


def boundedness():
    fresh()
    ev("await h.rail('Tables'); await h.sleep(1500);")
    samples = [memory('before, workspace closed')]
    ev("await h.rail('Schema compare'); await h.until(() => h.root());")
    many = compare(['gate-main', 'many_src', 'gate-main', 'many_tgt'])
    wide = compare(['gate-main', 'wide_src', 'gate-main', 'wide_tgt'])
    check('cap-sized comparisons complete', many['job']['phase'] == 'completed' and wide['job']['phase'] == 'completed')
    note('compareMs', {'many': many['ms'], 'wide': wide['ms']})
    samples.append(memory('two results retained natively'))
    loop_a = r"""
      const census = [], times = [];
      const timed = async (label, run) => { const t = performance.now(); await run(); times.push([label, Math.round(performance.now() - t)]); };
      h.frames.start();
      for (let cycle = 0; cycle < A.cycles; cycle++) {
        await timed('job switch', () => h.openResult('many_src', 'many_tgt'));
        await timed('object next', () => h.page('object', 'Next'));
        await timed('object next', () => h.page('object', 'Next'));
        const names = Object.keys(h.objects());
        await timed('object select', () => h.object(names[cycle % names.length]));
        await timed('object select', () => h.object(names[(cycle * 7 + 3) % names.length]));
        census.push({ at: 'many p3', ...h.census() });
        await timed('object previous', () => h.page('object', 'Previous'));
        await timed('object previous', () => h.page('object', 'Previous'));
        await timed('job switch', () => h.openResult('wide_src', 'wide_tgt'));
        await timed('object select', () => h.object('wide'));
        for (let i = 0; i < 3; i++) await timed('field next', () => h.page('field', 'Next'));
        const rows = h.fieldRows();
        await timed('field select', () => h.field(rows[(cycle * 11) % rows.length][0], false));
        census.push({ at: 'wide f4', ...h.census() });
        for (let i = 0; i < 3; i++) await timed('field previous', () => h.page('field', 'Previous'));
      }
      return { census, times, frames: h.frames.stop(), position: h.text().match(/Fields [^\n]+/)?.[0] };
    """
    loop_b = r"""
      const census = [], times = [];
      const timed = async (label, run) => { const t = performance.now(); await run(); times.push([label, Math.round(performance.now() - t)]); };
      h.frames.start();
      for (let cycle = 0; cycle < A.cycles; cycle++) {
        await timed('job switch', () => h.openResult('src', 'tgt'));
        await timed('object select', () => h.object('big_values'));
        await timed('value select', () => h.field('table comment', false));
        for (let i = 0; i < 3; i++) await timed('chunk next', () => h.chunk('Source', 'Next'));
        census.push({ at: 'big last chunk', ...h.census() });
        for (let i = 0; i < 3; i++) await timed('chunk previous', () => h.chunk('Source', 'Previous'));
        await timed('chunk next', () => h.chunk('Target', 'Next'));
        await timed('value select', () => h.field('column id comment', false));
        await timed('chunk next', () => h.chunk('Source', 'Next'));
        census.push({ at: 'big id chunk 2', ...h.census() });
        await timed('job switch', () => h.openResult('wide_src', 'wide_tgt'));
        await timed('object select', () => h.object('wide'));
        await timed('field select', () => h.field(h.fieldRows()[cycle % 100][0], false));
      }
      return { census, times, frames: h.frames.stop() };
    """
    results = {}
    for name, loop, rounds in (('pages', loop_a, 4), ('values', loop_b, 4)):
        if name == 'values':
            ev("const { observer } = await h.mods();"
               "const row = h.jobRows()[observer.store.getState().jobs.findIndex((j) => j.source.schema === 'many_src')];"
               "[...row.querySelectorAll('button')].find((b) => b.textContent.trim() === 'Dismiss').click(); await h.sleep(800);")
            big = compare(MAIN)
            check('large-value comparison completes', big['job']['phase'] == 'completed')
        ev("window.__trace.reset();")
        census, times, frames = [], [], []
        for round_number in range(rounds):
            part = ev(loop, cycles=6, timeout=600)
            census += part['census']; times += part['times']; frames.append(part['frames'])
            samples.append(memory(f'{name}: after {(round_number + 1) * 6} cycles'))
            print('   mem', mem_line(samples[-1]), flush=True)
        trace = ev("return h.traceSummary();")
        by_state = {}
        for row in census:
            by_state.setdefault(row.pop('at'), []).append(row)
        stats = {}
        for label in sorted({t[0] for t in times}):
            values = sorted(t[1] for t in times if t[0] == label)
            stats[label] = {'n': len(values), 'p50': values[len(values) // 2], 'p95': values[int(len(values) * 0.95) - 1], 'max': values[-1]}
        results[name] = {'cycles': rounds * 6, 'trace': trace, 'frames': frames, 'interactionMs': stats,
                         'census': {state: {'n': len(rows), 'domNodes': sorted({r['domNodes'] for r in rows}),
                                            'objectRows': sorted({r['objectRows'] for r in rows}),
                                            'fieldRows': sorted({r['fieldRows'] for r in rows}),
                                            'valueBlocks': sorted({r['valueBlocks'] for r in rows}),
                                            'maxValueChars': max(r['valueChars'] for r in rows)}
                                    for state, rows in by_state.items()}}
        check(f'{name}: 24 cycles, at most one outstanding read, all acknowledged',
              trace['maxInflightReads'] == 1 and trace['unackedNow'] == 0 and trace['maxUnacked'] <= 1
              and all(v['failed'] == 0 for v in trace['by'].values()), {k: trace[k] for k in ('maxInflightReads', 'unackedNow', 'maxUnacked')})
        pages = [v['maxBytes'] for k, v in trace['by'].items() if k.startswith('read:')]
        check(f'{name}: no response exceeds the 1 MiB page bound', max(pages) <= 1024 * 1024, max(pages))
        check(f'{name}: retained rows stay at one page', all(max(s['objectRows']) <= 100 and max(s['fieldRows']) <= 100 and max(s['valueBlocks']) <= 2
                                                           for s in results[name]['census'].values()), results[name]['census'])
    note('loops', results)
    cleanup = ev("const left = await h.clearJobs(); await h.rail('Tables'); await h.until(() => !h.root()); return left;")
    check('every job dismisses', cleanup == 0, cleanup)
    for wait, label in ((5, 'after cleanup, 5 s'), (30, 'after cleanup, 35 s')):
        time.sleep(wait if wait == 5 else 30)
        samples.append(memory(label))
        print('   mem', mem_line(samples[-1]), flush=True)
    note('memory', samples)
    for process in ('native', 'WebContent'):
        loop_rows = [s[process]['footprintMiB'] for s in samples if 'cycles' in s['label'] and process in s]
        note(f'{process}FootprintMiB', {'before': samples[0][process]['footprintMiB'], 'loop': loop_rows,
                                        'afterCleanup': samples[-1][process]['footprintMiB']})
    ev("await h.rail('Schema compare'); await h.until(() => h.root());")


def layout_theme_keyboard():
    fresh()
    compare(MAIN)
    ev("await h.object('orders'); await h.field('column amount default');")
    measure = r"""
      const rect = (el) => { if (!el) return null; const r = el.getBoundingClientRect(); return { x: Math.round(r.x), y: Math.round(r.y), w: Math.round(r.width), h: Math.round(r.height) }; };
      const root = h.root();
      const objects = h.byLabel('Compared objects').parentElement, fields = h.byLabel('Selected object fields');
      const source = h.pane('Source'), target = h.pane('Target');
      const scroller = fields.querySelector('table').parentElement;
      const identities = [...root.querySelectorAll('.bg-surface-sidebar .inline-flex')].map(rect);
      return {
        viewport: [innerWidth, innerHeight], root: rect(root), objects: rect(objects), fields: rect(fields),
        source: rect(source), target: rect(target),
        stackedPanes: rect(objects).y + rect(objects).h <= rect(fields).y + 1,
        stackedValues: rect(source).y + rect(source).h <= rect(target).y + 1,
        pageOverflowX: document.documentElement.scrollWidth - innerWidth,
        rootOverflowX: root.scrollWidth - root.clientWidth,
        tableScrollsInside: [scroller.scrollWidth, scroller.clientWidth],
        identitiesInView: identities.every((r) => r && r.x >= 0 && r.x + r.w <= innerWidth && r.w > 0),
        compareInView: (() => { const r = rect(h.compareButton()); return r.x + r.w <= innerWidth && r.y + r.h <= innerHeight; })(),
        jobListInView: (() => { const r = rect([...root.querySelectorAll('details')].at(-1).querySelector('summary')); return r.y + r.h <= innerHeight + 1; })(),
        selectedObject: h.byLabel('Compared objects').querySelector('[aria-pressed=true]')?.innerText.split('\n')[0],
        position: h.text().match(/\d+–\d+ of \d+/)?.[0],
        fonts: { object: getComputedStyle(h.byLabel('Compared objects').querySelector('li button')).fontSize,
                 field: getComputedStyle(fields.querySelector('tbody td')).fontSize,
                 value: getComputedStyle(source.querySelector('.font-mono')).fontSize },
        rows: { tree: rect(h.byLabel('Compared objects').querySelector('li button')).h, grid: rect(fields.querySelector('tbody tr')).h,
                toolbar: rect(root.querySelector('header')).h },
      };
    """
    sizes = {}
    ev("await h.window('set_min_size', { value: { Logical: { width: 640, height: 480 } } });")
    for label, width, height in (('default 1200x800', 1200, 800), ('minimum 900x560', 900, 560), ('narrow 700x560', 700, 560)):
        ev("await h.window('set_size', { value: { Logical: { width: A.w, height: A.h } } }); await h.sleep(700);", w=width, h=height)
        sizes[label] = ev(measure)
        m = sizes[label]
        check(f'{label}: no page-level horizontal overflow', m['pageOverflowX'] <= 0 and m['rootOverflowX'] <= 0, (m['pageOverflowX'], m['rootOverflowX']))
        check(f'{label}: endpoints, Compare and the job list stay reachable', m['identitiesInView'] and m['compareInView'] and m['jobListInView'], m)
        check(f'{label}: selection and page survive the layout change', m['selectedObject'] == 'orders' and m['position'] is not None, (m['selectedObject'], m['position']))
    note('sizes', sizes)
    check('wide layout puts the object list beside the inspector at 260px', not sizes['default 1200x800']['stackedPanes'] and sizes['default 1200x800']['objects']['w'] == 260,
          sizes['default 1200x800']['objects'])
    check('narrow layout stacks the object list above the inspector', sizes['narrow 700x560']['stackedPanes'], sizes['narrow 700x560']['objects'])
    check('text size is the same at every width', len({json.dumps(m['fonts']) for m in sizes.values()}) == 1, [m['fonts'] for m in sizes.values()])
    ev("await h.window('set_size', { value: { Logical: { width: 1200, height: 800 } } });"
       "await h.window('set_min_size', { value: { Logical: { width: 900, height: 560 } } }); await h.sleep(600);")

    # Theme and density mirrors live in WebView storage shared with every dev run
    # of this binary: put back whatever was there.
    before = ev("const { store, density } = await h.mods(); return { density: density.loadDensity(), theme: store.getState().appSettings?.theme ?? 'system' };")
    density = {}
    for mode in ('compact', 'default', 'comfortable'):
        ev("(await h.mods()).density.setDensity(A.mode); await h.sleep(300);", mode=mode)
        m = ev(measure)
        density[mode] = {'fonts': m['fonts'], 'rows': m['rows']}
    ev("(await h.mods()).density.setDensity(A.mode);", mode=before['density'])
    note('density', density)
    check('density changes spacing only', len({json.dumps(d['fonts']) for d in density.values()}) == 1
          and density['compact']['rows']['tree'] < density['default']['rows']['tree'] < density['comfortable']['rows']['tree'], density)

    colors = r"""
      const { store } = await h.mods();
      await store.getState().setTheme(A.mode);
      await h.sleep(500);
      const canvas = document.createElement('canvas').getContext('2d', { willReadFrequently: true });
      const rgb = (css) => { canvas.clearRect(0, 0, 1, 1); canvas.fillStyle = '#000'; canvas.fillStyle = css; canvas.fillRect(0, 0, 1, 1); const d = canvas.getImageData(0, 0, 1, 1).data; return [d[0], d[1], d[2], d[3] / 255]; };
      const over = (top, bottom) => top.slice(0, 3).map((c, i) => Math.round(c * top[3] + bottom[i] * (1 - top[3])));
      const background = (el) => { const layers = []; for (let n = el; n; n = n.parentElement) { const c = rgb(getComputedStyle(n).backgroundColor); if (c[3] > 0) { layers.push(c); if (c[3] === 1) break; } } let base = [255, 255, 255]; for (const layer of layers.reverse()) base = over(layer, base); return base; };
      const lum = (c) => { const [r, g, b] = c.map((v) => { v /= 255; return v <= 0.03928 ? v / 12.92 : ((v + 0.055) / 1.055) ** 2.4; }); return 0.2126 * r + 0.7152 * g + 0.0722 * b; };
      const contrast = (el) => { if (!el) return null; const bg = background(el); const fg = over(rgb(getComputedStyle(el).color), bg); const [a, b] = [lum(fg), lum(bg)].sort((x, y) => y - x); return { fg, bg, ratio: Math.round(((a + 0.05) / (b + 0.05)) * 100) / 100 }; };
      const root = h.root(), fields = h.byLabel('Selected object fields');
      const pick = (scope, cls) => scope.querySelector(cls);
      return {
        dark: document.documentElement.classList.contains('dark'),
        workspaceBackground: background(root), workspaceColor: rgb(getComputedStyle(root).color).slice(0, 3),
        samples: {
          objectName: contrast(h.byLabel('Compared objects').querySelector('li button:not([aria-pressed=true]) span')),
          selectedObject: contrast(h.byLabel('Compared objects').querySelector('[aria-pressed=true] span')),
          selectedField: contrast(fields.querySelector('tbody [aria-pressed=true]')),
          fieldGroupMuted: contrast(fields.querySelector('tbody .text-text-muted')),
          columnHeader: contrast(fields.querySelector('thead th')),
          value: contrast(h.pane('Source').querySelector('pre')),
          valueLabel: contrast(h.pane('Source').firstElementChild.firstElementChild),
          subtitle: contrast(root.querySelector('header span')),
          changedBadge: contrast([...fields.querySelectorAll('tbody td:last-child span')].find((s) => s.textContent === 'Changed')),
          reason: contrast(pick(fields, 'tbody td:last-child .text-text-muted')),
        },
      };
    """
    themes = {mode: ev(colors, mode=mode) for mode in ('dark', 'light')}
    ev("const { store } = await h.mods(); await store.getState().setTheme(A.mode);", mode=before['theme'])
    note('themes', themes)
    dark, light = themes['dark'], themes['light']
    check('dark mode uses the true-black surface with white text', dark['dark'] and dark['workspaceBackground'] == [0, 0, 0]
          and dark['workspaceColor'] == [255, 255, 255], (dark['workspaceBackground'], dark['workspaceColor']))
    check('light mode keeps the theme surface', not light['dark'] and min(light['workspaceBackground']) >= 230, light['workspaceBackground'])
    for mode, theme in themes.items():
        ratios = {k: v['ratio'] for k, v in theme['samples'].items() if v}
        check(f'{mode}: names and captured values meet 4.5:1 contrast', ratios['objectName'] >= 4.5 and ratios['value'] >= 4.5, ratios)
        # Muted, accent and warning text come from shared theme tokens; record them.
        note(f'{mode}Below4.5', {k: v for k, v in ratios.items() if v < 4.5})

    keyboard = ev(r"""
      const root = h.root();
      // Content of a collapsed <details> is deliberately out of the tab order.
      const collapsed = (el) => el.tagName !== 'SUMMARY' && el.closest('details:not([open])');
      const controls = [...root.querySelectorAll('button, select, input, summary, a[href], [tabindex]')].filter((el) => el.offsetParent !== null && !collapsed(el));
      const name = (el) => (el.getAttribute('aria-label') || el.innerText || el.getAttribute('placeholder') || '').trim().split('\n')[0];
      const problems = [];
      const order = [];
      for (const el of controls) {
        const tag = el.tagName.toLowerCase();
        if (!['button', 'select', 'input', 'summary', 'a'].includes(tag)) problems.push(`non-native ${tag}`);
        if (el.tabIndex !== 0) problems.push(`tabindex ${el.tabIndex} on ${name(el)}`);
        if (!name(el)) problems.push(`unnamed ${tag}`);
        if (!el.disabled) { el.focus(); if (document.activeElement !== el) problems.push(`not focusable: ${name(el)}`); }
        if (tag !== 'input' && tag !== 'summary' && !/focus-visible:/.test(el.className)) problems.push(`no focus style: ${name(el)}`);
        order.push(`${tag}:${name(el).slice(0, 28)}`);
      }
      document.activeElement?.blur();
      const positions = controls.map((el) => { const r = el.getBoundingClientRect(); return [Math.round(r.y), Math.round(r.x)]; });
      return { count: controls.length, problems: [...new Set(problems)], first: order.slice(0, 9), pressed: root.querySelectorAll('[aria-pressed=true]').length,
               labelled: ['Compared objects', 'Selected object fields', 'Value inspector', 'Source value', 'Target value'].every((l) => h.byLabel(l)) };
    """)
    note('keyboard', keyboard)
    check('every control is a native, named, focusable element in document order', keyboard['problems'] == [], keyboard['problems'])
    check('regions and selection state are exposed to assistive technology', keyboard['labelled'] and keyboard['pressed'] >= 3, keyboard['pressed'])


SCENARIOS = [same_connection, chunks, independent_databases, minor_versions, refusals, cancel_and_busy,
             concurrent_ddl, disconnect_reconnect, reload_document, view_switching, tunnel, visibility,
             boundedness, layout_theme_keyboard]
OPT_IN = [expiry]  # Slow; run by name.


def main():
    global current
    wanted = sys.argv[1:]
    path = WORK / 'report.json'
    if path.exists():
        report.update(json.loads(path.read_text()))
    ensure()
    report['environment'] = {
        'servers': {role: fixture.get('version') or fixture['image'] for role, fixture in FIXTURES.items()},
        'userAgent': raw('navigator.userAgent')['value'], 'macOS': sh('sw_vers', '-productVersion'),
        'machine': sh('sysctl', '-n', 'machdep.cpu.brand_string'), 'window': raw('[innerWidth, innerHeight, devicePixelRatio]')['value'],
    }
    setup()
    for scenario in SCENARIOS + OPT_IN:
        if scenario.__name__ not in (wanted or [s.__name__ for s in SCENARIOS]):
            continue
        ensure()
        current = {'startedAt': time.strftime('%H:%M:%S')}
        print(f'== {scenario.__name__}', flush=True)
        try:
            scenario()
        except SystemExit:
            raise
        except Exception as error:  # A broken step is evidence too; keep going.
            check('scenario ran to the end', False, f'{type(error).__name__}: {str(error)[:600]}')
            release_lock()
        current['visibleAtEnd'] = raw('document.visibilityState')['value']
        report[scenario.__name__] = current
        path.write_text(json.dumps(report, indent=1, ensure_ascii=False))
    failed = [(name, c['check']) for name, body in report.items() if isinstance(body, dict)
              for c in body.get('checks', []) if not c['pass']]
    print(f"\n{sum(len(b.get('checks', [])) for b in report.values() if isinstance(b, dict))} checks, {len(failed)} failed")
    for name, text in failed:
        print(f'  FAIL {name}: {text}')


if __name__ == '__main__':
    main()
