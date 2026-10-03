#!/usr/bin/env python3
"""Launch an owned native verification workspace; stage04 is the default.

General PostgreSQL profiles require explicit creation or a prior private launcher
receipt. Receipts bind this verification workflow to checked owned fixtures; they
do not restrict endpoints in the general app. A receipt proves owned marker
creation, not SQLite initialization or application readiness. No credentials are
read here.
"""
import argparse
import json
import os
from pathlib import Path
import platform
import signal
import stat
import subprocess
import sys
import tempfile
import time
import uuid

import fixture
import package
import profile as stage04_profile


def private_json(path, value):
    descriptor = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
    with os.fdopen(descriptor, "w") as output:
        json.dump(value, output, indent=2)
        output.write("\n")


def validate_path(path, owned, tls=None):
    if not path.is_absolute() or path != path.resolve() or not path.parent.is_dir():
        raise RuntimeError("Workspace profile must have a canonical absolute path and an existing parent")
    if path.exists():
        marker_path = path / ".dbunk-native-stage04"
        if not path.is_dir() or marker_path.is_symlink() or not marker_path.is_file():
            raise RuntimeError("Existing directory is not an owned stage04 profile")
        with marker_path.open("rb") as source:
            encoded = source.read(8193)
        if len(encoded) > 8192:
            raise RuntimeError("Workspace profile marker is oversized")
        marker = json.loads(encoded)
        if marker.get("version") != 1 or marker.get("path") != str(path):
            raise RuntimeError("Workspace profile marker identity differs")
        if marker.get("fixtures") != stage04_profile.manifest(owned, tls):
            raise RuntimeError("Workspace profile belongs to a different fixture; refusing to launch")
    # The Rust constructor performs authoritative ownership, private file,
    # marker/SQLite identity and exclusive lock checks before any credential I/O.


GENERAL_MARKER = ".dbunk-native-profile"
GENERAL_RECEIPT = "general-profile-owner.json"
GENERAL_KIND = "general-postgres"
RECEIPT_KIND = "dbunk-native-general-verification-owner"
GENERAL_FILES = {
    GENERAL_MARKER, ".dbunk-native-lock", "launch.json", "dbunk.sqlite",
    "dbunk.sqlite-wal", "dbunk.sqlite-shm",
}
MARKER_FIELDS = {"version", "kind", "path", "profile_id", "credential_namespace"}


def canonical_path(path, label):
    if not path.is_absolute() or path != path.resolve() or not path.parent.is_dir():
        raise RuntimeError(f"{label} must have a canonical absolute path and an existing parent")


def private_read_json(path, label, limit=8192):
    """Inspect only bounded metadata; never open profile credentials or SQLite."""
    canonical_path(path, label)
    descriptor = os.open(path, os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK)
    with os.fdopen(descriptor, "rb") as source:
        info = os.fstat(source.fileno())
        if (not stat.S_ISREG(info.st_mode) or info.st_uid != os.getuid()
                or stat.S_IMODE(info.st_mode) != 0o600 or info.st_nlink != 1):
            raise RuntimeError(f"{label} must be a private owned regular file with one link")
        encoded = source.read(limit + 1)
    if len(encoded) > limit:
        raise RuntimeError(f"{label} is oversized")
    value = json.loads(encoded, object_pairs_hook=unique_fields)
    if not isinstance(value, dict):
        raise RuntimeError(f"{label} must be a JSON object")
    return value


def unique_fields(pairs):
    value = {}
    for key, item in pairs:
        if key in value:
            raise RuntimeError("Duplicate JSON ownership field")
        value[key] = item
    return value


def uuid4_identity(value):
    if not isinstance(value, str):
        return False
    try:
        parsed = uuid.UUID(value)
    except ValueError:
        return False
    return parsed.version == 4 and str(parsed) == value


def general_marker(path):
    canonical_path(path, "General profile")
    info = path.lstat()
    if (not stat.S_ISDIR(info.st_mode) or info.st_uid != os.getuid()
            or stat.S_IMODE(info.st_mode) != 0o700):
        raise RuntimeError("General profile must be a private owned directory")
    for entry in path.iterdir():
        info = entry.lstat()
        if (entry.name not in GENERAL_FILES or not stat.S_ISREG(info.st_mode)
                or info.st_uid != os.getuid() or info.st_nlink != 1
                or stat.S_IMODE(info.st_mode) != 0o600):
            raise RuntimeError("General profile contains foreign, linked, non-private or mixed fixture files")
    marker = private_read_json(path / GENERAL_MARKER, "General profile marker")
    if (set(marker) != MARKER_FIELDS or type(marker.get("version")) is not int
            or marker["version"] != 1 or marker.get("kind") != GENERAL_KIND
            or marker.get("path") != str(path)
            or not uuid4_identity(marker.get("profile_id"))
            or not uuid4_identity(marker.get("credential_namespace"))
            or marker["profile_id"] == marker["credential_namespace"]):
        raise RuntimeError("General profile marker identity differs or has an unsupported version")
    return marker


def outside_profile(path, profile, label):
    canonical_path(path, label)
    if path == profile or path.is_relative_to(profile) or profile.is_relative_to(path):
        raise RuntimeError(f"{label} must be distinct from and outside the profile")


def validate_general(path, owned, tls, *, create=False, owner=None):
    canonical_path(path, "General profile")
    if create:
        if owner is not None:
            raise RuntimeError("General profile create and owner receipt flags cannot be mixed")
        if os.path.lexists(path):
            raise RuntimeError("General profile creation requires an absent path")
        return None
    if owner is None:
        raise RuntimeError("Existing general profile requires a prior launcher ownership receipt")
    outside_profile(owner, path, "General profile receipt")
    receipt = private_read_json(owner, "General profile receipt")
    marker = general_marker(path)
    expected = {"version", "kind", "marker", "fixtures", "launch_id", "executable_sha256"}
    if (set(receipt) != expected or type(receipt.get("version")) is not int
            or receipt["version"] != 1 or receipt.get("kind") != RECEIPT_KIND
            or receipt.get("marker") != marker
            or receipt.get("fixtures") != stage04_profile.manifest(owned, tls)
            or not uuid4_identity(receipt.get("launch_id"))
            or not valid_hash(receipt.get("executable_sha256"))):
        raise RuntimeError("General profile receipt identity, version or owned fixtures differ")
    intent = private_read_json(owner.parent / "general-profile-intent.json", "General profile creation intent")
    if type(intent.get("version")) is not int or intent != {
        "version": 1, "mode": "create-general-profile", "profile": str(path),
        "fixtures": receipt["fixtures"], "launch_id": receipt["launch_id"],
        "executable_sha256": receipt["executable_sha256"],
    }:
        raise RuntimeError("General profile receipt does not match its launcher creation intent")
    return receipt


def valid_hash(value):
    return isinstance(value, str) and len(value) == 64 and all(c in "0123456789abcdef" for c in value)


def wait_general_marker(path, child, timeout=10):
    deadline = time.monotonic() + timeout
    while True:
        code = child.poll()
        if code is not None and code != 0:
            raise RuntimeError(f"Native general profile creation exited {code} before ownership receipt")
        if os.path.lexists(path / GENERAL_MARKER):
            try:
                return general_marker(path)
            except json.JSONDecodeError:
                # create_new makes the file visible before its bounded write
                # completes. Only incomplete JSON is retried, never ownership.
                if code is not None:
                    raise RuntimeError("Native process left an incomplete general profile marker") from None
        if code is not None:
            raise RuntimeError("Native process exited before creating the general profile marker")
        if time.monotonic() >= deadline:
            raise RuntimeError("Timed out waiting for the general profile marker")
        time.sleep(min(0.05, max(0, deadline - time.monotonic())))


def bundle_executable(bundle):
    bundle = Path(os.path.abspath(bundle))
    if bundle != bundle.resolve():
        raise RuntimeError("Bundle must be canonical without symlinks")
    marker = bundle.parent / package.MARKER
    identity = json.loads(marker.read_text())
    if identity.get("bundle_id") != package.BUNDLE_ID or identity.get("bundle") != str(bundle):
        raise RuntimeError("Bundle is not a marked isolated native package")
    executable = bundle / "Contents/MacOS/dbunk-native"
    if identity.get("executable") != str(executable) or executable.is_symlink():
        raise RuntimeError("Packaged executable identity differs")
    for relative, expected in identity["files_sha256"].items():
        item = bundle / relative
        if not item.resolve().is_relative_to(bundle) or item.is_symlink() or package.digest(item) != expected:
            raise RuntimeError("Packaged resource identity changed")
    if "Contents/MacOS/dbunk-native" not in identity["files_sha256"]:
        raise RuntimeError("Package marker is missing the executable hash")
    return executable



def recheck_fixtures(owned, tls):
    """Fence a long build or preparation against replacement of the owned fixture."""
    current, _ = fixture.check()
    current_tls = None
    if tls is not None:
        import tls_fixture
        current_tls_owned, _ = tls_fixture.check()
        current_tls = tls_fixture.manifest(current_tls_owned)
    if stage04_profile.manifest(current, current_tls) != stage04_profile.manifest(owned, tls):
        raise RuntimeError("Owned fixture identity changed during launch preparation; refusing to spawn")


def launch(path, *, bundle=None, out=None, with_tls=False, no_build=False, create_general=False, general_owner=None):
    if sys.platform != "darwin" or platform.machine() != "arm64":
        raise RuntimeError("The native workspace requires Apple Silicon macOS")
    owned, _ = fixture.check()
    tls = None
    tls_owned = None
    if with_tls:
        import tls_fixture
        tls_owned, _ = tls_fixture.check()
        tls = tls_fixture.manifest(tls_owned)
    general = create_general or general_owner is not None
    if general:
        validate_general(path, owned, tls, create=create_general, owner=general_owner)
    else:
        validate_path(path, owned, tls)
    if bundle:
        executable = bundle_executable(bundle)
    else:
        if not no_build:
            subprocess.run(["cargo", "+1.98.1", "build", "--release", "--locked"], cwd=fixture.ROOT / "apps/native", check=True)
        executable = (fixture.ROOT / "apps/native/target/release/dbunk-native").resolve()
        if not executable.is_file():
            raise RuntimeError("Native release executable is missing; build it before using --no-build")
    evidence = Path(os.path.abspath(out)) if out else fixture.STATE.parent / "workspace-evidence" / str(uuid.uuid4())
    if evidence != evidence.resolve():
        raise RuntimeError("Evidence path must be canonical")
    if evidence == path or evidence.is_relative_to(path):
        raise RuntimeError("Evidence must be outside the private workspace profile")
    evidence.mkdir(mode=0o700, parents=True, exist_ok=False)
    executable_hash = package.digest(executable)
    mode = "create-general-profile" if create_general else "general-profile" if general else "stage04-fixture"
    receipt_path = evidence / GENERAL_RECEIPT if create_general else general_owner
    intent = None
    if create_general:
        intent = {
            "version": 1, "mode": mode, "profile": str(path),
            "fixtures": stage04_profile.manifest(owned, tls), "launch_id": str(uuid.uuid4()),
            "executable_sha256": executable_hash,
        }
        private_json(evidence / "general-profile-intent.json", intent)
    baseline = fixture.backend_count()
    tls_baseline = tls_fixture.backend_count(tls_owned) if tls_owned else None
    child = None
    print(f"Target: owned fixture {fixture.PROJECT} {fixture.ENDPOINT} instance={owned['instance']}", flush=True)
    if tls:
        print(f"TLS target: {tls['fixture']} 127.0.0.1:15433/dbunk_tls_demo instance={tls['instance']}", flush=True)
    if general:
        print(f"Explicit general PostgreSQL profile ({mode}): {path}", flush=True)
        print("Verification ownership is scoped to the checked fixtures; the general app profile does not restrict endpoints.", flush=True)
    else:
        print(f"Persistent isolated stage04 profile: {path}", flush=True)
    print(f"Executable: {executable}; evidence: {evidence}", flush=True)
    try:
        with tempfile.TemporaryDirectory(prefix="dbunk-native-workspace-launch-") as temporary:
            working_directory = Path(temporary).resolve()
            manifest = working_directory / "fixture.json"
            private_json(manifest, stage04_profile.manifest(owned, tls))
            env = os.environ.copy()
            env.pop("DBUNK_NATIVE_VERIFY", None)
            with (evidence / "native.log").open("w") as output:
                if general:
                    command = [str(executable), "--create-native-profile" if create_general else "--native-profile", str(path)]
                else:
                    command = [str(executable), "--workspace-profile", str(path), "--fixture-manifest", str(manifest)]
                recheck_fixtures(owned, tls)
                child = subprocess.Popen(command, cwd=working_directory, env=env, stdout=output, stderr=subprocess.STDOUT)
                private_json(evidence / "identity.json", {
                    "version": 1, "pid": child.pid, "executable": str(executable),
                    "executable_sha256": executable_hash, "profile": str(path),
                    "profile_mode": mode, "general_profile_owner": str(receipt_path) if receipt_path else None,
                    "fixture_instance": owned["instance"], "cwd": str(working_directory),
                    "tls_fixture_instance": tls["instance"] if tls else None,
                    "bundle": str(bundle) if bundle else None,
                })
                print(f"Native workspace PID: {child.pid}", flush=True)
                if create_general:
                    # The app writes its marker before SQLite initialization;
                    # this receipt establishes ownership, never a Ready gate.
                    marker = wait_general_marker(path, child)
                    private_json(receipt_path, {
                        "version": 1, "kind": RECEIPT_KIND, "marker": marker,
                        "fixtures": intent["fixtures"], "launch_id": intent["launch_id"],
                        "executable_sha256": executable_hash,
                    })
                    print(f"General profile ownership receipt: {receipt_path}", flush=True)
                code = child.wait()
            if code:
                raise RuntimeError(f"Native workspace exited {code}; inspect {evidence / 'native.log'}")
            fixture.wait_baseline(baseline)
            if tls_owned:
                tls_fixture.wait_baseline(tls_baseline, tls_owned)
            private_json(evidence / "teardown.json", {
                "exit_code": code, "backend_baseline": baseline,
                "backend_final": fixture.backend_count(), "fixture_instance": owned["instance"],
                "tls_backend_baseline": tls_baseline,
                "tls_backend_final": tls_fixture.backend_count(tls_owned) if tls_owned else None,
                "profile_retained": str(path),
            })
    finally:
        if child and child.poll() is None:
            child.send_signal(signal.SIGTERM)
            try:
                child.wait(timeout=5)
            except subprocess.TimeoutExpired:
                child.kill()
                child.wait()
            print("Forced failure cleanup is not a successful quit gate", file=sys.stderr)
    print(f"Workspace quit; drafts and profile retained at {path}", flush=True)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("path", type=Path, help="canonical profile path; stage04 fixture mode unless a general-profile option is explicit")
    parser.add_argument("--bundle", type=Path, help="marked package.py app bundle instead of building a CLI executable")
    parser.add_argument("--out", type=Path, help="new evidence directory")
    parser.add_argument("--tls", action="store_true", help="include only the separately verified owned TLS fixture")
    parser.add_argument("--no-build", action="store_true", help="launch the existing native release executable for iterative QA; its exact hash is recorded")
    general = parser.add_mutually_exclusive_group()
    general.add_argument("--create-general-profile", action="store_true", help="explicitly create a new general PostgreSQL profile at an absent path")
    general.add_argument("--general-profile-owner", type=Path, metavar="RECEIPT", help="reopen only a general profile matching a prior launcher ownership receipt")
    args = parser.parse_args()
    launch(args.path, bundle=args.bundle, out=args.out, with_tls=args.tls, no_build=args.no_build, create_general=args.create_general_profile, general_owner=args.general_profile_owner)


if __name__ == "__main__":
    try:
        main()
    except (RuntimeError, OSError, ValueError, subprocess.SubprocessError) as error:
        sys.exit(f"Native workspace launch failed: {error}")
