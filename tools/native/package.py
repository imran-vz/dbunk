#!/usr/bin/env python3
"""Package the pinned native release host; optionally probe its isolated window."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import platform
import plistlib
import shutil
import signal
import subprocess
import sys
import tempfile

import fixture
import launch

BUNDLE_ID = "codes.imran.dbunk.native.stage04.preflight"
APP_NAME = "dbunk Native Preflight.app"
MARKER = ".dbunk-native-bundle.json"


def digest(path):
    with path.open("rb") as source:
        if hasattr(hashlib, "file_digest"):
            return hashlib.file_digest(source, "sha256").hexdigest()
        # Python < 3.11: same SHA-256 over bounded chunks.
        hasher = hashlib.sha256()
        for chunk in iter(lambda: source.read(1024 * 1024), b""):
            hasher.update(chunk)
        return hasher.hexdigest()


def assemble(executable, destination):
    """Create only a new directory; never overwrite or clean an existing bundle."""
    if destination != destination.resolve():
        raise RuntimeError("Bundle output must be canonical and must not contain symlinks")
    destination.mkdir(mode=0o700, parents=True, exist_ok=False)
    bundle = destination / APP_NAME
    contents = bundle / "Contents"
    (contents / "MacOS").mkdir(parents=True)
    (contents / "Resources").mkdir()
    binary = contents / "MacOS/dbunk-native"
    shutil.copy2(executable, binary)
    shutil.copy2(fixture.ROOT / "backend/icons/icon.icns", contents / "Resources/dbunk.icns")
    shutil.copy2(fixture.ROOT / "apps/native/THIRD-PARTY-NOTICES.txt", contents / "Resources/THIRD-PARTY-NOTICES.txt")
    info = {
        "CFBundleDevelopmentRegion": "en",
        "CFBundleDisplayName": "dbunk Native Preflight",
        "CFBundleName": "dbunk Native Preflight",
        "CFBundleExecutable": "dbunk-native",
        "CFBundleIdentifier": BUNDLE_ID,
        "CFBundleInfoDictionaryVersion": "6.0",
        "CFBundlePackageType": "APPL",
        "CFBundleShortVersionString": "0.0.0",
        "CFBundleVersion": "27",
        "CFBundleIconFile": "dbunk.icns",
        "NSHighResolutionCapable": True,
        "NSSupportsAutomaticGraphicsSwitching": True,
    }
    with (contents / "Info.plist").open("wb") as output:
        plistlib.dump(info, output, sort_keys=True)
    (contents / "PkgInfo").write_bytes(b"APPL????")
    files = {
        str(path.relative_to(bundle)): digest(path)
        for path in sorted(bundle.rglob("*")) if path.is_file()
    }
    identity = {
        "version": 1, "bundle_id": BUNDLE_ID, "bundle": str(bundle),
        "executable": str(binary), "files_sha256": files,
        "bundle_bytes": sum(path.stat().st_size for path in bundle.rglob("*") if path.is_file()),
        "resources": "Fonts, keymaps, themes and SQL grammar embedded in the release executable; icon in Contents/Resources",
        "signing": "No signing command; copied linker-produced signature unchanged",
    }
    (destination / MARKER).write_text(json.dumps(identity, indent=2) + "\n")
    return identity


def verify(identity, destination):
    """Run only this bundle with a new stage03 profile and an empty external cwd."""
    owned, _ = fixture.check()
    baseline = fixture.backend_count()
    probe = destination / "editor-accessibility"
    subprocess.run(["swiftc", str(fixture.ROOT / "tools/measure/editor-accessibility.swift"), "-o", str(probe)], check=True)
    profile, marker = launch.create_profile()
    child = None
    passed = False
    print(f"Target: owned fixture {fixture.PROJECT} {fixture.ENDPOINT} instance={owned['instance']}", flush=True)
    print(f"Bundle: {identity['bundle']}; isolated profile: {profile}", flush=True)
    try:
        with tempfile.TemporaryDirectory(prefix="dbunk-native-bundle-cwd-") as temporary:
            working_directory = Path(temporary).resolve()
            if working_directory.is_relative_to(fixture.ROOT):
                raise RuntimeError("Packaging probe requires a working directory outside the repository")
            env = os.environ.copy()
            env.pop("DBUNK_NATIVE_VERIFY", None)
            with (destination / "native.txt").open("w") as output:
                child = subprocess.Popen(
                    [identity["executable"], "--profile", str(profile)],
                    cwd=working_directory, env=env, stdout=output, stderr=subprocess.STDOUT,
                )
                launched = {
                    "pid": child.pid, "executable": identity["executable"],
                    "bundle": identity["bundle"], "bundle_id": BUNDLE_ID,
                    "profile_id": marker["profile_id"], "fixture_instance": owned["instance"],
                    "cwd": str(working_directory), "profile": str(profile),
                }
                (profile / "launch.json").write_text(json.dumps(launched) + "\n")
                (destination / "identity.json").write_text(json.dumps(launched, indent=2) + "\n")
                with (destination / "accessibility.txt").open("w") as output:
                    for action in ["--check-startup", "--window-step=recovery", "--window-step=quit"]:
                        subprocess.run(
                            [str(probe), "--native-fixture", str(child.pid), str(profile), action],
                            cwd=working_directory, stdout=output, stderr=subprocess.STDOUT,
                            check=True, timeout=30,
                        )
                code = child.wait(timeout=7)
                if code:
                    raise RuntimeError(f"Packaged host exited {code}")
            fixture.wait_baseline(baseline)
            log = (destination / "native.txt").read_text()
            queues = [line for line in log.splitlines() if "remaining_bytes=" in line]
            if "Native cleanup failed" in log or not queues or any(not line.endswith("remaining_bytes=0") for line in queues):
                raise RuntimeError("Missing successful queue and runtime cleanup evidence")
            (destination / "teardown.json").write_text(json.dumps({
                "exit_code": code, "backend_baseline": baseline,
                "backend_final": fixture.backend_count(), "fixture_instance": owned["instance"],
                "keychain": "Not exercised; unchanged open_fixture uses plain SQLite",
            }, indent=2) + "\n")
            passed = True
    finally:
        if child and child.poll() is None:
            # Forced failure cleanup never counts as successful app quit.
            child.send_signal(signal.SIGTERM)
            try:
                child.wait(timeout=5)
            except subprocess.TimeoutExpired:
                child.kill()
                child.wait()
        if passed:
            launch.cleanup_profile(profile, marker)
        else:
            print(f"FAILED: retained marked profile {profile}", file=sys.stderr)
    print("PASS: packaged AX identity, external-cwd startup, query and clean quit", flush=True)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--out", type=Path, required=True, help="new canonical output directory")
    parser.add_argument("--verify", action="store_true", help="requires the owned fixture and foreground Accessibility permission")
    args = parser.parse_args()
    if sys.platform != "darwin" or platform.machine() != "arm64":
        raise RuntimeError("Packaging preflight requires Apple Silicon macOS")
    destination = Path(os.path.abspath(args.out))
    if destination != destination.resolve() or destination.exists():
        raise RuntimeError("Choose a new canonical bundle output directory")
    if args.verify:
        fixture.check()
    subprocess.run(["cargo", "+1.98.1", "build", "--release", "--locked"], cwd=fixture.ROOT / "apps/native", check=True)
    subprocess.run([sys.executable, str(fixture.ROOT / "tools/native/dependencies.py")], check=True)
    identity = assemble(fixture.ROOT / "apps/native/target/release/dbunk-native", destination)
    for name, command in {
        "linked-libraries.txt": ["otool", "-L", identity["executable"]],
        "binary.txt": ["file", identity["executable"]],
        "signature.txt": ["codesign", "-d", "--verbose=2", identity["executable"]],
    }.items():
        # Inspection only. Unsigned is a valid result from codesign -d.
        result = subprocess.run(command, text=True, capture_output=True, check=name != "signature.txt")
        (destination / name).write_text(result.stdout + result.stderr)
    print(f"Built {identity['bundle']} ({identity['bundle_bytes']} bytes)", flush=True)
    if args.verify:
        verify(identity, destination)


if __name__ == "__main__":
    try:
        main()
    except (RuntimeError, OSError, ValueError, subprocess.SubprocessError) as error:
        sys.exit(f"Native packaging failed: {error}")
