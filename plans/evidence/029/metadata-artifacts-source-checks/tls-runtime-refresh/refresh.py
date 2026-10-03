"""One-time guarded repair of this named owned fixture; preserves prior runtime."""
import fcntl, json, os, signal, socket, subprocess, sys, time
from pathlib import Path
ROOT = Path(__file__).resolve().parents[5]
sys.path.insert(0, str(ROOT / "tools/native"))
import fixture, tls_fixture as tls, tls_runtime as runtime
out = Path(__file__).resolve().parent
with (tls.STATE.parent / "tls-fixture.lock").open("a") as lock:
    fcntl.flock(lock, fcntl.LOCK_EX)
    owned = tls.state()
    assert owned["instance"] == "15151cfc-5885-4066-831f-2717ed9b4587"
    assert owned["phase"] == "running" and owned["pid"] == 35510
    root = tls.directory(owned)
    assert owned["executable"] == str(runtime.PREFIX / "bin/postgres")
    marker = root / tls.MARKER
    assert not marker.is_symlink() and json.loads(marker.read_text()) == tls.immutable(owned)
    assert runtime.ROOT == runtime.ROOT.resolve() and not runtime.ROOT.is_symlink()
    runtime_marker = runtime.ROOT / "source-built.json"
    assert not runtime_marker.is_symlink()
    previous = json.loads(runtime_marker.read_text())
    current = runtime.inputs()
    assert previous["inputs"]["source"] == current["source"]
    assert previous["inputs"]["openssl_prefix"] == "/opt/homebrew/Cellar/openssl@3/3.6.4"
    assert current["openssl_prefix"] == "/opt/homebrew/Cellar/openssl@3/3.6.5"
    for path, digest in previous["inputs"]["openssl_sha256"].items():
        assert runtime.digest(Path(path)) == digest
    for name, digest in previous["executables_sha256"].items():
        binary = runtime.PREFIX / "bin" / name
        assert not binary.is_symlink() and runtime.digest(binary) == digest
    for relative, digest in owned["files_sha256"].items():
        path = root / relative
        assert not path.is_symlink() and path.resolve().is_relative_to(root)
        assert runtime.digest(path) == digest
    def same_process():
        command = fixture.run(["/bin/ps", "-p", str(owned["pid"]), "-o", "command="])
        assert command == f"{owned['executable']} -D {root / 'data'}"
        lines = (root / "data/postmaster.pid").read_text().splitlines()
        assert int(lines[0]) == owned["pid"] and lines[1] == str(root / "data")
        assert lines[3] == str(tls.PORT) and lines[5] == "127.0.0.1"
    same_process()
    backup = runtime.ROOT.with_name("tls-runtime-retired-3.6.4-20261003")
    assert not backup.exists() and not backup.is_symlink()
    (out / "before.json").write_text(json.dumps({"fixture": tls.manifest(owned), "pid": owned["pid"], "previous_runtime": previous, "current_inputs": current, "retained_runtime": str(backup)}, indent=2) + "\n")
    same_process()
    os.kill(owned["pid"], signal.SIGINT)  # PostgreSQL fast, transactional shutdown.
    deadline = time.monotonic() + 20
    while time.monotonic() < deadline:
        result = subprocess.run(["/bin/ps", "-p", str(owned["pid"]), "-o", "command="], capture_output=True)
        if result.returncode == 1 and not (root / "data/postmaster.pid").exists():
            break
        time.sleep(.2)
    else:
        raise RuntimeError("Owned shutdown did not join; all files retained")
    owned.update(phase="prepared")
    owned.pop("pid")
    tls.private_json(tls.STATE, owned, replace=True)
    runtime.ROOT.rename(backup)
    print("Old owned TLS process stopped; prior runtime retained at", backup, flush=True)
    runtime.prepare()
    assert runtime.inputs() == current
    tls.validate(owned, running=False)
    with socket.socket() as probe:
        probe.bind(("127.0.0.1", tls.PORT))
    with (root / "postgres.log").open("a") as log:
        process = subprocess.Popen([owned["executable"], "-D", str(root / "data")], stdout=log, stderr=subprocess.STDOUT, start_new_session=True)
    owned.update(phase="running", pid=process.pid)
    tls.private_json(tls.STATE, owned, replace=True)
    deadline = time.monotonic() + 15
    while time.monotonic() < deadline:
        if process.poll() is not None:
            raise RuntimeError("Owned restarted server exited; retained for inspection")
        try:
            tls.check()
            break
        except (RuntimeError, FileNotFoundError):
            time.sleep(.1)
    else:
        raise RuntimeError("Owned existing database did not become ready")
    tls.wait_baseline(0, owned)
    (out / "after.json").write_text(json.dumps({"fixture": tls.description(owned), "runtime": json.loads((runtime.ROOT / "source-built.json").read_text()), "other_backends": tls.backend_count(owned), "existing_database_reused": True}, indent=2) + "\n")
    tls.matrix()
