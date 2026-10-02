#!/usr/bin/env python3
"""Build and launch the native host with a fresh, identity-marked profile."""
import argparse
import json
import os
from pathlib import Path
import signal
import subprocess
import sys
import tempfile
import uuid

import fixture

MARKER = ".dbunk-native-stage03"


def create_profile():
    path = Path(tempfile.mkdtemp(prefix="dbunk-native-stage03-")).resolve()
    os.chmod(path, 0o700)
    marker = {
        "version": 1, "fixture": fixture.PROJECT,
        "host": "127.0.0.1", "port": 15432, "database": "dbunk_demo",
        "profile_id": str(uuid.uuid4()),
    }
    (path / MARKER).write_text(json.dumps(marker) + "\n")
    return path, marker


def cleanup_profile(path, marker):
    if path.is_symlink() or json.loads((path / MARKER).read_text()) != marker:
        raise RuntimeError("Profile ownership changed; refusing cleanup")
    allowed = {MARKER, "launch.json", ".dbunk-native-lock", "dbunk.sqlite", "dbunk.sqlite-wal", "dbunk.sqlite-shm"}
    entries = list(path.iterdir())
    if any(entry.is_symlink() for entry in entries):
        raise RuntimeError("Profile contains a symlink; refusing cleanup")
    if any(not entry.is_file() or entry.name not in allowed for entry in entries):
        raise RuntimeError("Profile contains foreign entries; refusing cleanup")
    for entry in entries:
        entry.unlink()
    path.rmdir()


def launch(verify=False):
    if sys.platform != "darwin":
        raise RuntimeError("The native host requires macOS")
    owned, _ = fixture.check()
    subprocess.run(["cargo", "+1.98.1", "build", "--release", "--locked"], cwd=fixture.ROOT / "apps/native", check=True)
    executable = (fixture.ROOT / "apps/native/target/release/dbunk-native").resolve()
    if not executable.is_file():
        raise RuntimeError(f"Expected native executable at {executable}")
    baseline = fixture.backend_count()
    profile, marker = create_profile()
    evidence = fixture.STATE.parent / "evidence" / marker["profile_id"]
    evidence.mkdir(parents=True, mode=0o700)
    child = None
    clean_exit = False
    try:
        print(f"Target: owned fixture {fixture.PROJECT} {fixture.ENDPOINT} instance={owned['instance']}", flush=True)
        print(f"Isolated profile: {profile}", flush=True)
        print(f"Evidence: {evidence}", flush=True)
        with (evidence / "native.log").open("w") as log:
            child = subprocess.Popen([str(executable), "--profile", str(profile)], stdout=log, stderr=subprocess.STDOUT)
            launch_identity = {"pid": child.pid, "executable": str(executable), "profile_id": marker["profile_id"], "fixture_instance": owned["instance"]}
            (profile / "launch.json").write_text(json.dumps(launch_identity) + "\n")
            print(f"Native PID: {child.pid}", flush=True)
            if verify:
                probe = evidence / "editor-accessibility"
                subprocess.run(["swiftc", str(fixture.ROOT / "tools/measure/editor-accessibility.swift"), "-o", str(probe)], check=True)
                with (evidence / "accessibility.txt").open("w") as output:
                    subprocess.run([str(probe), "--native-fixture", str(child.pid), str(profile)], stdout=output, stderr=subprocess.STDOUT, check=True, timeout=120)
                # The native probe closes the actual window through AXClose.
                code = child.wait(timeout=7)
            else:
                code = child.wait()
            if code:
                raise RuntimeError(f"Native host exited {code}; inspect {evidence / 'native.log'}")
        fixture.wait_baseline(baseline)
        clean_exit = True
        (evidence / "teardown.json").write_text(json.dumps({"fixture_instance": owned["instance"], "backend_baseline": baseline, "backend_final": fixture.backend_count(), "exit_code": 0}) + "\n")
        if verify:
            print("PASS: native AX workflow and PostgreSQL teardown baseline")
    finally:
        if child and child.poll() is None:
            # Failure cleanup is not evidence of the native shutdown barrier.
            child.send_signal(signal.SIGTERM)
            try:
                child.wait(timeout=5)
            except subprocess.TimeoutExpired:
                child.kill()
                child.wait()
        if clean_exit:
            cleanup_profile(profile, marker)
        else:
            print(f"Failed launch profile retained for inspection: {profile}", file=sys.stderr)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--verify", action="store_true", help="drive the actual native window using the AX probe")
    args = parser.parse_args()
    launch(args.verify)


if __name__ == "__main__":
    try:
        main()
    except (RuntimeError, OSError, ValueError, subprocess.SubprocessError) as error:
        sys.exit(f"Native launch failed: {error}")
