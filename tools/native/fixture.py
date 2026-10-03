#!/usr/bin/env python3
"""Own one disposable stage03 fixture; never adopt or reset another database."""

import argparse
from datetime import datetime
import fcntl
import json
import os
import re
from pathlib import Path
import shutil
import socket
import subprocess
import sys
import time
import uuid

ROOT = Path(__file__).resolve().parents[2]
PROJECT = "dbunk-native-stage03"
STATE = ROOT / "tools/native/.state/fixture.json"
COMPOSE = ROOT / "tools/native/compose.yml"
ENDPOINT = "127.0.0.1:15432/dbunk_demo"


def run(args, *, input=None, env=None):
    result = subprocess.run(args, input=input, text=True, capture_output=True, env=env)
    if result.returncode:
        raise RuntimeError(result.stderr.strip() or result.stdout.strip() or str(args))
    return result.stdout.strip()


def docker(*args, **kwargs):
    return run(["docker", *args], **kwargs)


def prerequisites():
    if not shutil.which("docker"):
        raise RuntimeError("Docker is required; no fixture was created")
    docker("info", "--format", "{{.ServerVersion}}")
    docker("compose", "version")


def state():
    if STATE.is_symlink() or STATE.parent != STATE.parent.resolve():
        raise RuntimeError("Refusing symlink fixture state")
    value = json.loads(STATE.read_text())
    uuid.UUID(value["instance"])
    if value["project"] != PROJECT:
        raise RuntimeError("Foreign fixture state")
    return value


def containers():
    return docker("ps", "-aq", "--filter", f"label=com.docker.compose.project={PROJECT}").split()


def validate_container(info, owned):
    labels = info["Config"].get("Labels") or {}
    expected = {
        "com.docker.compose.project": PROJECT,
        "com.docker.compose.service": "postgres",
        "dev.dbunk.fixture": "native-stage03",
        "dev.dbunk.fixture.instance": owned["instance"],
    }
    if any(labels.get(key) != value for key, value in expected.items()):
        raise RuntimeError("Refusing foreign fixture container: ownership labels differ")
    bindings = info["HostConfig"].get("PortBindings", {}).get("5432/tcp")
    if bindings != [{"HostIp": "127.0.0.1", "HostPort": "15432"}]:
        raise RuntimeError("Fixture must bind only 127.0.0.1:15432")
    if "/var/lib/postgresql/data" not in info["HostConfig"].get("Tmpfs", {}):
        raise RuntimeError("Fixture must use disposable tmpfs storage")


def owned_container(owned):
    ids = containers()
    if len(ids) != 1:
        raise RuntimeError(f"Expected exactly one owned fixture container, found {len(ids)}")
    info = json.loads(docker("inspect", ids[0]))[0]
    validate_container(info, owned)
    return info["Id"]


def sql(container, query):
    if isinstance(container, Path):
        import local_fixture
        return local_fixture.sql(container, query)
    return docker("exec", "-i", container, "psql", "-X", "-qAt", "-v", "ON_ERROR_STOP=1", "-U", "dbunk", "-d", "dbunk_demo", input=query)


def check():
    owned = state()
    if owned.get("driver") == "local":
        import local_fixture
        container = local_fixture.validate(owned)
    else:
        prerequisites()
        container = owned_container(owned)
    actual = sql(container, "SELECT instance FROM plan026.fixture_identity;")
    if actual != owned["instance"]:
        raise RuntimeError("Fixture SQL sentinel differs; refusing this database")
    return owned, container


def up():
    if STATE.exists():
        owned, _ = check()
        print(f"Fixture already ready: {PROJECT} {ENDPOINT} instance={owned['instance']}")
        return
    try:
        prerequisites()
    except RuntimeError:
        import local_fixture
        local_fixture.up()
        return
    if containers():
        raise RuntimeError("Existing stage03 resources have no matching ownership file; refusing to adopt/reset them")
    probe = socket.socket()
    try:
        probe.bind(("127.0.0.1", 15432))
    except OSError as error:
        raise RuntimeError("Port 15432 is occupied; refusing to contact or replace its listener") from error
    finally:
        probe.close()
    if STATE.parent.is_symlink():
        raise RuntimeError("Refusing symlink fixture state directory")
    STATE.parent.mkdir(mode=0o700, exist_ok=True)
    owned = {"project": PROJECT, "instance": str(uuid.uuid4())}
    with STATE.open("x") as output:
        os.chmod(STATE, 0o600)
        json.dump(owned, output)
    env = dict(os.environ, DBUNK_NATIVE_FIXTURE_INSTANCE=owned["instance"])
    print(f"Creating owned tmpfs fixture: {PROJECT} {ENDPOINT}", flush=True)
    # On failure retain ownership state for explicit, identity-checked cleanup.
    run(["docker", "compose", "--project-name", PROJECT, "-f", str(COMPOSE), "up", "--detach", "--wait", "--wait-timeout", "60"], env=env)
    container = owned_container(owned)
    source = (ROOT / "tools/measure/fixtures/postgres.sql").read_text()
    source += (ROOT / "tools/native/fixture.sql").read_text()
    source += f"\nINSERT INTO plan026.fixture_identity VALUES ('{owned['instance']}');\n"
    sql(container, "BEGIN;\n" + source + "COMMIT;\n")
    check()
    print(f"Fixture ready: {PROJECT} {ENDPOINT} instance={owned['instance']}")


def down():
    owned = state()
    if owned.get("driver") == "local":
        import local_fixture
        local_fixture.down(owned)
        return
    prerequisites()
    ids = containers()
    if ids:
        container = owned_container(owned)
        print(f"Removing owned fixture: {PROJECT} instance={owned['instance']}")
        # Remove only this identity-checked container, never broad compose down.
        docker("rm", "--force", container)
    network_ids = docker("network", "ls", "-q", "--filter", f"label=com.docker.compose.project={PROJECT}").split()
    for network in network_ids:
        info = json.loads(docker("network", "inspect", network))[0]
        if info.get("Labels", {}).get("dev.dbunk.fixture.instance") != owned["instance"]:
            raise RuntimeError("Foreign fixture network ownership; leaving it intact")
        if info.get("Containers"):
            raise RuntimeError("Fixture network still has containers; leaving it and ownership state intact")
        docker("network", "rm", network)
    STATE.unlink()


def terminate_query(pid, backend_start, instance):
    """Inject loss only into the exact backend identified by the native fixture."""
    if type(pid) is not int or not 0 < pid <= 2147483647:
        raise RuntimeError("A positive PostgreSQL backend PID is required")
    # The helper accepts an identity timestamp, never a caller-supplied SQL
    # expression. Normalize only after requiring this narrow, quote-free form.
    if not re.fullmatch(r"[0-9]{4}-[0-9]{2}-[0-9]{2} [0-9]{2}:[0-9]{2}:[0-9]{2}(?:\.[0-9]{1,6})?[+-][0-9]{2}(?::[0-9]{2})?", backend_start):
        raise RuntimeError("A PostgreSQL backend_start timestamp with offset is required")
    # Python < 3.11 rejects PostgreSQL's hour-only offset; "+00" == "+00:00".
    normalized = backend_start + ":00" if re.search(r"[+-][0-9]{2}$", backend_start) else backend_start
    timestamp = datetime.fromisoformat(normalized)
    if timestamp.tzinfo is None:
        raise RuntimeError("Backend identity timestamp requires a timezone")
    instance = str(uuid.UUID(instance))
    owned, target = check()
    if owned["instance"] != instance:
        raise RuntimeError("Fixture instance differs from the native launch; refusing fault injection")
    result = sql(target, f"""
SELECT pg_terminate_backend(pid)
FROM pg_stat_activity
WHERE pid = {pid}
  AND backend_start = '{timestamp.isoformat(sep=' ')}'::timestamptz
  AND datname = 'dbunk_demo'
  AND usename = 'dbunk'
  AND pid <> pg_backend_pid();
""")
    if result != "t":
        raise RuntimeError("Exact owned query backend was not terminated; stale or mismatched identity")
    print(f"Terminated owned fixture query backend PID={pid} instance={instance}")


def backend_count():
    _, container = check()
    return int(sql(container, "SELECT count(*) FROM pg_stat_activity WHERE datname='dbunk_demo' AND pid <> pg_backend_pid();"))


def wait_baseline(expected, seconds=7):
    deadline = time.monotonic() + seconds
    actual = backend_count()
    while actual != expected and time.monotonic() < deadline:
        time.sleep(0.2)
        actual = backend_count()
    if actual != expected:
        raise RuntimeError(f"PostgreSQL teardown failed: expected {expected} backends, found {actual}")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("action", choices=["up", "down", "check", "count", "terminate-query"])
    parser.add_argument("--pid", type=int)
    parser.add_argument("--backend-start")
    parser.add_argument("--instance")
    args = parser.parse_args()
    identity = (args.pid, args.backend_start, args.instance)
    if args.action == "terminate-query" and any(value is None for value in identity):
        parser.error("terminate-query requires --pid, --backend-start and --instance")
    if args.action != "terminate-query" and any(value is not None for value in identity):
        parser.error("backend identity arguments are only valid for terminate-query")
    if STATE.parent.is_symlink():
        raise RuntimeError("Refusing symlink fixture state directory")
    STATE.parent.mkdir(mode=0o700, exist_ok=True)
    lock_path = STATE.parent / "fixture.lock"
    if lock_path.is_symlink():
        raise RuntimeError("Refusing symlink fixture operation lock")
    with lock_path.open("a") as lock:
        fcntl.flock(lock, fcntl.LOCK_EX)
        if args.action == "up":
            up()
        elif args.action == "down":
            down()
        elif args.action == "terminate-query":
            terminate_query(args.pid, args.backend_start, args.instance)
        elif args.action == "count":
            print(backend_count())
        else:
            owned, _ = check()
            print(f"Verified {PROJECT} {ENDPOINT} instance={owned['instance']}")


if __name__ == "__main__":
    try:
        main()
    except (RuntimeError, OSError, ValueError, KeyError) as error:
        sys.exit(f"Fixture unavailable: {error}")
