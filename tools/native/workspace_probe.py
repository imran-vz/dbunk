#!/usr/bin/env python3
"""Verify native workspace services with a fresh owned profile and fixture.

--general-profile exercises the separate general endpoint capability while this
probe still contacts only its fixed launcher-verified PostgreSQL fixture.
"""
import argparse
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile

import fixture
import package
from profile import manifest


def run(path, *, general_profile=False):
    if sys.platform != "darwin" or not path.is_absolute() or path.exists():
        raise RuntimeError("Use a new absolute stage04 profile path on macOS")
    owned, _ = fixture.check()
    baseline = fixture.backend_count()
    print(f"Target: owned fixture {fixture.PROJECT} {fixture.ENDPOINT} instance={owned['instance']}", flush=True)
    print(f"Persistent probe profile: {path}", flush=True)
    print(f"Profile capability: {'general PostgreSQL' if general_profile else 'owned fixtures'}", flush=True)
    # Compile before the final ownership check. cargo run could otherwise spend
    # minutes building between checking the fixture and contacting its port.
    build = [
        "cargo", "build", "--quiet", "--locked", "--manifest-path", str(fixture.ROOT / "src-tauri/Cargo.toml"),
        "--no-default-features", "--features", "isolated-profile", "--example", "native_workspace_probe",
    ]
    subprocess.run(build, cwd=fixture.ROOT, check=True)
    metadata = json.loads(subprocess.check_output([
        "cargo", "metadata", "--locked", "--no-deps", "--format-version", "1",
        "--manifest-path", str(fixture.ROOT / "src-tauri/Cargo.toml"),
    ], cwd=fixture.ROOT, text=True))
    executable = Path(metadata["target_directory"]) / "debug/examples/native_workspace_probe"
    if not executable.is_file() or executable.is_symlink():
        raise RuntimeError("Built workspace probe executable is missing or linked")
    executable_hash = package.digest(executable)
    print(f"Probe executable: {executable}; SHA256: {executable_hash}", flush=True)
    with tempfile.TemporaryDirectory(prefix="dbunk-native-workspace-manifest-") as directory:
        source = Path(directory) / "fixture.json"
        source.write_text(json.dumps(manifest(owned)) + "\n")
        os.chmod(source, 0o600)
        operations = ["create-general", "reopen-general"] if general_profile else ["create", "reopen"]
        for operation in operations:
            if package.digest(executable) != executable_hash:
                raise RuntimeError("Probe executable changed after the build")
            # Recheck ownership before each process. Restoration does no network I/O.
            current, _ = fixture.check()
            if current["instance"] != owned["instance"]:
                raise RuntimeError("Fixture ownership changed; profile preserved")
            subprocess.run([
                str(executable), operation, str(path), str(source),
            ], cwd=fixture.ROOT, check=True)
            fixture.wait_baseline(baseline)
    print(f"PASS: PostgreSQL activity {baseline} -> {fixture.backend_count()}; profile retained", flush=True)


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("path", type=Path)
    parser.add_argument("--general-profile", action="store_true", help="exercise only a new disposable general-capability profile against the owned fixture")
    try:
        args = parser.parse_args()
        run(args.path, general_profile=args.general_profile)
    except (RuntimeError, OSError, ValueError, subprocess.SubprocessError) as error:
        sys.exit(f"Native workspace probe failed: {error}")
