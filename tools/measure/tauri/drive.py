#!/usr/bin/env python3
"""Plan 024: puts the isolated Tauri measurement build into a known state.

The build is the production frontend bundle with the Plan 022 evaluation
bridge, served by that plan's `serve.py` on port 3024:

    python3 infrastructure/test-db/schema-compare/webview-driver/serve.py \
        /tmp/dbunk-plan024/dist/public 3024 &
    DBUNK_DEV_CONFIG_DIR=/tmp/dbunk-plan024/config /tmp/dbunk-plan024/bin/dbunk-bridge &
    python3 tools/measure/tauri/drive.py setup

Only state is set up here. Input and timing come from `measure`, the same
way they do for the native spike.

Usage: drive.py setup | editor | eval '<js returning a value>'
"""
import json
from pathlib import Path
import subprocess
import sys
import urllib.request

ROOT = Path(__file__).resolve().parents[3]
DRIVER = 'http://127.0.0.1:3024/__dbunk'
CONNECTION = 'plan024'
PRELUDE = """
const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms));
const until = async (probe, ms = 20000) => {
  const end = performance.now() + ms;
  for (;;) {
    const value = await probe();
    if (value) return value;
    if (performance.now() > end) throw new Error('timeout waiting for ' + probe);
    await sleep(25);
  }
};
const { store, forms } = await window.__dbunkModules();
"""


def ev(body, timeout=60, **params):
    expr = '(async () => { const A = %s; %s %s })()' % (json.dumps(params), PRELUDE, body)
    request = urllib.request.Request(DRIVER + '/eval', data=expr.encode(),
                                     headers={'x-timeout-ms': str(timeout * 1000)})
    out = json.load(urllib.request.urlopen(request, timeout=timeout + 15))
    if not out.get('ok'):
        raise SystemExit('driver error: %s' % out.get('error'))
    return out.get('value')


def setup():
    settings = ev('await until(() => store.getState().appSettings); return store.getState().appSettings;')
    if not settings['onboardingCompleted']:
        # Onboarding clears the shared `dbunk` keychain entry whatever the
        # config directory is: never do that to a real one.
        present = subprocess.run(['security', 'find-generic-password', '-s', 'dbunk', '-a',
                                  'connection-credentials'], capture_output=True).returncode == 0
        if present:
            raise SystemExit('A dbunk keychain entry exists; onboarding here would delete it. Stop.')
        ev("if (!(await store.getState().configureCredentialStorage({ mode: 'plain-sqlite' })))"
           " throw new Error('onboarding failed');")
    return ev(r"""
      if (!store.getState().connections.find((c) => c.id === A.id)) {
        const parsed = forms.connectionSchema.parse({ ...forms.EMPTY_NEW_DEFAULTS, name: A.id,
          engine: 'PostgreSQL', host: '127.0.0.1', port: 15432, database: 'dbunk_demo',
          user: 'dbunk', password: 'dbunk', environment: 'development' });
        await store.getState().addConnection(
          forms.buildConnectionFromForm(parsed, A.id, { status: 'disconnected', latency: '' }));
      }
      store.getState().setActiveConnectionId(A.id);
      const status = () => store.getState().connections.find((c) => c.id === A.id).status;
      if (status() !== 'Connected') await store.getState().connectConnection(A.id);
      await until(() => status() === 'Connected');
      return store.getState().connections.map((c) => `${c.id}:${c.status}`);
    """, id=CONNECTION)


def editor():
    """One query tab holding the shared 2,000-line document."""
    text = (ROOT / 'tools/measure/fixtures/editor-2000.sql').read_text()
    return ev(r"""
      for (const tab of store.getState().workspaceTabs.filter((t) => t.kind === 'query'))
        await store.getState().closeTab(tab.id);
      store.getState().openWorkspaceTab({ kind: 'query', label: 'plan024.sql',
        connectionId: A.id, schema: 'public', query: A.text });
      await until(() => document.querySelector('.monaco-editor textarea'));
      await sleep(500);
      document.querySelector('.monaco-editor textarea').focus();
      return { tabs: store.getState().workspaceTabs.map((t) => `${t.kind}:${t.label}`),
               lines: document.querySelectorAll('.monaco-editor .view-line').length };
    """, id=CONNECTION, text=text)


if __name__ == '__main__':
    command = sys.argv[1] if len(sys.argv) > 1 else ''
    if command == 'setup':
        print(json.dumps(setup()))
    elif command == 'editor':
        print(json.dumps(editor()))
    elif command == 'eval':
        print(json.dumps(ev(sys.argv[2])))
    else:
        raise SystemExit(__doc__)
