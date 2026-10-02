#!/usr/bin/env python3
"""Actual-window race and release-performance probes on the owned fixture only."""
import argparse
from contextlib import contextmanager
import json
import os
from pathlib import Path
import signal
import subprocess
import time

import fixture
import launch

ROOT = fixture.ROOT
PROBE = ROOT / "tools/native/.state/window-probe"
MEASURE = ROOT / "tools/measure/.build/release/measure"
EXECUTABLE = ROOT / "apps/native/target/release/dbunk-native"


def wait_for(child, log, predicate, description, timeout=15):
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        text = log.read_text()
        if predicate(text):
            return text
        if child.poll() is not None:
            raise RuntimeError(f"Host exited before {description}: {child.returncode}")
        time.sleep(0.025)
    raise RuntimeError(f"Timed out waiting for {description}; inspect {log}")


@contextmanager
def window(evidence, scenario=None):
    owned, _ = fixture.check()
    baseline = fixture.backend_count()
    profile, marker = launch.create_profile()
    evidence.mkdir(parents=True, exist_ok=False)
    child = None
    passed = False
    try:
        env = os.environ.copy()
        env.pop("DBUNK_NATIVE_VERIFY", None)
        if scenario:
            env["DBUNK_NATIVE_VERIFY"] = scenario
        with (evidence / "native.log").open("w") as output:
            child = subprocess.Popen([str(EXECUTABLE), "--profile", str(profile)], env=env, stdout=output, stderr=subprocess.STDOUT)
            identity = {"pid": child.pid, "executable": str(EXECUTABLE), "profile_id": marker["profile_id"], "fixture_instance": owned["instance"], "scenario": scenario}
            (profile / "launch.json").write_text(json.dumps(identity) + "\n")
            (evidence / "identity.json").write_text(json.dumps(identity) + "\n")
            print(f"Window {evidence.name}: PID {child.pid}; owned fixture {owned['instance']}", flush=True)
            yield child, profile
            code = child.wait(timeout=7)
            if code:
                raise RuntimeError(f"Native exit {code}")
        fixture.wait_baseline(baseline)
        log = (evidence / "native.log").read_text()
        if "Native cleanup failed" in log or "remaining_bytes=" not in log:
            raise RuntimeError("Missing successful queue/cleanup evidence")
        for line in log.splitlines():
            if "remaining_bytes=" in line and not line.endswith("remaining_bytes=0"):
                raise RuntimeError(f"Mailbox leaked: {line}")
        (evidence / "native.txt").write_text(log)
        (evidence / "teardown.json").write_text(json.dumps({"exit_code": code, "backend_baseline": baseline, "backend_final": fixture.backend_count(), "fixture_instance": owned["instance"]}) + "\n")
        passed = True
    finally:
        if child and child.poll() is None:
            child.send_signal(signal.SIGTERM)
            try:
                child.wait(timeout=5)
            except subprocess.TimeoutExpired:
                child.kill()
                child.wait()
        if passed:
            launch.cleanup_profile(profile, marker)
        else:
            print(f"FAILED: retained marked profile {profile}", flush=True)


def step(child, profile, evidence, action):
    with (evidence / "accessibility.txt").open("a") as output:
        subprocess.run([str(PROBE), "--native-fixture", str(child.pid), str(profile), f"--window-step={action}"], stdout=output, stderr=subprocess.STDOUT, check=True, timeout=25)


RACES = {
    "connect-close": "connect",
    "reconnect-close": "reconnect",
    "credit-close": "credit",
    "credit-quit": "credit",
    "credit-stop": "credit",
    "saturated-close": "saturated",
    "saturated-replace": "saturated",
    "metadata-failure": "saturated",
    "streaming-close": "streaming",
}


def race(evidence, name):
    with window(evidence, RACES[name]) as (child, profile):
        def action(value):
            step(child, profile, evidence, value)

        def barrier(predicate, description):
            return wait_for(child, evidence / "native.log", predicate, description)

        if name == "connect-close":
            barrier(lambda log: "VERIFY open-pending 1" in log, "admitted pending open")
        else:
            action("ready")
            if name == "reconnect-close":
                action("reconnect")
                barrier(lambda log: "VERIFY open-pending 2" in log, "pending reconnect after old owner cleanup")
            elif name == "streaming-close":
                action("run-stream")
                barrier(lambda log: "VERIFY offered rows" in log, "streaming row delivery")
            else:
                action("run-metadata" if name == "metadata-failure" else "run-credit")
                if RACES[name] == "credit":
                    log = barrier(lambda log: log.count("VERIFY offered rows") == 4, "four held batches")
                    if "VERIFY offered terminal" in log:
                        raise RuntimeError("Query completed before held-credit barrier")
                    if name == "credit-stop":
                        action("stop")
                        barrier(lambda log: "VERIFY cancel-ok" in log, "cancellation accepted before drain resumes")
                        action("resume")
                        action("settled")
                        action("recovery")
                else:
                    barrier(lambda log: "VERIFY queue-rejected Some(Full)" in log, "saturated queue")
                    if name == "saturated-replace":
                        action("replace")
                        barrier(lambda log: "VERIFY view-replaced" in log, "actual root view replacement")
                        action("ready")
                        action("recovery")
                    elif name == "metadata-failure":
                        action("resume")
                        action("disconnected")
        action("quit" if name == "credit-quit" else "close")
    log = (evidence / "native.log").read_text()
    if name in {"connect-close", "reconnect-close"}:
        after = log.split("VERIFY closing")[-1]
        if "VERIFY offered session" in after:
            raise RuntimeError("Late session after delayed-open closure")
    print(f"PASS: {name}", flush=True)


def performance(evidence):
    with window(evidence) as (child, profile):
        step(child, profile, evidence, "ready")

        def measure(*args):
            subprocess.run([str(MEASURE), *args, "--pid", str(child.pid), "--foreground"], check=True, timeout=60, stdout=subprocess.DEVNULL)

        for fixture_name in ["many", "wide", "large"]:
            with (evidence / f"prepare-{fixture_name}.txt").open("w") as output:
                subprocess.run([str(PROBE), "--native-fixture", str(child.pid), str(profile), f"--prepare-metrics={fixture_name}"], stdout=output, stderr=subprocess.STDOUT, check=True, timeout=30)
            measure("footprint", "--seconds", "5", "--out", str(evidence / f"footprint-{fixture_name}.json"))
            if fixture_name == "many":
                step(child, profile, evidence, "typing-document")
                for run in range(1, 4):
                    measure("latency", "--count", "300", "--interval-ms", "120", "--out", str(evidence / f"latency-{run}.json"))
                measure("footprint", "--seconds", "30", "--out", str(evidence / "idle.json"))
            for direction in (["down", "right"] if fixture_name == "wide" else ["down"]):
                extra = ["--horizontal"] if direction == "right" else []
                delta = "-10" if direction == "right" else ("-5" if fixture_name == "large" else "-20")
                for run in range(1, 4):
                    target = evidence / f"scroll-{fixture_name}-{direction}-{run}.json"
                    measure("scroll", "--at", "0.6,0.8", "--seconds", "5", "--delta", delta, "--hz", "240", *extra, "--out", str(target))
                    result = json.loads(target.read_text())
                    if result["frames"] < 100:
                        raise RuntimeError(f"Insufficient changed frames to establish scrolling: {target}")
                    measure("scroll", "--at", "0.6,0.8", "--seconds", "1", "--delta", "2000", *extra, "--out", os.devnull)
        step(child, profile, evidence, "close")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("mode", choices=["races", "performance"])
    parser.add_argument("--out", type=Path, required=True)
    parser.add_argument("--repeat", type=int, default=3)
    parser.add_argument("--scenario", choices=list(RACES))
    args = parser.parse_args()
    if args.repeat < 1:
        parser.error("--repeat must be positive")
    if args.mode == "performance" and args.scenario:
        parser.error("--scenario applies only to races")
    fixture.check()
    build = ["cargo", "+1.98.1", "build", "--release", "--locked"]
    if args.mode == "races":
        build += ["--features", "fixture-verification"]
    subprocess.run(build, cwd=ROOT / "apps/native", check=True)
    subprocess.run(["swiftc", str(ROOT / "tools/measure/editor-accessibility.swift"), "-o", str(PROBE)], check=True)
    if args.mode == "performance":
        performance(args.out.resolve())
    else:
        for number in range(1, args.repeat + 1):
            for scenario in ([args.scenario] if args.scenario else RACES):
                race(args.out.resolve() / f"{number}-{scenario}", scenario)


if __name__ == "__main__":
    main()
