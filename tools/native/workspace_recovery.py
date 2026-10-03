#!/usr/bin/env python3
"""Seed and verify recovery cases in a CLOSED, owned stage04 fixture profile.

Only the native workspace record is changed. The app's exclusive profile lock,
private file rules, live fixture ownership, and marker/SQLite identity are checked
before SQLite writes. Receipts contain hashes, never credential contents.
"""
import argparse
from contextlib import contextmanager
import fcntl
import hashlib
import json
import os
from pathlib import Path
import sqlite3
import stat
import sys
import uuid

import fixture
import profile as stage04_profile

KEY = "ui.v1.native.workspace"
LIMIT = 448 * 1024
TAIL = "-- unsaved recovery tail " + "z" * 2048
FILES = {".dbunk-native-stage04", ".dbunk-native-lock", "launch.json", "dbunk.sqlite", "dbunk.sqlite-wal", "dbunk.sqlite-shm"}


def digest(value):
    return hashlib.sha256(value if isinstance(value, bytes) else value.encode()).hexdigest()


def private_file(path):
    info = path.lstat()
    if not stat.S_ISREG(info.st_mode) or info.st_uid != os.getuid() or stat.S_IMODE(info.st_mode) != 0o600 or info.st_nlink != 1:
        raise RuntimeError("Recovery requires private, owned, unlinked regular profile files")


def canonical(path):
    if not path.is_absolute() or path != path.resolve():
        raise RuntimeError("Recovery paths must be canonical and absolute")


@contextmanager
def closed_profile(path):
    canonical(path)
    info = path.lstat()
    if not stat.S_ISDIR(info.st_mode) or info.st_uid != os.getuid() or stat.S_IMODE(info.st_mode) != 0o700:
        raise RuntimeError("Recovery requires a private owned stage04 directory")
    for item in path.iterdir():
        if item.name not in FILES:
            raise RuntimeError("Recovery profile contains a foreign entry")
        private_file(item)
    marker_file = path / ".dbunk-native-stage04"
    if marker_file.stat().st_size > 8192:
        raise RuntimeError("Recovery marker is oversized")
    marker = json.loads(marker_file.read_text())
    if marker.get("version") != 1 or marker.get("path") != str(path):
        raise RuntimeError("Recovery marker does not match the profile")
    for key in ("profile_id", "credential_namespace"):
        value = uuid.UUID(marker[key])
        if value.version != 4 or str(value) != marker[key]:
            raise RuntimeError("Recovery marker identity is invalid")
    if marker["profile_id"] == marker["credential_namespace"]:
        raise RuntimeError("Recovery marker identities are not independent")
    owned, _ = fixture.check()
    tls = None
    if marker.get("fixtures", {}).get("tls") is not None:
        import tls_fixture
        tls_owned, _ = tls_fixture.check()
        tls = tls_fixture.manifest(tls_owned)
    if marker.get("fixtures") != stage04_profile.manifest(owned, tls):
        raise RuntimeError("Recovery profile fixture ownership changed")
    print(f"Target: stage04 profile {path}; owned fixture instance={owned['instance']}", flush=True)
    descriptor = os.open(path / ".dbunk-native-lock", os.O_RDWR | os.O_NOFOLLOW)
    try:
        try:
            fcntl.flock(descriptor, fcntl.LOCK_EX | fcntl.LOCK_NB)
        except BlockingIOError as error:
            raise RuntimeError("Recovery refuses an active native profile") from error
        connection = sqlite3.connect(f"file:{path / 'dbunk.sqlite'}?mode=rw", uri=True, timeout=0)
        try:
            row = connection.execute("SELECT value FROM app_settings WHERE key='native.stage04.identity'").fetchone()
            if row is None or json.loads(row[0]) != marker:
                raise RuntimeError("Recovery SQLite identity differs from its marker")
            yield connection, marker
        finally:
            connection.close()
    finally:
        os.close(descriptor)


def other_state(connection):
    # Hash persisted metadata and credential bytes locally without emitting them.
    hasher = hashlib.sha256()
    for table in ("connections", "credentials", "credential_verifier", "app_settings"):
        rows = connection.execute(f"SELECT * FROM {table} ORDER BY rowid").fetchall()
        hasher.update(table.encode())
        hasher.update(json.dumps(rows, ensure_ascii=True, separators=(",", ":")).encode())
    rows = connection.execute("SELECT * FROM ui_state WHERE key != ? ORDER BY key", (KEY,)).fetchall()
    hasher.update(json.dumps(rows, ensure_ascii=True, separators=(",", ":")).encode())
    return hasher.hexdigest()


def near_limit(connection_id):
    document = {"id": str(uuid.uuid4()), "name": "Recovery draft", "connectionId": connection_id,
                "sql": "", "pinned": False, "selection": {"anchor": 0, "head": 0}}
    record = {"version": 1, "snapshot": {"documents": [document], "activeDocumentId": document["id"],
              "layout": "stacked", "density": "comfortable", "navigatorWidth": 240.0}}
    encode = lambda: json.dumps(record, ensure_ascii=True, separators=(",", ":"))
    text = "-- recovery baseline\n" + ("--" + "x" * 1000 + "\n") * 500
    low, high = 0, len(text)
    while low < high:
        middle = (low + high + 1) // 2
        document["sql"] = text[:middle]
        document["selection"] = {"anchor": middle, "head": middle}
        if len(encode().encode()) <= LIMIT - 512:
            low = middle
        else:
            high = middle - 1
    document["sql"] = text[:low]
    document["selection"] = {"anchor": low, "head": low}
    return encode(), document["sql"]


def new_receipt(path, value):
    canonical(path)
    descriptor = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW, 0o600)
    with os.fdopen(descriptor, "w") as output:
        json.dump(value, output, indent=2)
        output.write("\n")
        output.flush()
        os.fsync(output.fileno())


def seed(path, kind, receipt):
    canonical(receipt)
    if receipt.exists() or receipt.is_relative_to(path):
        raise RuntimeError("Choose a new receipt outside the profile")
    with closed_profile(path) as (connection, marker):
        ids = [row[0] for row in connection.execute("SELECT id FROM connections ORDER BY id")]
        if not ids:
            raise RuntimeError("Prepare at least one fixture connection through the native app first")
        sql = None
        missing_binding = None
        document_id = None
        if kind == "missing":
            row = connection.execute("SELECT value FROM ui_state WHERE key=?", (KEY,)).fetchone()
            record = json.loads(row[0]) if row else {}
            documents = record.get("snapshot", {}).get("documents", [])
            if record.get("version") not in (1, 2) or len(documents) != 1:
                raise RuntimeError("Missing-binding acceptance requires one valid saved document")
            document = documents[0]
            sql = document["sql"]
            document_id = document["id"]
            missing_binding = str(uuid.uuid4())
            if missing_binding in ids:
                raise RuntimeError("Generated missing binding unexpectedly exists")
            document["connectionId"] = missing_binding
            encoded = json.dumps(record, ensure_ascii=True, separators=(",", ":"))
        elif kind == "corrupt":
            encoded = "{broken native workspace\n"
        elif kind == "future":
            encoded = '{"version":999,"snapshot":{"future":"preserve this exact record"}}\n'
        else:
            encoded, sql = near_limit(ids[0])
        revision = str(uuid.uuid4())
        evidence = {"version": 1, "profile": str(path), "profile_id": marker["profile_id"], "kind": kind,
                    "seeded_sha256": digest(encoded), "seeded_revision": revision,
                    "connection_ids": ids, "other_state_sha256": other_state(connection)}
        if missing_binding is not None:
            evidence.update(sql_sha256=digest(sql), document_id=document_id, missing_binding=missing_binding)
        elif sql is not None:
            exported = "-- Query document 1\n\n" + sql + TAIL + "\n\n"
            evidence.update(sql_sha256=digest(sql), sql_bytes=len(sql.encode()),
                            export_sha256=digest(exported), export_bytes=len(exported.encode()))
        new_receipt(receipt, evidence)
        with connection:
            connection.execute("INSERT INTO ui_state(key,value,updated_at) VALUES(?,?,?) ON CONFLICT(key) DO UPDATE SET value=excluded.value,updated_at=excluded.updated_at", (KEY, encoded, revision))
        print(f"PASS: seeded {kind} recovery case; receipt={receipt}")


def check(path, receipt, expected, export):
    canonical(receipt)
    private_file(receipt)
    evidence = json.loads(receipt.read_text())
    with closed_profile(path) as (connection, marker):
        if evidence.get("profile") != str(path) or evidence.get("profile_id") != marker["profile_id"]:
            raise RuntimeError("Recovery receipt belongs to another profile")
        if other_state(connection) != evidence["other_state_sha256"]:
            raise RuntimeError("Recovery changed connection, credential, or unrelated state")
        row = connection.execute("SELECT value,updated_at FROM ui_state WHERE key=?", (KEY,)).fetchone()
        if row is None:
            raise RuntimeError("Recovery workspace record is missing")
        if expected == "unchanged":
            if digest(row[0]) != evidence["seeded_sha256"] or row[1] != evidence["seeded_revision"]:
                raise RuntimeError("Normal quit changed the original recovery record")
        elif expected == "missing":
            saved = json.loads(row[0])
            documents = saved.get("snapshot", {}).get("documents", [])
            if saved.get("version") not in (1, 2) or len(documents) != 1:
                raise RuntimeError("Missing-binding restoration discarded the document")
            document = documents[0]
            if document.get("id") != evidence["document_id"] or document.get("connectionId") != evidence["missing_binding"] or digest(document["sql"]) != evidence["sql_sha256"]:
                raise RuntimeError("Missing-binding restoration rewrote the binding or SQL")
        elif expected == "reset":
            saved = json.loads(row[0])
            if saved.get("version") not in (1, 2) or saved["snapshot"]["documents"] != [] or saved["snapshot"]["activeDocumentId"] is not None:
                raise RuntimeError("Explicit recovery reset did not save an empty workspace")
        else:
            saved = json.loads(row[0])
            documents = saved["snapshot"]["documents"]
            if len(documents) != 1 or digest(documents[0]["sql"]) != evidence.get("sql_sha256"):
                raise RuntimeError("Oversized edits replaced the last durable SQL")
            if export is None:
                raise RuntimeError("Oversize check requires the native exported SQL file")
            canonical(export)
            if export.is_symlink() or not export.is_file():
                raise RuntimeError("Native SQL export is missing or symlinked")
            data = export.read_bytes()
            if len(data) != evidence["export_bytes"] or digest(data) != evidence["export_sha256"]:
                raise RuntimeError("Native SQL export did not preserve all oversized in-memory SQL")
        print(f"PASS: {expected}; connections, credentials and unrelated records unchanged")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("operation", choices=["seed-corrupt", "seed-future", "seed-large", "seed-missing", "check-unchanged", "check-reset", "check-oversize", "check-missing"])
    parser.add_argument("profile", type=Path)
    parser.add_argument("receipt", type=Path)
    parser.add_argument("--export", type=Path)
    args = parser.parse_args()
    if args.operation.startswith("seed-"):
        seed(args.profile, args.operation.removeprefix("seed-"), args.receipt)
    else:
        check(args.profile, args.receipt, args.operation.removeprefix("check-"), args.export)


if __name__ == "__main__":
    try:
        main()
    except (RuntimeError, OSError, ValueError, KeyError, sqlite3.Error) as error:
        sys.exit(f"Native workspace recovery failed: {error}")
