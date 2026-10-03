#!/usr/bin/env python3
"""Owned release-workspace diagnostics comparable to Plan 026.

No numeric performance pass threshold is invented here. The runner enforces
capture validity, the fixed workload and cleanup, then records measurements and
baseline deltas for review. Foreground interruption invalidates the run.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import platform
import signal
import statistics
import subprocess
import sys
import time

import fixture
import package
import workspace_launch

ROOT = fixture.ROOT
PROBE = ROOT / "tools/native/.state/workspace-performance"
MEASURE = ROOT / "tools/measure/.build/release/measure"
BASELINE = ROOT / "plans/evidence/026/final-verification/performance-summary.json"


def write_json(path, value):
    workspace_launch.private_json(path, value)


def build_tools():
    PROBE.parent.mkdir(parents=True, exist_ok=True)
    subprocess.run(["swiftc", str(ROOT / "tools/measure/workspace-performance.swift"), "-o", str(PROBE)], check=True)
    subprocess.run(["swift", "build", "-c", "release", "--package-path", str(ROOT / "tools/measure")], check=True)


def wait_launch(launcher, evidence):
    deadline = time.monotonic() + 30
    while time.monotonic() < deadline:
        if launcher.poll() is not None:
            raise RuntimeError("Workspace launcher exited before ownership evidence; inspect launcher.txt")
        identity = evidence / "identity.json"
        if identity.is_file():
            value = json.loads(identity.read_text())
            if (Path(value["profile"]) / ".dbunk-native-stage04").is_file():
                return value
        time.sleep(0.1)
    raise RuntimeError("Workspace did not publish its owned profile identity")


def guard(identity):
    executable, profile = Path(identity["executable"]), Path(identity["profile"])
    if executable != executable.resolve() or profile != profile.resolve():
        raise RuntimeError("Owned executable/profile path changed")
    if package.digest(executable) != identity["executable_sha256"]:
        raise RuntimeError("Release executable changed during capture")
    command = subprocess.check_output(["/bin/ps", "-p", str(identity["pid"]), "-o", "command="], text=True).strip()
    expected = f"{executable} --workspace-profile {profile} --fixture-manifest {identity['cwd']}/fixture.json"
    if command != expected:
        raise RuntimeError("PID no longer has the exact owned workspace arguments")
    marker = json.loads((profile / ".dbunk-native-stage04").read_text())
    manifest = json.loads((Path(identity["cwd"]) / "fixture.json").read_text())
    if marker.get("path") != str(profile) or marker.get("fixtures") != manifest or manifest.get("instance") != identity["fixture_instance"]:
        raise RuntimeError("Profile/manifest/fixture identity changed")


def summarize(out):
    baseline = json.loads(BASELINE.read_text())
    result = {"scope": "Release actual-window AX-active diagnostics; no human VoiceOver or IME claim", "baseline": str(BASELINE.relative_to(ROOT)), "sessions": {}}
    for sessions in (1, 4):
        directory = out / f"sessions-{sessions}"
        latency = [json.loads(path.read_text()) for path in sorted(directory.glob("latency-*.json"))]
        scroll = [json.loads(path.read_text()) for path in sorted(directory.glob("scroll-*.json"))]
        idle = json.loads((directory / "idle.json").read_text())
        footprints = {name: json.loads((directory / f"footprint-{name}.json").read_text())["footprintMiB"] for name in ("many", "wide", "large")}
        environment = idle["environment"]
        fields = ("displayMaxHz", "displayPoints", "displayScale", "lowPowerMode", "model", "os", "power", "thermalState")
        mismatch = {key: {"baseline": baseline["environment"].get(key), "current": environment.get(key)} for key in fields if baseline["environment"].get(key) != environment.get(key)}
        median_p95 = statistics.median(value["summary"]["p95"] for value in latency)
        result["sessions"][str(sessions)] = {
            "typing_samples": sum(value["summary"]["count"] for value in latency),
            "typing_p50_ms_median": statistics.median(value["summary"]["p50"] for value in latency),
            "typing_p95_ms_median": median_p95,
            "typing_p95_ms_delta_vs_plan026": median_p95 - baseline["typing_p95_ms_median"],
            "missed": sum(value["missed"] for value in latency),
            "idle_cpu_percent_one_core": idle["cpuPercentOfOneCore"],
            "idle_footprint_mib": idle["footprintMiB"], "fixture_footprints_mib": footprints,
            "scroll_runs": len(scroll), "scroll_p95_ms_range": [min(value["summary"]["p95"] for value in scroll), max(value["summary"]["p95"] for value in scroll)],
            "long_frames": sum(value["longFrames"] for value in scroll),
            "environment": environment, "baseline_environment_differences": mismatch,
            "comparison_note": "Same workload, capture tool, 1440x932 window, input count and cadence. Four sessions retain three additional many-fixture results. Review environmental differences and raw samples before interpreting deltas.",
        }
    cycles = json.loads((out / "cycles-memory.json").read_text())
    before = [sample["physicalFootprintBytes"] for sample in cycles["samples"] if sample["phase"] == "before"]
    after = [sample["physicalFootprintBytes"] for sample in cycles["samples"] if sample["phase"] == "after"]
    result["memory_cycles"] = {"cycles": cycles["cycles"], "sampled_peak_bytes": max(sample["physicalFootprintBytes"] for sample in cycles["samples"]),
        "process_lifetime_peak_bytes": max(sample["lifetimePeakPhysicalFootprintBytes"] for sample in cycles["samples"]),
        "before_median_bytes": statistics.median(before), "last_5_seconds_median_bytes": statistics.median(after[-50:]),
        "settled_delta_bytes": statistics.median(after[-50:]) - statistics.median(before),
        "sampling": cycles["sampling"], "note": "Physical footprint includes allocator/caches and is distinct from encoded retained-result and queue limits; investigate persistent growth rather than infer a leak from one delta."}
    log = (out / "launch/native.log").read_text()
    queues = []
    models = []
    for line in log.splitlines():
        if "remaining_bytes=" in line:
            if not line.endswith("remaining_bytes=0"):
                raise RuntimeError(f"Nonzero queue after quit: {line}")
            queues.append({"scope": "workspace" if "workspace queue" in line else "document", "peak_bytes": int(line.split("high_water_bytes=")[1].split()[0])})
        if "Native retained bytes: " in line:
            models.append(int(line.split("Native retained bytes: ")[1]))
    if not queues or not models or "Native cleanup failed" in log:
        raise RuntimeError("Missing queue/model peak or successful cleanup evidence")
    result["queue_high_water"] = queues
    result["largest_reported_document_encoded_result_bytes"] = max(models)
    result["encoded_model_scope"] = "Per-document terminal observations. Aggregate result-model peak is not directly instrumented by this run."
    result["teardown"] = json.loads((out / "launch/teardown.json").read_text())
    result["acceptance"] = "Diagnostic capture and cleanup completed. Numerical regressions and settle trace require review; no universal performance threshold asserted."
    write_json(out / "summary.json", result)


def run(path, out, no_build):
    if platform.system() != "Darwin" or platform.machine() != "arm64":
        raise RuntimeError("Apple Silicon macOS is required")
    if path.exists() or not path.is_absolute() or path != path.resolve():
        raise RuntimeError("Use a NEW canonical absolute stage04 profile")
    if out != out.resolve() or out.exists() or out.is_relative_to(path):
        raise RuntimeError("Use a NEW canonical evidence directory outside the profile")
    build_tools()
    if not no_build:
        subprocess.run(["cargo", "+1.98.1", "build", "--release", "--locked"], cwd=ROOT / "apps/native", check=True)
    owned, _ = fixture.check()
    baseline = fixture.backend_count()
    if baseline:
        raise RuntimeError("Owned fixture has other sessions; wait for the other verification to finish")
    workspace_launch.validate_path(path, owned)
    out.mkdir(mode=0o700, parents=True)
    paths = ["tools/native/workspace_performance.py", "tools/measure/workspace-performance.swift", "tools/native/workspace_launch.py", "tools/measure/Sources/measure/main.swift", "tools/measure/fixtures/editor-2000.sql", "apps/native/Cargo.lock"]
    write_json(out / "environment.json", {"command": sys.argv, "build_variant": "release, existing native binary" if no_build else "release, default features; no fixture-verification hooks", "os": platform.platform(), "commit": subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=ROOT, text=True).strip(), "dirty_diff_sha256": hashlib.sha256(subprocess.check_output(["git", "diff", "HEAD"], cwd=ROOT)).hexdigest(), "source_sha256": {name: package.digest(ROOT / name) for name in paths}, "probe_sha256": package.digest(PROBE), "measure_sha256": package.digest(MEASURE), "fixture": owned, "baseline": baseline})
    print(f"Performance target: owned {fixture.ENDPOINT} instance={owned['instance']}; NEW profile {path}", flush=True)
    print("Foreground diagnostics take about 8 minutes. Any foreground interruption invalidates the run.", flush=True)
    launcher = None
    identity = None
    complete = False
    try:
        with (out / "launcher.txt").open("w") as launch_log:
            launcher = subprocess.Popen([sys.executable, str(ROOT / "tools/native/workspace_launch.py"), str(path), "--no-build", "--out", str(out / "launch")], stdout=launch_log, stderr=subprocess.STDOUT)
            identity = wait_launch(launcher, out / "launch")

            def step(action, *extra):
                guard(identity)
                with (out / "accessibility.txt").open("a") as log:
                    subprocess.run([str(PROBE), str(out / "launch/identity.json"), action, *map(str, extra)], stdout=log, stderr=subprocess.STDOUT, check=True, timeout=240 if action == "cycles" else 45)

            def measure(*args):
                guard(identity)
                subprocess.run([str(MEASURE), *map(str, args), "--pid", str(identity["pid"]), "--foreground"], check=True, timeout=60, stdout=subprocess.DEVNULL)

            step("setup-one")
            measure("place", "--size", "1440x932")
            for sessions in (1, 4):
                if sessions == 4:
                    step("setup-four")
                directory = out / f"sessions-{sessions}"
                directory.mkdir()
                actual_count = fixture.backend_count()
                if actual_count != baseline + sessions + 1:
                    raise RuntimeError(f"Expected {sessions} document sessions plus one shared observer, found {actual_count - baseline} backends")
                write_json(directory / "sessions.json", {"document_sessions": sessions, "shared_observer_connections": 1, "observed_backend_count": actual_count, "baseline": baseline})
                for name in ("many", "wide", "large"):
                    step(name)
                    measure("footprint", "--seconds", "5", "--out", directory / f"footprint-{name}.json")
                    if name == "many":
                        step("typing", ROOT / "tools/measure/fixtures/editor-2000.sql")
                        for number in range(1, 4):
                            measure("latency", "--count", "300", "--interval-ms", "120", "--out", directory / f"latency-{number}.json")
                        measure("footprint", "--seconds", "30", "--out", directory / "idle.json")
                    for direction in (["down", "right"] if name == "wide" else ["down"]):
                        extra = ["--horizontal"] if direction == "right" else []
                        delta = "-10" if direction == "right" else ("-5" if name == "large" else "-20")
                        for number in range(1, 4):
                            target = directory / f"scroll-{name}-{direction}-{number}.json"
                            measure("scroll", "--at", "0.6,0.8", "--seconds", "5", "--delta", delta, "--hz", "240", *extra, "--out", target)
                            if json.loads(target.read_text())["frames"] < 100:
                                raise RuntimeError(f"Insufficient changed frames to establish scrolling: {target}")
                            measure("scroll", "--at", "0.6,0.8", "--seconds", "1", "--delta", "2000", *extra, "--out", os.devnull)
            step("cycles", out / "cycles-memory.json")
            if fixture.backend_count() != baseline + 2:
                raise RuntimeError("Cycle close did not return to one retained document session")
            step("quit")
            launcher.wait(timeout=15)
            if launcher.returncode:
                raise RuntimeError("Workspace launcher reported failed cleanup")
            fixture.wait_baseline(baseline)
            summarize(out)
            complete = True
    finally:
        if not complete:
            write_json(out / "failed.json", {"scope": "Capture invalid/incomplete; no performance acceptance", "profile_retained": str(path), "identity": identity})
        if launcher and launcher.poll() is None:
            if identity:
                guard(identity)
                # Only the exact child owned by the recorded launcher may be
                # stopped. Forced teardown never counts as successful quit.
                os.kill(identity["pid"], signal.SIGTERM)
            launcher.wait(timeout=10)
    print(f"Workspace diagnostics captured: {out / 'summary.json'}", flush=True)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("path", type=Path, nargs="?")
    parser.add_argument("--out", type=Path)
    parser.add_argument("--no-build", action="store_true")
    parser.add_argument("--prepare-only", action="store_true", help="compile measurement tools without launching an app or accessing a fixture")
    args = parser.parse_args()
    if args.prepare_only:
        build_tools()
    elif args.path is None or args.out is None:
        parser.error("path and --out are required unless --prepare-only")
    else:
        run(args.path, args.out, args.no_build)


if __name__ == "__main__":
    try:
        main()
    except (RuntimeError, OSError, ValueError, subprocess.SubprocessError) as error:
        sys.exit(f"Workspace performance failed: {error}")
