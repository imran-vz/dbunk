#!/usr/bin/env python3
"""Build and verify an unsigned local DMG for an assembled native bundle.

Only new, owned paths are written: a DMG file, a private mountpoint and a
disposable Applications directory. The image is attached read-only without
Finder browsing and always detached. The app is not launched here; first-run
and joined-quit behavior need a separate window run.
"""
import argparse
import json
from pathlib import Path
import subprocess
import tempfile

import package


def new_path(path, label):
    if path != path.resolve():
        raise RuntimeError(f"{label} must be canonical and must not contain symlinks")
    if path.exists():
        raise FileExistsError(f"{label} already exists: {path}")
    if not path.parent.is_dir():
        raise RuntimeError(f"{label} parent must exist")
    return path


def bundle_hashes(bundle):
    return {
        str(path.relative_to(bundle)): package.digest(path)
        for path in sorted(bundle.rglob("*")) if path.is_file()
    }


def compare(expected, bundle, label):
    actual = bundle_hashes(bundle)
    if actual != expected:
        missing = sorted(set(expected) - set(actual))
        extra = sorted(set(actual) - set(expected))
        changed = sorted(key for key in expected.keys() & actual.keys() if expected[key] != actual[key])
        raise RuntimeError(f"{label} differs: missing={missing} extra={extra} changed={changed}")
    return len(actual)


def create(bundle_dir, image):
    """`bundle_dir` is the package output directory holding the marker and app."""
    identity = json.loads((bundle_dir / package.MARKER).read_text())
    bundle = bundle_dir / package.APP_NAME
    compare(identity["files_sha256"], bundle, "Source bundle")
    new_path(image, "DMG")
    with tempfile.TemporaryDirectory() as staging:
        stage = Path(staging).resolve() / "volume"
        stage.mkdir()
        subprocess.run(["ditto", str(bundle), str(stage / package.APP_NAME)], check=True)
        (stage / "Applications").symlink_to("/Applications")
        subprocess.run(
            ["hdiutil", "create", "-volname", "dbunk Native Preflight", "-srcfolder", str(stage),
             "-format", "UDZO", "-ov", "-quiet", str(image)],
            check=True,
        )
    return identity


def verify(image, identity, applications):
    """Attach read-only, verify hashes, copy to a disposable Applications dir."""
    new_path(applications, "Applications directory")
    expected = identity["files_sha256"]
    report = {"image": str(image), "image_sha256": package.digest(image)}
    with tempfile.TemporaryDirectory() as mounts:
        mountpoint = Path(mounts).resolve() / "mount"
        mountpoint.mkdir()
        subprocess.run(
            ["hdiutil", "attach", "-readonly", "-nobrowse", "-noautoopen", "-mountpoint",
             str(mountpoint), "-quiet", str(image)],
            check=True,
        )
        try:
            mounted = mountpoint / package.APP_NAME
            report["mounted_files"] = compare(expected, mounted, "Mounted bundle")
            probe = mounted / "Contents/.dbunk-write-probe"
            try:
                probe.write_text("x")
                report["read_only"] = False
            except OSError:
                report["read_only"] = True
            if not report["read_only"]:
                raise RuntimeError("Mounted DMG volume accepted a write")
            applications.mkdir(mode=0o700)
            copied = applications / package.APP_NAME
            subprocess.run(["ditto", str(mounted), str(copied)], check=True)
        finally:
            subprocess.run(["hdiutil", "detach", "-quiet", str(mountpoint)], check=True)
    report["copied_files"] = compare(expected, copied, "Copied bundle")
    report["copied_bundle"] = str(copied)
    report["launch"] = "not performed; first-run and joined-quit checks need a window run"
    return report


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--bundle-dir", required=True, type=Path, help="package.py --out directory")
    parser.add_argument("--image", required=True, type=Path)
    parser.add_argument("--applications", required=True, type=Path)
    args = parser.parse_args()
    identity = create(args.bundle_dir.resolve(), args.image)
    print(json.dumps(verify(args.image, identity, args.applications), indent=2))


if __name__ == "__main__":
    main()
