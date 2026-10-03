"""Separate SSL-enabled build of the already pinned private PostgreSQL source.

Never changes the plain fixture runtime, installs packages, or starts a service.
"""
import hashlib
import json
import os
from pathlib import Path
import platform
import subprocess
import sys
import tarfile

import runtime

ROOT = runtime.ROOT.parent / "tls-runtime"
PREFIX = ROOT / "installed"
EXECUTABLES = ("postgres", "initdb", "psql", "pg_ctl")


def digest(path):
    with path.open("rb") as source:
        if hasattr(hashlib, "file_digest"):
            return hashlib.file_digest(source, "sha256").hexdigest()
        # Python < 3.11: same SHA-256 over bounded chunks.
        hasher = hashlib.sha256()
        for chunk in iter(lambda: source.read(1024 * 1024), b""):
            hasher.update(chunk)
        return hasher.hexdigest()


def inputs():
    openssl = Path("/opt/homebrew/opt/openssl@3").resolve()
    files = [openssl / relative for relative in ("bin/openssl", "lib/libssl.3.dylib", "lib/libcrypto.3.dylib", "include/openssl/ssl.h")]
    if any(not path.is_file() for path in files):
        raise RuntimeError("An existing OpenSSL development runtime is required; no packages were installed")
    return {
        "source": runtime.SOURCE,
        "openssl_prefix": str(openssl),
        "openssl_sha256": {str(path): digest(path) for path in files},
    }


def validate():
    if ROOT != ROOT.resolve() or ROOT.is_symlink():
        raise RuntimeError("TLS runtime directory must be canonical without symlinks")
    marker = ROOT / "source-built.json"
    if marker.is_symlink():
        raise RuntimeError("TLS runtime ownership marker is a symlink")
    value = json.loads(marker.read_text())
    if value["inputs"] != inputs():
        raise RuntimeError("TLS runtime source or OpenSSL identity changed; retained for inspection")
    for name in EXECUTABLES:
        binary = PREFIX / "bin" / name
        if binary.is_symlink() or digest(binary) != value["executables_sha256"][name]:
            raise RuntimeError("TLS runtime executable identity changed")
    return PREFIX


def prepare():
    if sys.platform != "darwin" or platform.machine() != "arm64":
        raise RuntimeError("Owned TLS runtime currently requires Apple Silicon macOS")
    if ROOT != ROOT.resolve() or ROOT.is_symlink():
        raise RuntimeError("TLS runtime path must not contain symlinks")
    if (ROOT / "source-built.json").exists():
        return validate()
    source_inputs = inputs()
    archive = runtime.ROOT / f"postgresql-{runtime.VERSION}.tar.bz2"
    if not archive.is_file() or digest(archive) != runtime.SHA256:
        raise RuntimeError("Verified PostgreSQL source archive is required; no downloads or packages were requested")
    ROOT.mkdir(parents=True, mode=0o700, exist_ok=True)
    build = ROOT / f"postgresql-{runtime.VERSION}"
    if build.exists():
        raise RuntimeError("Incomplete TLS build retained; inspect it before retrying")
    with tarfile.open(archive, "r:bz2") as source:
        source.extractall(ROOT, filter="data")
    openssl = Path(source_inputs["openssl_prefix"])
    env = dict(os.environ, CPPFLAGS=f"-I{openssl / 'include'}", LDFLAGS=f"-L{openssl / 'lib'} -Wl,-rpath,{openssl / 'lib'}")
    print(f"Building separate PostgreSQL {runtime.VERSION} TLS runtime at {PREFIX}", flush=True)
    with (ROOT / "source-build.log").open("w") as log:
        for command in [
            ["./configure", f"--prefix={PREFIX}", "--without-icu", "--without-readline", "--without-zlib", "--with-ssl=openssl"],
            ["make", "-j4"], ["make", "install"],
        ]:
            result = subprocess.run(command, cwd=build, env=env, stdout=log, stderr=subprocess.STDOUT)
            if result.returncode:
                raise RuntimeError(f"TLS runtime build failed; inspect {ROOT / 'source-build.log'}")
    value = {"inputs": source_inputs, "executables_sha256": {name: digest(PREFIX / "bin" / name) for name in EXECUTABLES}}
    (ROOT / "source-built.json").write_text(json.dumps(value, indent=2) + "\n")
    return validate()
