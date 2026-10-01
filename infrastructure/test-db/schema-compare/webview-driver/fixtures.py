#!/usr/bin/env python3
"""Disposable fixtures for the Plan 022 native WebView walkthrough.

Usage: fixtures.py up | reseed | down | status

`up` creates newly named loopback-only containers with tmpfs data, seeds the
comparison schemas and writes their names and ports to a state file. `reseed`
drops and recreates the databases in those containers, keeping their ports.
`down` removes exactly the containers recorded there. No DSN or existing
container is accepted and no shared compose resource is used.

- primary: ``postgres:16``, databases ``cmp_main`` (base and bulk schemas) and
  ``cmp_other`` (base schemas, one changed default);
- minor: ``postgres:16.14``, database ``cmp_minor``;
- other: ``postgres:17``, database ``cmp_pg17``;
- bastion: an SSH server built from ``Dockerfile.bastion`` that reaches the
  primary over the container bridge, for the tunnel teardown scenario.
"""
import json
from pathlib import Path
import subprocess
import sys
import time
import uuid

HERE = Path(__file__).resolve().parent
STATE = Path('/tmp/dbunk-plan022-gate/fixtures.json')
LABEL = 'dbunk.fixture=schema-compare-plan022-gate'
BASTION_IMAGE = 'dbunk-plan022-gate-bastion:local'
SERVERS = [
    # role, image, databases
    ('primary', 'postgres:16', ['cmp_main', 'cmp_other']),
    ('minor', 'postgres:16.14', ['cmp_minor']),
    ('other', 'postgres:17', ['cmp_pg17']),
]


def docker(*args, stdin=None):
    return subprocess.run(['docker', *args], input=stdin, text=True, check=True,
                          capture_output=True).stdout.strip()


def psql(name, database, sql=None, path=None):
    args = ['exec', '-i', name, 'psql', '-X', '-q', '-v', 'ON_ERROR_STOP=1',
            '-U', 'postgres', '-d', database]
    if sql is not None:
        return docker(*args, '-Atc', sql)
    return docker(*args, stdin=Path(path).read_text())


def load():
    return json.loads(STATE.read_text())


def save(state):
    STATE.write_text(json.dumps(state, indent=1))


def create(role, image, state, port, *extra):
    name = f'dbunk-plan022-gate-{uuid.uuid4().hex[:12]}'
    print('Disposable fixture:', name, image, flush=True)
    docker('create', '--name', name, '--label', LABEL, '--publish', f'127.0.0.1::{port}',
           *extra, image)
    state[role] = {'name': name, 'image': image}
    save(state)
    docker('start', name)
    state[role]['port'] = int(docker('port', name, f'{port}/tcp').rsplit(':', 1)[1])
    return name


def seed(state):
    for role, _, databases in SERVERS:
        name = state[role]['name']
        for database in databases:
            psql(name, 'postgres', f'DROP DATABASE IF EXISTS {database} WITH (FORCE)')
            psql(name, 'postgres', f'CREATE DATABASE {database}')
            psql(name, database, path=HERE / 'base.sql')
    primary = state['primary']['name']
    psql(primary, 'cmp_main', path=HERE / 'bulk.sql')
    psql(primary, 'cmp_other', "ALTER TABLE src.orders ALTER COLUMN status SET DEFAULT 'other-db'")


def up():
    if STATE.exists():
        sys.exit(f'{STATE} exists; run down first')
    STATE.parent.mkdir(parents=True, exist_ok=True)
    state = {}
    try:
        for role, image, databases in SERVERS:
            name = create(role, image, state, 5432, '--tmpfs', '/var/lib/postgresql/data',
                          '--env', 'POSTGRES_HOST_AUTH_METHOD=trust')
            for _ in range(120):
                ready = subprocess.run(['docker', 'exec', name, 'pg_isready', '-h', '127.0.0.1',
                                        '-U', 'postgres'], capture_output=True)
                if ready.returncode == 0:
                    break
                time.sleep(0.25)
            else:
                raise RuntimeError(f'{name} did not become ready')
            state[role]['databases'] = databases
            state[role]['version'] = psql(name, 'postgres', 'SELECT version()')
        # The address the bastion uses to reach the primary over the bridge.
        state['primary']['bridgeAddress'] = docker(
            'inspect', '--format', '{{range .NetworkSettings.Networks}}{{.IPAddress}}{{end}}', state['primary']['name'])
        docker('build', '--quiet', '-t', BASTION_IMAGE, '-f', str(HERE / 'Dockerfile.bastion'),
               str(HERE))
        create('bastion', BASTION_IMAGE, state, 22)
        save(state)
        seed(state)
    except BaseException:
        down()
        raise
    print(json.dumps(state, indent=1))


def down():
    if not STATE.exists():
        return
    for fixture in load().values():
        docker('rm', '-f', fixture['name'])
        print('Removed:', fixture['name'], flush=True)
    STATE.unlink()


if __name__ == '__main__':
    command = sys.argv[1] if len(sys.argv) > 1 else ''
    if command == 'up':
        up()
    elif command == 'reseed':
        seed(load())
    elif command == 'down':
        down()
    elif command == 'status':
        print(STATE.read_text() if STATE.exists() else 'no fixtures')
    else:
        sys.exit(__doc__)
