#!/usr/bin/env python3
"""Prepare/run stage04 lifecycle races only after human desktop checks finish.

Uses a NEW isolated profile, three repetitions of each active-query lifecycle
case, then deliberately SIGKILLs the owned app only after Saved is observed.
The second process must restore exact acknowledged SQL without connecting.
Later phases inject a reversible SQLite writer lock and a missing document
connection binding, then verify explicit Retry and lossless restoration.
No Keychain operation, product hook, or existing table mutation is involved.
"""
import argparse
import json
import os
from pathlib import Path
import signal
import sqlite3
from contextlib import contextmanager
import subprocess
import sys

import fixture
import package
import workspace_launch
import workspace_recovery
from workspace_performance import guard, wait_launch

ROOT = fixture.ROOT
DRAFT = "-- acknowledged force recovery α\nSELECT 42 AS durable_answer;\n"
RETRY_DRAFT = "-- retained through SQLite busy α\nSELECT 43 AS retried_draft;\n"


def require_new(path, label):
    workspace_recovery.canonical(path)
    if path.exists() or not path.parent.is_dir():
        raise RuntimeError(f"{label} must be new with an existing canonical parent")


def check_saved_profile(path, expected_sql=DRAFT):
    # Reuse the exclusive-lock, private-file, marker/SQLite and owned-fixture
    # checks. The forced process must be gone before this read is admitted.
    with workspace_recovery.closed_profile(path) as (connection, marker):
        row = connection.execute("SELECT value,updated_at FROM ui_state WHERE key=?", (workspace_recovery.KEY,)).fetchone()
        if row is None:
            raise RuntimeError("Acknowledged workspace record is missing")
        record = json.loads(row[0])
        documents = record.get("snapshot", {}).get("documents", [])
        if record.get("version") not in (1, 2) or len(documents) != 1 or documents[0].get("name") != "Race draft" or documents[0].get("sql") != expected_sql:
            raise RuntimeError("Forced termination did not retain the exact acknowledged draft")
        settings = dict(connection.execute("SELECT key,value FROM app_settings"))
        if settings.get("credentialStorageMode") != "plain-sqlite":
            raise RuntimeError("Race profile did not finish back in plain SQLite mode")
        return {"profile_id": marker["profile_id"], "sql_sha256": workspace_recovery.digest(expected_sql),
                "record_sha256": workspace_recovery.digest(row[0]), "revision": row[1],
                "other_state_sha256": workspace_recovery.other_state(connection)}


@contextmanager
def hold_sqlite_writer(identity, fresh_profile, profile_id):
    """Inject contention in this runner's new profile, never write stored rows."""
    guard(identity)
    path = Path(identity["profile"])
    if path != fresh_profile:
        raise RuntimeError("SQLite injection profile differs from the runner's fresh profile")
    for item in path.iterdir():
        if item.name not in workspace_recovery.FILES:
            raise RuntimeError("SQLite injection profile contains a foreign entry")
        workspace_recovery.private_file(item)
    marker = json.loads((path / ".dbunk-native-stage04").read_text())
    if marker.get("profile_id") != profile_id or marker.get("path") != str(path):
        raise RuntimeError("SQLite injection profile identity changed")
    connection = sqlite3.connect(f"file:{path / 'dbunk.sqlite'}?mode=rw", uri=True, timeout=0)
    try:
        stored = connection.execute("SELECT value FROM app_settings WHERE key='native.stage04.identity'").fetchone()
        if stored is None or json.loads(stored[0]) != marker:
            raise RuntimeError("SQLite injection identity differs from its profile marker")
        connection.execute("BEGIN IMMEDIATE")
        before = connection.execute("SELECT value,updated_at FROM ui_state WHERE key=?", (workspace_recovery.KEY,)).fetchone()
        if before is None:
            raise RuntimeError("SQLite injection requires an existing durable draft")
        yield connection, before
    finally:
        # ROLLBACK releases the reserved writer lock on success, probe failure
        # or interruption; no record mutation or journal-mode change is made.
        connection.rollback()
        connection.close()


def run(path, out, bundle):
    if sys.platform != "darwin":
        raise RuntimeError("Workspace acceptance requires macOS")
    require_new(path, "Workspace profile")
    require_new(out, "Evidence directory")
    if out.is_relative_to(path) or path.is_relative_to(out):
        raise RuntimeError("Profile and evidence directories must be separate")
    executable = workspace_launch.bundle_executable(bundle) if bundle else ROOT / "apps/native/target/release/dbunk-native"
    if not executable.is_file() or executable != executable.resolve():
        raise RuntimeError("Build the release executable before this runner; it never rebuilds")
    owned, _ = fixture.check()
    baseline = fixture.backend_count()
    if baseline != 0:
        raise RuntimeError("Other fixture sessions are active; wait until human and backend checks finish")
    out.mkdir(mode=0o700)
    probe = out / "workspace-accessibility"
    subprocess.run(["swiftc", str(ROOT / "tools/measure/workspace-accessibility.swift"), "-o", str(probe)], check=True, timeout=60)
    workspace_launch.private_json(out / "environment.json", {
        "fixture_instance": owned["instance"], "baseline": baseline,
        "profile": str(path), "executable": str(executable), "executable_sha256": package.digest(executable),
        "source_sha256": {name: package.digest(ROOT / name) for name in [
            "tools/native/workspace_races.py", "tools/native/workspace_launch.py",
            "tools/native/workspace_recovery.py", "tools/native/workspace_performance.py",
            "tools/measure/workspace-accessibility.swift"]},
        "scope": "Three repetitions per active-query overlap; acknowledged draft recovery after intentional process kill; SQLite save/retry and missing-binding restoration. No human accessibility claim.",
    })
    print(f"Target: NEW profile {path}; owned fixture {fixture.ENDPOINT} instance={owned['instance']}", flush=True)
    print("Foreground automation begins. Human VoiceOver/IME and all other fixture checks must be finished.", flush=True)

    def phase(name, actions, *, kill_after_saved=False, sqlite_profile_id=None):
        evidence = out / name
        identity = None
        launcher = None
        completed = False
        with (out / f"{name}-launcher.log").open("w") as launch_log:
            command = [sys.executable, str(ROOT / "tools/native/workspace_launch.py"), str(path), "--no-build", "--out", str(evidence)]
            if bundle:
                command.extend(["--bundle", str(bundle)])
            try:
                launcher = subprocess.Popen(command, stdout=launch_log, stderr=subprocess.STDOUT)
                identity = wait_launch(launcher, evidence)
                if identity["executable_sha256"] != package.digest(executable):
                    raise RuntimeError("Launch executable differs from recorded race build")
                for number, action in enumerate(actions):
                    guard(identity)
                    with (out / f"{name}-{number:02d}-{action}.log").open("w") as log:
                        probe_command = [str(probe), str(evidence / "identity.json"), action]
                        if action == "sqlite-fail-save":
                            if sqlite_profile_id is None:
                                raise RuntimeError("SQLite injection requires the previously verified fresh profile ID")
                            with hold_sqlite_writer(identity, path, sqlite_profile_id) as (connection, before):
                                subprocess.run(probe_command, stdout=log, stderr=subprocess.STDOUT, check=True, timeout=100)
                                after = connection.execute("SELECT value,updated_at FROM ui_state WHERE key=?", (workspace_recovery.KEY,)).fetchone()
                                if after != before:
                                    raise RuntimeError("Failed save replaced the last durable draft")
                                workspace_launch.private_json(out / "sqlite-failure.json", {
                                    "profile_id": sqlite_profile_id, "last_durable_unchanged": True,
                                    "record_sha256": workspace_recovery.digest(before[0]), "revision": before[1],
                                    "injection": "BEGIN IMMEDIATE without any record writes",
                                })
                            # This proof is recorded only after ROLLBACK/close.
                            workspace_launch.private_json(out / "sqlite-lock-released.json", {"released_before_retry": True})
                        else:
                            subprocess.run(probe_command, stdout=log, stderr=subprocess.STDOUT, check=True, timeout=100)
                if kill_after_saved:
                    guard(identity)
                    os.kill(identity["pid"], signal.SIGKILL)
                    code = launcher.wait(timeout=30)
                    if code == 0:
                        raise RuntimeError("Intentional forced termination was incorrectly reported as graceful")
                    fixture.wait_baseline(baseline)
                    workspace_launch.private_json(out / "intentional-termination.json", {
                        "pid": identity["pid"], "signal": "SIGKILL", "launcher_exit_code": code,
                        "saved_acknowledged": True, "backend_final": fixture.backend_count(),
                        "graceful_shutdown": False,
                    })
                else:
                    if launcher.wait(timeout=30) != 0:
                        raise RuntimeError("Normal reopen/quit did not complete; inspect launcher evidence")
                    if not (evidence / "teardown.json").is_file():
                        raise RuntimeError("Normal reopen lacks joined teardown evidence")
                completed = True
            finally:
                if launcher is not None and launcher.poll() is None:
                    if identity is None:
                        # Do not terminate an unknown descendant. Retain the
                        # launcher/profile for diagnosis if identity was never
                        # published; no broad process-name cleanup is allowed.
                        print("Launcher still active without verified child identity; manual diagnosis required", file=sys.stderr)
                    else:
                        guard(identity)
                        os.kill(identity["pid"], signal.SIGTERM)
                        try:
                            launcher.wait(timeout=10)
                        except subprocess.TimeoutExpired:
                            guard(identity)
                            os.kill(identity["pid"], signal.SIGKILL)
                            launcher.wait(timeout=10)
                        print("Failure cleanup is not successful lifecycle acceptance", file=sys.stderr)
                if not completed:
                    print(f"Incomplete phase retained: {evidence}", file=sys.stderr)

    actions = ["race-setup"]
    for _ in range(3):
        actions.extend(["race-close", "race-reconnect", "race-credentials"])
    actions.append("force-saved")
    phase("races-and-force", actions, kill_after_saved=True)
    durable = check_saved_profile(path)
    workspace_launch.private_json(out / "durable-after-kill.json", durable)
    phase("forced-reopen", ["force-reopen"])
    reopened = check_saved_profile(path)
    if durable["sql_sha256"] != reopened["sql_sha256"] or durable["other_state_sha256"] != reopened["other_state_sha256"]:
        raise RuntimeError("Reopen changed acknowledged SQL, credentials or connection metadata")
    phase("sqlite-retry", ["sqlite-ready", "sqlite-fail-save", "sqlite-retry"], sqlite_profile_id=durable["profile_id"])
    retried = check_saved_profile(path, RETRY_DRAFT)
    if reopened["other_state_sha256"] != retried["other_state_sha256"]:
        raise RuntimeError("SQLite save/retry changed credentials or connection metadata")
    workspace_launch.private_json(out / "durable-after-retry.json", retried)
    missing_receipt = out / "missing-binding.json"
    workspace_recovery.seed(path, "missing", missing_receipt)
    phase("missing-binding", ["missing-reopen"])
    workspace_recovery.check(path, missing_receipt, "missing", None)
    workspace_launch.private_json(out / "summary.json", {
        "passed": True, "repetitions": 3,
        "cases": ["close tab with admitted long query", "disconnect active query and explicitly reconnect", "encrypted/plain SQLite conversion with admitted long query"],
        "forced_termination": "SIGKILL after observed Saved; exact SQL verified on disk and disconnected UI reopen",
        "sqlite_retry": "Reversible reserved writer lock surfaces failed save and blocks quit; explicit lock release precedes Retry and exact durable SQL check",
        "missing_binding": "Only the closed profile's document binding is replaced by a nonexistent UUID; disconnected restore/connect refusal preserve exact SQL and all connection metadata",
        "additional_teardowns": {name: json.loads((out / name / "teardown.json").read_text()) for name in ["sqlite-retry", "missing-binding"]},
        "normal_reopen_teardown": json.loads((out / "forced-reopen/teardown.json").read_text()),
        "limitations": ["No injected pending-open/driver barrier; overlap starts after UI Running acknowledgement", "No human VoiceOver or IME claim"],
    })
    print(f"PASS: workspace lifecycle, forced-draft recovery, SQLite retry and missing binding; evidence={out}")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("profile", type=Path, help="NEW canonical isolated profile")
    parser.add_argument("--out", type=Path, required=True, help="NEW canonical evidence directory")
    parser.add_argument("--bundle", type=Path, help="existing marked isolated bundle; otherwise existing release binary")
    parser.add_argument("--human-checks-finished", action="store_true", help="required acknowledgement that human desktop checks and other fixture work have finished")
    args = parser.parse_args()
    if not args.human_checks_finished:
        parser.error("wait for the human desktop checks and other fixture work, then pass --human-checks-finished")
    run(args.profile, args.out, args.bundle)


if __name__ == "__main__":
    try:
        main()
    except (RuntimeError, OSError, ValueError, KeyError, sqlite3.Error, subprocess.SubprocessError) as error:
        sys.exit(f"Native workspace races failed: {error}")
