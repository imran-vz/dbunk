#!/usr/bin/env python3
"""Run native comparison tests only in newly owned, disposable loopback fixtures.

No DSN or existing container is accepted. No shared compose resources are used.
Four containers are created and removed by this process:

- the PG16 baseline (``postgres:16``), primary endpoint for every native test;
- a second PG16 minor (``postgres:16.14``) for cross-minor coverage;
- a non-16 major (``postgres:17``) for version refusal;
- a TLS-enabled PG16 built from ``infrastructure/test-db/postgres-tls`` with
  host-generated throwaway certificates, for verification failure coverage.

The memory profile test runs in its own cargo invocation with one test thread
so its process-wide peak RSS is attributable to that test alone.
"""
import os
from pathlib import Path
import subprocess
import tempfile
import time
import uuid

ROOT = Path(__file__).resolve().parents[3]
TLS_DIR = ROOT / 'infrastructure/test-db/postgres-tls'
TLS_IMAGE = 'dbunk-schema-compare-tls:local'
CARGO_TARGET = '/tmp/dbunk-plan021-target'

FIXTURES = [
    # env variable, image, extra docker create arguments
    ('DBUNK_SCHEMA_COMPARE_TEST_PORT', 'postgres:16', []),
    ('DBUNK_SCHEMA_COMPARE_MINOR_PORT', 'postgres:16.14', []),
    ('DBUNK_SCHEMA_COMPARE_OTHER_PORT', 'postgres:17', []),
]


def docker(*args):
    return subprocess.check_output(['docker', *args], text=True).strip()


class Fixture:
    def __init__(self, image, extra):
        self.name = 'dbunk-schema-compare-native-' + uuid.uuid4().hex[:12]
        self.image = image
        self.extra = extra
        self.created = False
        self.port = None
        self.version = None

    def start(self):
        print('Disposable fixture:', self.name, self.image, flush=True)
        docker('create', '--name', self.name, '--label', 'dbunk.fixture=schema-compare-native',
               '--publish', '127.0.0.1::5432', '--tmpfs', '/var/lib/postgresql/data',
               '--env', 'POSTGRES_HOST_AUTH_METHOD=trust',
               '--env', 'POSTGRES_DB=schema_compare_native', *self.extra, self.image)
        self.created = True
        docker('start', self.name)
        for _ in range(120):
            ready = subprocess.run(['docker', 'exec', self.name, 'pg_isready', '-h', '127.0.0.1', '-U', 'postgres'],
                                   capture_output=True)
            if ready.returncode == 0:
                break
            time.sleep(0.25)
        else:
            raise RuntimeError(f'disposable fixture {self.name} did not become ready')
        # Readiness checks TCP so the image's temporary initialization server
        # cannot be mistaken for the final server.
        self.port = docker('port', self.name, '5432/tcp').rsplit(':', 1)[1]
        self.version = docker('exec', self.name, 'psql', '-U', 'postgres', '-Atc', 'SELECT version()')
        print(self.version, flush=True)

    def remove(self):
        if self.created:
            docker('rm', '-f', self.name)
            print('Removed:', self.name, flush=True)


def build_tls_image():
    subprocess.run(['sh', str(TLS_DIR / 'gen-certs.sh')], check=True)
    subprocess.run(['docker', 'build', '--quiet', '-t', TLS_IMAGE, str(TLS_DIR)], check=True)


def cargo_test(env, *args):
    subprocess.run(['cargo', 'test', '--manifest-path', str(ROOT / 'backend/Cargo.toml'),
                    *args], cwd=ROOT, env=env, check=True, timeout=1800)


def main():
    fixtures = []
    empty_init = tempfile.mkdtemp(prefix='dbunk-schema-compare-tls-init-')
    try:
        build_tls_image()
        for variable, image, extra in FIXTURES:
            fixture = Fixture(image, extra)
            fixtures.append((variable, fixture))
            fixture.start()
        tls = Fixture(TLS_IMAGE, [
            '--volume', f'{TLS_DIR / "certs"}:/certs:ro',
            '--volume', f'{empty_init}:/fixture-sql:ro',
        ])
        fixtures.append(('DBUNK_SCHEMA_COMPARE_TLS_PORT', tls))
        tls.start()
        env = dict(os.environ, CARGO_TARGET_DIR=CARGO_TARGET,
                   DBUNK_SCHEMA_COMPARE_TLS_CA=str(TLS_DIR / 'certs' / 'ca.crt'))
        for variable, fixture in fixtures:
            env[variable] = fixture.port
        print('Tested servers:', flush=True)
        for variable, fixture in fixtures:
            print(f'  {variable}: {fixture.version}', flush=True)
        cargo_test(env, 'schema_compare::', '--', '--ignored', '--nocapture',
                   '--skip', 'native_memory_profile')
        cargo_test(env, 'schema_compare::manager::validation::native_memory_profile', '--',
                   '--ignored', '--nocapture', '--test-threads=1')
    finally:
        for _, fixture in fixtures:
            fixture.remove()
        os.rmdir(empty_init)


if __name__ == '__main__':
    main()
