#!/usr/bin/env python3
"""Run native capture tests only in a newly owned, disposable loopback PG16.

No DSN or existing container is accepted. No shared compose resources are used.
"""
import os
from pathlib import Path
import subprocess
import time
import uuid

ROOT = Path(__file__).resolve().parents[3]

def docker(*args):
    return subprocess.check_output(['docker', *args], text=True).strip()

def main():
    name = 'dbunk-schema-compare-native-' + uuid.uuid4().hex[:12]
    print('Disposable fixture:', name, flush=True)
    created = False
    try:
        docker('create', '--name', name, '--label', 'dbunk.fixture=schema-compare-native',
               '--publish', '127.0.0.1::5432', '--tmpfs', '/var/lib/postgresql/data',
               '--env', 'POSTGRES_HOST_AUTH_METHOD=trust', '--env', 'POSTGRES_DB=schema_compare_native',
               'postgres:16')
        created = True
        docker('start', name)
        for _ in range(60):
            ready = subprocess.run(['docker', 'exec', name, 'pg_isready', '-h', '127.0.0.1', '-U', 'postgres'], capture_output=True)
            if ready.returncode == 0:
                break
            time.sleep(0.25)
        else:
            raise RuntimeError('disposable fixture did not become ready')
        port = docker('port', name, '5432/tcp').rsplit(':', 1)[1]
        print(docker('exec', name, 'psql', '-U', 'postgres', '-Atc', 'SELECT version()'), flush=True)
        env = dict(os.environ, DBUNK_SCHEMA_COMPARE_TEST_PORT=port,
                   CARGO_TARGET_DIR='/tmp/dbunk-plan021-target')
        subprocess.run(['cargo', 'test', '--manifest-path', str(ROOT / 'src-tauri/Cargo.toml'),
                        'schema_compare::capture::tests::native_capture', '--', '--ignored', '--nocapture'],
                       cwd=ROOT, env=env, check=True, timeout=900)
    finally:
        if created:
            docker('rm', '-f', name)
            print('Removed:', name, flush=True)

if __name__ == '__main__':
    main()
