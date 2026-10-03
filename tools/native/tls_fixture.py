#!/usr/bin/env python3
"""Own a separate loopback TLS fixture; never adopt a listener or trust a global CA.

prepare builds/initializes without starting PostgreSQL and prints exact ownership.
up starts only that prepared UUID. Certificates remain private fixture files.
"""
import argparse
import fcntl
import json
import os
from pathlib import Path
import shutil
import socket
import subprocess
import sys
import time
import uuid

import fixture
import tls_runtime

PROJECT = "dbunk-native-stage04-tls"
PORT = 15433
DATABASE = "dbunk_tls_demo"
STATE = fixture.STATE.parent / "tls-fixture.json"
MARKER = ".dbunk-native-tls"
CERTIFICATES = ("ca.pem", "ca-key.pem", "untrusted-ca.pem", "untrusted-ca-key.pem", "server.pem", "server-key.pem", "client.pem", "client-key.pem")


def private_json(path, value, *, replace=False):
    target = path.with_name(path.name + ".new") if replace else path
    descriptor = os.open(target, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
    with os.fdopen(descriptor, "w") as output:
        json.dump(value, output, indent=2)
        output.write("\n")
        output.flush()
        os.fsync(output.fileno())
    if replace:
        target.replace(path)


def directory(owned):
    instance = str(uuid.UUID(owned["instance"]))
    if instance != owned["instance"] or owned.get("project") != PROJECT:
        raise RuntimeError("Invalid TLS fixture identity")
    root = STATE.parent / "tls" / instance
    if root != root.resolve():
        raise RuntimeError("TLS fixture path must be canonical without symlinks")
    return root


def immutable(owned):
    return {key: value for key, value in owned.items() if key not in ("phase", "pid")}


def state():
    if STATE.is_symlink() or STATE.parent != STATE.parent.resolve():
        raise RuntimeError("Refusing symlink TLS fixture state")
    owned = json.loads(STATE.read_text())
    directory(owned)
    if owned.get("phase") not in ("prepared", "running"):
        raise RuntimeError("TLS fixture state phase is invalid; retaining files")
    if owned["phase"] == "running" and (not isinstance(owned.get("pid"), int) or owned["pid"] <= 0):
        raise RuntimeError("TLS fixture PID is invalid; retaining files")
    return owned


def validate(owned, *, running=True):
    root = directory(owned)
    executable = tls_runtime.PREFIX / "bin/postgres"
    if owned.get("executable") != str(executable) or executable.is_symlink():
        raise RuntimeError("TLS fixture executable is not the owned TLS runtime")
    marker = root / MARKER
    if marker.is_symlink() or json.loads(marker.read_text()) != immutable(owned):
        raise RuntimeError("TLS fixture ownership marker differs")
    if owned.get("phase") == "prepared" and (root / "data/postmaster.pid").exists():
        raise RuntimeError("Prepared TLS directory has a postmaster; refusing to adopt or delete it")
    tls_runtime.validate()
    for relative, expected in owned["files_sha256"].items():
        path = root / relative
        if path.is_symlink() or not path.resolve().is_relative_to(root) or tls_runtime.digest(path) != expected:
            raise RuntimeError("TLS fixture certificate/configuration identity changed")
    if running:
        if owned.get("phase") != "running" or not isinstance(owned.get("pid"), int) or owned["pid"] <= 0:
            raise RuntimeError("TLS fixture is not a running owned process")
        actual = fixture.run(["/bin/ps", "-p", str(owned["pid"]), "-o", "command="])
        if actual != f"{executable} -D {root / 'data'}":
            raise RuntimeError("TLS fixture process identity differs; refusing to contact or stop it")
        postmaster = (root / "data/postmaster.pid").read_text().splitlines()
        if int(postmaster[0]) != owned["pid"] or postmaster[1] != str(root / "data") or postmaster[3] != str(PORT) or postmaster[5] != "127.0.0.1":
            raise RuntimeError("TLS fixture postmaster identity differs")
    return root


def certificates(root):
    openssl = str(Path(tls_runtime.inputs()["openssl_prefix"]) / "bin/openssl")
    for name in ("ca", "untrusted-ca"):
        fixture.run([openssl, "req", "-x509", "-newkey", "rsa:2048", "-nodes", "-days", "2", "-keyout", str(root / f"{name}-key.pem"), "-out", str(root / f"{name}.pem"), "-subj", f"/CN=dbunk-{name}-{root.name}", "-addext", "basicConstraints=critical,CA:TRUE,pathlen:0", "-addext", "keyUsage=critical,keyCertSign,cRLSign"])
    for name, usage in (("server", "serverAuth"), ("client", "clientAuth")):
        extensions = root / f"{name}-extensions.cnf"
        extensions.write_text(f"basicConstraints=critical,CA:FALSE\nkeyUsage=critical,digitalSignature,keyEncipherment\nextendedKeyUsage={usage}\nsubjectAltName=DNS:localhost,IP:127.0.0.1\n")
        fixture.run([openssl, "req", "-new", "-newkey", "rsa:2048", "-nodes", "-keyout", str(root / f"{name}-key.pem"), "-out", str(root / f"{name}.csr"), "-subj", "/CN=dbunk"])
        fixture.run([openssl, "x509", "-req", "-in", str(root / f"{name}.csr"), "-CA", str(root / "ca.pem"), "-CAkey", str(root / "ca-key.pem"), "-set_serial", "1" if name == "server" else "2", "-days", "2", "-extfile", str(extensions), "-out", str(root / f"{name}.pem")])
    for path in root.iterdir():
        if path.is_file():
            path.chmod(0o600)


def prepare():
    if STATE.exists():
        owned = state()
        validate(owned, running=owned.get("phase") == "running")
        print(json.dumps(description(owned), indent=2), flush=True)
        return
    # Check occupation without connecting to or identifying an existing listener.
    with socket.socket() as probe:
        try:
            probe.bind(("127.0.0.1", PORT))
        except OSError as error:
            raise RuntimeError("Port 15433 is occupied; refusing to contact or replace its listener") from error
    executable = tls_runtime.prepare() / "bin/postgres"
    owned = {"version": 1, "project": PROJECT, "instance": str(uuid.uuid4()), "executable": str(executable)}
    root = directory(owned)
    root.mkdir(parents=True, mode=0o700)
    certificates(root)
    password = root / "initial-password"
    descriptor = os.open(password, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
    with os.fdopen(descriptor, "w") as output:
        output.write("dbunk\n")
    try:
        fixture.run([str(executable.with_name("initdb")), "-L", str(tls_runtime.PREFIX / "share/postgresql"), "-D", str(root / "data"), "-U", "dbunk", "--pwfile", str(password), "--auth-host=scram-sha-256", "--auth-local=scram-sha-256", "--encoding=UTF8", "--locale=C"])
    finally:
        password.unlink(missing_ok=True)
    with (root / "data/postgresql.conf").open("a") as output:
        output.write(f"\nlisten_addresses = '127.0.0.1'\nport = {PORT}\nunix_socket_directories = ''\nmax_connections = 30\nssl = on\n")
        for setting, name in (("ssl_cert_file", "server.pem"), ("ssl_key_file", "server-key.pem"), ("ssl_ca_file", "ca.pem")):
            path = str(root / name).replace("'", "''")
            output.write(f"{setting} = '{path}'\n")
    (root / "data/pg_hba.conf").write_text("hostssl all dbunk 127.0.0.1/32 scram-sha-256\nhostnossl all all 127.0.0.1/32 reject\n")
    owned["files_sha256"] = {name: tls_runtime.digest(root / name) for name in (*CERTIFICATES, "data/postgresql.conf", "data/pg_hba.conf")}
    private_json(root / MARKER, owned)
    owned["phase"] = "prepared"
    private_json(STATE, owned)
    print(json.dumps(description(owned), indent=2), flush=True)
    print("Prepared only; no PostgreSQL listener started. Name this target before running up.", flush=True)


def connection_env(root, *, mode="verify-full", ca="ca.pem", server_name="127.0.0.1", client=False):
    env = {name: value for name, value in os.environ.items() if not name.startswith("PG")}
    env.update(PGHOST=server_name, PGHOSTADDR="127.0.0.1", PGPORT=str(PORT), PGUSER="dbunk", PGPASSWORD="dbunk", PGDATABASE=DATABASE, PGSSLMODE=mode, PGCONNECT_TIMEOUT="3", PGGSSENCMODE="disable", PGPASSFILE=str(root / "no-passfile"), PGSSLROOTCERT=str(root / ca), PGSSLCERT=str(root / ("client.pem" if client else "no-client.pem")), PGSSLKEY=str(root / ("client-key.pem" if client else "no-client-key.pem")), PGSSLCRL=str(root / "no-crl.pem"), PGSSLCRLDIR=str(root / "no-crl-dir"))
    return env


def query(owned, sql, *, database=DATABASE, **tls):
    root = validate(owned)
    env = connection_env(root, **tls)
    env["PGDATABASE"] = database
    return subprocess.run([str(tls_runtime.PREFIX / "bin/psql"), "-X", "-w", "-qAt", "-v", "ON_ERROR_STOP=1"], input=sql, text=True, capture_output=True, env=env, timeout=8)


def up():
    owned = state()
    if owned.get("phase") == "running":
        check()
        print(json.dumps(description(owned), indent=2))
        return
    root = validate(owned, running=False)
    with socket.socket() as probe:
        try:
            probe.bind(("127.0.0.1", PORT))
        except OSError as error:
            raise RuntimeError("Port 15433 is occupied; refusing to contact or replace its listener") from error
    print(f"Starting owned TLS fixture {PROJECT} 127.0.0.1:{PORT}/{DATABASE} instance={owned['instance']} data={root / 'data'}", flush=True)
    with (root / "postgres.log").open("w") as log:
        process = subprocess.Popen([owned["executable"], "-D", str(root / "data")], stdout=log, stderr=subprocess.STDOUT, start_new_session=True)
    owned.update(phase="running", pid=process.pid)
    private_json(STATE, owned, replace=True)
    deadline = time.monotonic() + 15
    while time.monotonic() < deadline:
        if process.poll() is not None:
            raise RuntimeError(f"Owned TLS postgres exited; inspect {root / 'postgres.log'}")
        try:
            ready = query(owned, "SELECT 1;", database="postgres")
            if ready.returncode == 0:
                break
        except (RuntimeError, FileNotFoundError):
            pass
        time.sleep(0.1)
    else:
        raise RuntimeError(f"Owned TLS postgres did not become ready; retained {root}")
    result = query(owned, f"CREATE DATABASE {DATABASE};", database="postgres")
    if result.returncode:
        raise RuntimeError("Owned TLS fixture database creation failed; profile retained")
    source = (fixture.ROOT / "tools/measure/fixtures/postgres.sql").read_text()
    source += f"\nCREATE SCHEMA plan027; CREATE TABLE plan027.fixture_identity (instance uuid PRIMARY KEY); INSERT INTO plan027.fixture_identity VALUES ('{owned['instance']}');\n"
    result = query(owned, "BEGIN;\n" + source + "COMMIT;\n")
    if result.returncode:
        raise RuntimeError("Owned TLS fixture initialization failed; profile retained")
    check()
    print(json.dumps(description(owned), indent=2), flush=True)


def check():
    owned = state()
    root = validate(owned)
    result = query(owned, "SELECT instance FROM plan027.fixture_identity;")
    if result.returncode or result.stdout.strip() != owned["instance"]:
        raise RuntimeError("TLS fixture SQL sentinel or trusted certificate differs; refusing this database")
    return owned, root


def backend_count(owned=None):
    if owned is None:
        owned, _ = check()
    result = query(owned, "SELECT count(*) FROM pg_stat_activity WHERE datname='dbunk_tls_demo' AND pid <> pg_backend_pid();")
    if result.returncode:
        raise RuntimeError("Owned TLS backend count failed")
    return int(result.stdout.strip())


def wait_baseline(expected, owned=None, seconds=7):
    if owned is None:
        owned, _ = check()
    deadline = time.monotonic() + seconds
    actual = backend_count(owned)
    while actual != expected and time.monotonic() < deadline:
        time.sleep(0.2)
        actual = backend_count(owned)
    if actual != expected:
        raise RuntimeError(f"TLS teardown failed: expected {expected} backends, found {actual}")


def manifest(owned):
    return {"fixture": PROJECT, "instance": owned["instance"], "host": "127.0.0.1", "port": PORT, "database": DATABASE, "user": "dbunk"}


def description(owned):
    root = directory(owned)
    return {"manifest": manifest(owned), "phase": owned["phase"], "data": str(root / "data"), "pid": owned.get("pid"), "trusted_ca": str(root / "ca.pem"), "untrusted_ca": str(root / "untrusted-ca.pem"), "client_certificate": str(root / "client.pem"), "client_key": str(root / "client-key.pem"), "global_trust": "unchanged; certificates are fixture files only"}


def matrix():
    owned, _ = check()
    cases = [
        ("trusted-verify-full", {}, True),
        ("untrusted-ca-rejected", {"ca": "untrusted-ca.pem"}, False),
        ("hostname-mismatch-rejected", {"server_name": "wrong.dbunk.invalid"}, False),
        ("verify-ca-ignores-hostname", {"mode": "verify-ca", "server_name": "wrong.dbunk.invalid"}, True),
        ("require-encrypts", {"mode": "require", "ca": "no-root.pem"}, True),
        ("trusted-client-certificate", {"client": True}, True),
        ("plaintext-rejected", {"mode": "disable"}, False),
    ]
    results = []
    for name, options, expected in cases:
        result = query(owned, "SELECT ssl FROM pg_stat_ssl WHERE pid = pg_backend_pid();", **options)
        passed = (result.returncode == 0 and result.stdout.strip() == "t") if expected else result.returncode != 0
        results.append({"case": name, "passed": passed, "connected": result.returncode == 0, "error": result.stderr.strip() if result.returncode else None})
        if not passed:
            raise RuntimeError(f"TLS fixture matrix failed: {name}")
    print(json.dumps({"scope": "owned libpq transport; native facade/window acceptance is separate", "fixture": manifest(owned), "cases": results}, indent=2), flush=True)


def down():
    owned = state()
    root = validate(owned, running=False)
    if owned.get("phase") == "running":
        status = subprocess.run(["/bin/ps", "-p", str(owned["pid"]), "-o", "command="], text=True, capture_output=True)
        if status.returncode == 0:
            validate(owned)
            fixture.run([str(tls_runtime.PREFIX / "bin/pg_ctl"), "-D", str(root / "data"), "stop", "-m", "fast", "-w", "-t", "10"])
        elif status.returncode != 1:
            raise RuntimeError("Cannot establish TLS process state; retaining files")
    validate(owned, running=False)
    if any(path.is_symlink() for path in root.rglob("*")):
        raise RuntimeError("TLS fixture contains a symlink; retaining files")
    shutil.rmtree(root)
    STATE.unlink()
    print(f"Removed only owned TLS fixture instance={owned['instance']}; plain fixture unchanged", flush=True)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("operation", choices=["prepare", "up", "check", "matrix", "down"])
    args = parser.parse_args()
    if STATE.parent != STATE.parent.resolve():
        raise RuntimeError("TLS state parent must be canonical")
    STATE.parent.mkdir(parents=True, mode=0o700, exist_ok=True)
    lock_path = STATE.parent / "tls-fixture.lock"
    descriptor = os.open(lock_path, os.O_RDWR | os.O_CREAT | os.O_NOFOLLOW, 0o600)
    with os.fdopen(descriptor, "w") as lock:
        fcntl.flock(lock, fcntl.LOCK_EX)
        if args.operation == "check":
            owned, _ = check()
            print(json.dumps(description(owned), indent=2))
        else:
            {"prepare": prepare, "up": up, "matrix": matrix, "down": down}[args.operation]()


if __name__ == "__main__":
    try:
        main()
    except (RuntimeError, OSError, ValueError, subprocess.SubprocessError) as error:
        sys.exit(f"Native TLS fixture failed: {error}")
