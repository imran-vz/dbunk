"""Build checksum-pinned official PostgreSQL source in a task-private prefix."""
import hashlib
import json
from pathlib import Path
import platform
import shutil
import subprocess
import sys
import tarfile
import urllib.request

ROOT = Path("/private/tmp/dbunk-native-comparison-runtime-20261003")
VERSION = "16.14"
SHA256 = "f6d077142737920858ce958ccdb75c6ee137a63b5b0853c70693d401ac7e3471"
URL = f"https://ftp.postgresql.org/pub/source/v{VERSION}/postgresql-{VERSION}.tar.bz2"
SOURCE = {"version": VERSION, "url": URL, "sha256": SHA256}
PREFIX = ROOT / "installed"


def prepare():
    if sys.platform != "darwin" or platform.machine() != "arm64":
        raise RuntimeError("Private PostgreSQL fallback requires Apple Silicon macOS; use an owned Docker fixture on other systems")
    if ROOT.is_symlink() or ROOT != ROOT.resolve():
        raise RuntimeError("Private runtime path must not contain symlinks")
    ROOT.mkdir(parents=True, exist_ok=True, mode=0o700)
    ready = ROOT / "source-built.json"
    if ready.exists():
        if json.loads(ready.read_text()) != SOURCE:
            raise RuntimeError("Private runtime source identity differs")
        return PREFIX
    archive = ROOT / f"postgresql-{VERSION}.tar.bz2"
    if not archive.exists():
        partial = archive.with_suffix(".partial")
        print(f"Downloading official PostgreSQL {VERSION} into {ROOT}", flush=True)
        with urllib.request.urlopen(URL, timeout=60) as response, partial.open("wb") as output:
            shutil.copyfileobj(response, output)
        partial.rename(archive)
    with archive.open("rb") as source:
        actual = hashlib.file_digest(source, "sha256").hexdigest()
    if actual != SHA256:
        raise RuntimeError("Private PostgreSQL source checksum mismatch")
    with tarfile.open(archive, "r:bz2") as source:
        source.extractall(ROOT, filter="data")
    build = ROOT / f"postgresql-{VERSION}"
    print(f"Building PostgreSQL {VERSION} with private prefix {PREFIX}", flush=True)
    with (ROOT / "source-build.log").open("w") as log:
        # These optional integrations are unnecessary for the loopback fixture;
        # omitting them avoids installing or altering any system packages.
        for command in [
            ["./configure", f"--prefix={PREFIX}", "--without-icu", "--without-readline", "--without-zlib"],
            ["make", "-j4"],
            ["make", "install"],
        ]:
            result = subprocess.run(command, cwd=build, stdout=log, stderr=subprocess.STDOUT)
            if result.returncode:
                raise RuntimeError(f"Private PostgreSQL build failed; inspect {ROOT / 'source-build.log'}")
    subprocess.run([str(PREFIX / "bin/postgres"), "--version"], check=True)
    ready.write_text(json.dumps(SOURCE, indent=2) + "\n")
    return PREFIX


if __name__ == "__main__":
    print(prepare())
