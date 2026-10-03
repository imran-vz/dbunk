#!/usr/bin/env python3
"""Create or check a stage04 profile after verifying the owned fixture."""
import argparse
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import uuid

import fixture


def manifest(owned, tls=None):
    value = {
        "version": 1, "fixture": fixture.PROJECT, "instance": owned["instance"],
        "host": "127.0.0.1", "port": 15432, "database": "dbunk_demo", "user": "dbunk",
    }
    if tls is not None:
        value["tls"] = tls
    return value


def run(operation, path, *, with_tls=False):
    if sys.platform != "darwin":
        raise RuntimeError("Stage04 native profiles require macOS")
    if not path.is_absolute():
        raise RuntimeError("A canonical absolute profile path is required")
    owned, _ = fixture.check()
    tls = None
    if with_tls:
        import tls_fixture
        tls_owned, _ = tls_fixture.check()
        tls = tls_fixture.manifest(tls_owned)
    print(f"Target: owned fixture {fixture.PROJECT} {fixture.ENDPOINT} instance={owned['instance']}", flush=True)
    keychain = operation.startswith("keychain-")
    if keychain and operation != "keychain-prepare":
        with (path / ".dbunk-native-stage04").open("rb") as source:
            encoded = source.read(8193)
        if len(encoded) > 8192:
            raise RuntimeError("Development marker is too large")
        marker = json.loads(encoded)
        namespace = marker["credential_namespace"]
        if str(uuid.UUID(namespace)) != namespace:
            raise RuntimeError("Invalid development credential namespace")
        print(f"Keychain target: service=dbunk-native-stage04-{namespace}; accounts=connection-credentials-{namespace}, connection-credentials-{namespace}-connection-rollback-v1", flush=True)
    # Rust creates the profile and namespace, checks paths and takes its lock.
    # Python never adopts, edits or removes a profile and never receives secrets.
    with tempfile.TemporaryDirectory(prefix="dbunk-native-manifest-") as directory:
        source = Path(directory) / "fixture.json"
        source.write_text(json.dumps(manifest(owned, tls)) + "\n")
        os.chmod(source, 0o600)
        subprocess.run([
            "cargo", "run", "--quiet", "--manifest-path", str(fixture.ROOT / "backend/Cargo.toml"),
            "--no-default-features", "--features", "isolated-profile", "--example",
            "native_keychain_probe" if keychain else "native_profile",
            "--", operation.removeprefix("keychain-"), str(path), str(source),
        ], cwd=fixture.ROOT, check=True)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("operation", choices=["create", "check", "keychain-prepare", "keychain-seed", "keychain-reopen", "keychain-cleanup"],
                        help="Keychain phases are opt-in foreground acceptance; prepare creates a new disposable profile without OS Keychain access")
    parser.add_argument("path", type=Path)
    parser.add_argument("--tls", action="store_true", help="explicitly include the separately verified owned TLS fixture; requires a new matching profile")
    args = parser.parse_args()
    run(args.operation, args.path, with_tls=args.tls)


if __name__ == "__main__":
    try:
        main()
    except (RuntimeError, OSError, ValueError, subprocess.SubprocessError) as error:
        sys.exit(f"Native profile failed: {error}")
