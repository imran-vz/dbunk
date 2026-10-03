#!/usr/bin/env python3
"""Run the native TLS facade matrix only after validating both owned fixtures."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import platform
import subprocess
import sys
import tempfile
import uuid

import fixture
import profile as stage04_profile
import tls_fixture
from workspace_launch import private_json


def run(path, out):
    owned, _ = fixture.check()
    tls_owned, root = tls_fixture.check()
    if not path.is_absolute() or path != path.resolve() or not path.parent.is_dir() or path.exists():
        raise RuntimeError("TLS acceptance requires a new canonical profile")
    out = Path(os.path.abspath(out)) if out else fixture.STATE.parent / "tls-evidence" / str(uuid.uuid4())
    if out != out.resolve() or out == path or out.is_relative_to(path):
        raise RuntimeError("TLS evidence must be a canonical directory outside the new profile")
    out.mkdir(parents=True, mode=0o700, exist_ok=False)
    print(f"Plain target: {fixture.ENDPOINT} instance={owned['instance']}", flush=True)
    print(f"TLS target: 127.0.0.1:15433/dbunk_tls_demo instance={tls_owned['instance']}; profile={path}", flush=True)
    print(f"Evidence: {out}", flush=True)
    build = ["cargo", "build", "--manifest-path", str(fixture.ROOT / "backend/Cargo.toml"), "--no-default-features", "--features", "isolated-profile", "--example", "native_tls_probe"]
    with (out / "build.txt").open("w") as log:
        subprocess.run(build, cwd=fixture.ROOT, stdout=log, stderr=subprocess.STDOUT, check=True)
    executable = fixture.ROOT / "backend/target/debug/examples/native_tls_probe"
    baseline = {"plain": fixture.backend_count(), "tls": tls_fixture.backend_count(tls_owned)}
    with tempfile.TemporaryDirectory(prefix="dbunk-native-tls-manifest-") as temporary:
        manifest = Path(temporary) / "fixture.json"
        private_json(manifest, stage04_profile.manifest(owned, tls_fixture.manifest(tls_owned)))
        command = [str(executable), str(path), str(manifest), str(root / "ca.pem"), str(root / "untrusted-ca.pem")]
        private_json(out / "identity.json", {
            "command": command, "profile": str(path), "fixtures": stage04_profile.manifest(owned, tls_fixture.manifest(tls_owned)),
            "executable_sha256": hashlib.sha256(executable.read_bytes()).hexdigest(),
            "platform": platform.platform(), "commit": subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=fixture.ROOT, text=True).strip(),
            "scope": "public native facade and real owned TLS endpoint; no actual window claim",
        })
        with (out / "native-tls.txt").open("w") as log:
            subprocess.run(command, cwd=fixture.ROOT, stdout=log, stderr=subprocess.STDOUT, check=True, timeout=90)
    fixture.wait_baseline(baseline["plain"])
    tls_fixture.wait_baseline(baseline["tls"], tls_owned)
    actual = tls_fixture.backend_count(tls_owned)
    private_json(out / "teardown.json", {"baseline": baseline, "final": {"plain": fixture.backend_count(), "tls": actual}, "profile_retained": str(path), "joined_native_shutdown": True})
    print("PASS: native TLS matrix; both owned PostgreSQL backend counts returned to baseline", flush=True)


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("path", type=Path, help="new disposable stage04 profile")
    parser.add_argument("--out", type=Path, help="new evidence directory")
    args = parser.parse_args()
    try:
        run(args.path, args.out)
    except (RuntimeError, OSError, ValueError, subprocess.SubprocessError) as error:
        sys.exit(f"Native TLS acceptance failed: {error}")
