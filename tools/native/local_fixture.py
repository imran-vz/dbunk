"""Explicitly owned local PostgreSQL fallback; no package install or system service."""
import json
import os
from pathlib import Path
import shutil
import socket
import subprocess
import time
import uuid

import fixture


def directory(owned):
    # Construct the path from a validated UUID; never trust a stored data path.
    instance = str(uuid.UUID(owned["instance"]))
    root = fixture.STATE.parent / "local" / instance
    if root != root.resolve():
        raise RuntimeError("Local fixture directory must not contain symlinks")
    return root


def validate(owned, *, running=True):
    from runtime import PREFIX

    executable = Path(owned["executable"])
    if executable != PREFIX / "bin/postgres" or executable.is_symlink():
        raise RuntimeError("Local fixture must use its task-private PostgreSQL executable")
    root = directory(owned)
    marker = root / ".dbunk-native-local"
    if root.is_symlink() or marker.is_symlink() or json.loads(marker.read_text()) != owned:
        raise RuntimeError("Local fixture ownership marker differs")
    if running:
        pid = owned["pid"]
        actual = fixture.run(["/bin/ps", "-p", str(pid), "-o", "command="])
        expected = f"{owned['executable']} -D {root / 'data'}"
        if actual != expected:
            raise RuntimeError("Local fixture process identity differs; refusing to contact/stop it")
        postmaster = (root / "data/postmaster.pid").read_text().splitlines()
        if int(postmaster[0]) != pid or postmaster[1] != str(root / "data") or postmaster[3] != "15432" or postmaster[5] != "127.0.0.1":
            raise RuntimeError("Local fixture postmaster identity differs")
    return root


def sql(root, query, *, database="dbunk_demo"):
    owned = json.loads((root / ".dbunk-native-local").read_text())
    validate(owned)
    psql = str(Path(owned["executable"]).with_name("psql"))
    return fixture.run([psql, "-X", "-qAt", "-v", "ON_ERROR_STOP=1", "-h", "127.0.0.1", "-p", "15432", "-U", "dbunk", "-d", database], input=query, env=dict(os.environ, PGPASSWORD="dbunk"))


def up():
    from runtime import prepare

    if fixture.STATE.exists():
        owned, _ = fixture.check()
        print(f"Fixture already ready: {fixture.ENDPOINT} instance={owned['instance']}")
        return
    probe = socket.socket()
    try:
        probe.bind(("127.0.0.1", 15432))
    except OSError as error:
        raise RuntimeError("Port 15432 occupied; refusing to contact or replace listener") from error
    finally:
        probe.close()
    executable = prepare() / "bin/postgres"
    owned = {"project": fixture.PROJECT, "instance": str(uuid.uuid4()), "driver": "local", "executable": str(executable)}
    root = directory(owned)
    root.mkdir(parents=True, mode=0o700)
    (root / ".dbunk-native-local").write_text(json.dumps(owned) + "\n")
    password = root / "initial-password"
    password.write_text("dbunk\n")
    os.chmod(password, 0o600)
    initdb = str(executable.with_name("initdb"))
    try:
        fixture.run([initdb, "-L", str(executable.parents[1] / "share/postgresql"), "-D", str(root / "data"), "-U", "dbunk", "--pwfile", str(password), "--auth-host=scram-sha-256", "--auth-local=scram-sha-256", "--encoding=UTF8", "--locale=C"])
    except BaseException:
        if json.loads((root / ".dbunk-native-local").read_text()) == owned and not any(path.is_symlink() for path in root.rglob("*")):
            shutil.rmtree(root)
        raise
    finally:
        password.unlink(missing_ok=True)
    with (root / "data/postgresql.conf").open("a") as config:
        config.write(f"\nlisten_addresses = '127.0.0.1'\nport = 15432\nunix_socket_directories = ''\nmax_connections = 100\n")
    print(f"Starting owned local PostgreSQL: {fixture.ENDPOINT} data={root / 'data'}", flush=True)
    with (root / "postgres.log").open("w") as log:
        process = subprocess.Popen([str(executable), "-D", str(root / "data")], stdout=log, stderr=subprocess.STDOUT, start_new_session=True)
    owned["pid"] = process.pid
    (root / ".dbunk-native-local").write_text(json.dumps(owned) + "\n")
    with fixture.STATE.open("x") as output:
        os.chmod(fixture.STATE, 0o600)
        json.dump(owned, output)
    deadline = time.monotonic() + 15
    while time.monotonic() < deadline:
        if process.poll() is not None:
            raise RuntimeError(f"Local postgres exited; inspect {root / 'postgres.log'}")
        try:
            sql(root, "SELECT 1;", database="postgres")
            break
        except (RuntimeError, FileNotFoundError):
            time.sleep(0.1)
    else:
        raise RuntimeError(f"Local postgres did not become ready; inspect {root / 'postgres.log'}")
    sql(root, "CREATE DATABASE dbunk_demo;", database="postgres")
    source = (fixture.ROOT / "tools/measure/fixtures/postgres.sql").read_text()
    source += (fixture.ROOT / "tools/native/fixture.sql").read_text()
    source += f"\nINSERT INTO plan026.fixture_identity VALUES ('{owned['instance']}');\n"
    sql(root, "BEGIN;\n" + source + "COMMIT;\n")
    fixture.check()
    print(f"Local fixture ready: {fixture.ENDPOINT} instance={owned['instance']} pid={process.pid}")


def down(owned):
    root = validate(owned, running=False)
    print(f"Stopping owned local fixture: {fixture.ENDPOINT} instance={owned['instance']}")
    pg_ctl = str(Path(owned["executable"]).with_name("pg_ctl"))
    status = subprocess.run(["/bin/ps", "-p", str(owned["pid"]), "-o", "command="], text=True, capture_output=True)
    if status.returncode == 0:
        validate(owned)
        fixture.run([pg_ctl, "-D", str(root / "data"), "stop", "-m", "fast", "-w", "-t", "10"])
    elif status.returncode != 1:
        raise RuntimeError("Cannot establish owned fixture process state; retaining files")
    validate(owned, running=False)
    # No tablespaces are created by this fixture; reject links before removal.
    if any(path.is_symlink() for path in root.rglob("*")):
        raise RuntimeError("Local fixture contains a symlink; retaining files")
    shutil.rmtree(root)
    fixture.STATE.unlink()
