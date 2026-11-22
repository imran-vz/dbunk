#!/usr/bin/env python3
"""Capture the native dbunk window, screen by screen, for the UI council.

Launches the release build against the owned PostgreSQL fixture with a fresh
stage04 profile (tools/native/workspace_launch.py --no-build), places the
window at a fixed size, plays the steps of each scenario in scenarios.json and
writes one PNG per "shot" step. Each scenario gets a fresh app process unless
it sets "continue": true, so state never leaks between scenarios.

    python3 tools/ui-council/capture.py --out DIR [--only NAME ...] [--keep-open]
    python3 tools/ui-council/capture.py --launch          # start one app, print its PID, leave it running

Needs: `just native-fixture-up`, a release build (`cargo +1.98.1 build
--release --locked` in apps/native), Screen Recording and Accessibility
permission for the calling terminal. Leave the mouse alone during a run: clicks
are real pointer events.

Step forms (scenarios.json):
  {"key": "cmd+k"}  {"type": "select 1"}  {"wait": 0.5}
  {"press": "Name", "role": "AXButton", "nth": 0}   AXPress, falls back to a click
  {"click": "Name", "right": true, "double": true}  pointer click at the element centre
  {"clickat": [x, y]}  {"hover": [x, y]}  {"scroll": [x, y, dy]}   window-relative points
  {"size": [w, h]}     resize the window (points)
  {"shot": "file-stem"}
  {"optional": true, ...} on any step: a failure is logged, not fatal
"""
import argparse
import json
import os
from pathlib import Path
import re
import signal
import subprocess
import sys
import tempfile
import time

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[1]
DRIVE = HERE / ".build/drive"
SCENARIOS = HERE / "scenarios.json"
WINDOW = (1440, 900)


def ensure_driver():
    source = HERE / "drive.swift"
    if not DRIVE.exists() or DRIVE.stat().st_mtime < source.stat().st_mtime:
        DRIVE.parent.mkdir(exist_ok=True)
        subprocess.run(["swiftc", "-O", str(source), "-o", str(DRIVE)], check=True, stderr=subprocess.DEVNULL)


def drive(pid, command, *args, check=True):
    result = subprocess.run([str(DRIVE), command, "--pid", str(pid), *map(str, args)], capture_output=True, text=True, timeout=60)
    if check and result.returncode:
        raise RuntimeError(f"drive {command} {' '.join(map(str, args))}: {result.stderr.strip()}")
    return result.stdout


def launch(release=False):
    """Start the app on the owned fixture with a fresh stage04 profile.

    Default: `cargo run` (incremental dev build, then launch). `release=True`
    uses tools/native/workspace_launch.py --no-build with the release binary.
    """
    log = open(HERE / ".build/launch.log", "a")
    if release:
        child = subprocess.Popen(
            [sys.executable, str(ROOT / "tools/native/workspace_launch.py"), "--no-build"],
            cwd=ROOT, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True, start_new_session=True,
        )
        pid = None
        deadline = time.monotonic() + 60
        while time.monotonic() < deadline:
            line = child.stdout.readline()
            if not line:
                if child.poll() is not None:
                    break
                continue
            log.write(line)
            log.flush()
            match = re.search(r"Native workspace PID: (\d+)", line)
            if match:
                pid = int(match.group(1))
                break
        if pid is None:
            raise RuntimeError(f"Native app did not start; see {HERE / '.build/launch.log'}")
        subprocess.Popen(["cat"], stdin=child.stdout, stdout=log)
    else:
        sys.path.insert(0, str(ROOT / "tools/native"))
        import fixture
        import profile as stage04_profile
        import workspace_launch
        owned, _ = fixture.check()
        path = workspace_launch.fresh_profile_path()
        work = Path(tempfile.mkdtemp(prefix="dbunk-council-launch-")).resolve()
        manifest = work / "fixture.json"
        workspace_launch.private_json(manifest, stage04_profile.manifest(owned, None))
        child = subprocess.Popen(
            ["cargo", "+1.98.1", "run", "--locked", "--quiet", "--",
             "--workspace-profile", str(path), "--fixture-manifest", str(manifest)],
            cwd=ROOT / "apps/native", stdout=log, stderr=subprocess.STDOUT, start_new_session=True,
        )
        pid = None
        deadline = time.monotonic() + 1800  # the first run may compile
        while time.monotonic() < deadline and pid is None:
            if child.poll() is not None:
                raise RuntimeError(f"cargo run exited {child.returncode}; see {HERE / '.build/launch.log'}")
            # cargo execs the binary in place on Unix, so the app takes over
            # cargo's pid once the build is done.
            command = subprocess.run(["ps", "-p", str(child.pid), "-o", "command="],
                                     capture_output=True, text=True).stdout
            pid = child.pid if "target/debug/dbunk-native" in command else None
            if pid is None:
                time.sleep(0.5)
        if pid is None:
            raise RuntimeError("cargo run did not start the app")
    # Wait for the window, then for the accessibility tree to be populated.
    for _ in range(240):
        if subprocess.run([str(DRIVE), "shot", "--pid", str(pid), "--out", "/dev/null"], capture_output=True).returncode == 0:
            break
        time.sleep(0.25)
    for attempt in range(120):
        if attempt % 6 == 0:
            # AccessKit builds the tree only once the window has been active.
            subprocess.run([str(DRIVE), "raise", "--pid", str(pid)], capture_output=True)
        tree = subprocess.run([str(DRIVE), "ax", "--pid", str(pid), "--depth", "8"], capture_output=True, text=True).stdout
        if tree.count("AXButton") > 4:
            break
        time.sleep(0.5)
    return child, pid


def stop(child, pid):
    try:
        os.kill(pid, signal.SIGTERM)
    except ProcessLookupError:
        pass
    try:
        child.wait(timeout=15)
    except subprocess.TimeoutExpired:
        os.killpg(child.pid, signal.SIGTERM)
        child.wait(timeout=5)


def place(pid, size):
    drive(pid, "place", "--x", 40, "--y", 40, "--w", size[0], "--h", size[1])


def expand(steps, macros):
    """Inline {"macro": name, "args": [...]} steps; "$1".. in strings take the args."""
    result = []
    for step in steps:
        if "macro" not in step:
            result.append(step)
            continue
        body = json.dumps(macros[step["macro"]])
        for index, value in enumerate(step.get("args", []), start=1):
            body = body.replace(f"${index}", value)
        result.extend(expand(json.loads(body), macros))
    return result


def load_demo():
    """Load demo.sql into the owned fixture only (identity-checked by fixture.check)."""
    sys.path.insert(0, str(ROOT / "tools/native"))
    import fixture
    _, container = fixture.check()
    fixture.sql(container, (HERE / "demo.sql").read_text())


def play(pid, steps, out, log):
    for step in steps:
        try:
            if "key" in step:
                drive(pid, "key", "--chord", step["key"])
            elif "type" in step:
                drive(pid, "type", "--text", step["type"])
            elif "wait" in step:
                time.sleep(step["wait"])
            elif "press" in step or "click" in step:
                verb = "press" if "press" in step else "click"
                args = ["--name", step[verb]]
                if step.get("role"):
                    args += ["--role", step["role"]]
                if step.get("nth"):
                    args += ["--nth", step["nth"]]
                if step.get("right"):
                    args += ["--right"]
                if step.get("double"):
                    args += ["--double"]
                drive(pid, verb, *args)
            elif "clickat" in step:
                x, y = step["clickat"]
                drive(pid, "clickat", "--x", x, "--y", y, *(["--right"] if step.get("right") else []), *(["--double"] if step.get("double") else []))
            elif "hover" in step:
                x, y = step["hover"]
                drive(pid, "hover", "--x", x, "--y", y)
            elif "scroll" in step:
                x, y, dy = step["scroll"]
                drive(pid, "scroll", "--x", x, "--y", y, "--dy", dy)
            elif "size" in step:
                place(pid, step["size"])
            elif "shot" in step:
                time.sleep(step.get("settle", 0.6))
                path = out / f"{step['shot']}.png"
                drive(pid, "shot", "--out", path)
                log.append(str(path))
                print(f"  shot {path.name}", flush=True)
            time.sleep(step.get("after", 0.25))
        except RuntimeError as error:
            if step.get("optional"):
                print(f"  (optional step skipped) {error}", flush=True)
                continue
            raise


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--out", type=Path)
    parser.add_argument("--only", nargs="*", help="scenario names to run")
    parser.add_argument("--scenarios", type=Path, default=SCENARIOS)
    parser.add_argument("--launch", action="store_true", help="start one app, place it, print its PID and leave it running")
    parser.add_argument("--release", action="store_true", help="use the release binary via workspace_launch.py --no-build instead of cargo run")
    parser.add_argument("--keep-open", action="store_true", help="leave the app running after the last scenario (prints its PID)")
    args = parser.parse_args()
    ensure_driver()
    if args.launch:
        child, pid = launch(args.release)
        time.sleep(2)
        place(pid, WINDOW)
        print(json.dumps({"pid": pid, "launcher_pid": child.pid, "drive": str(DRIVE)}))
        return
    if not args.out:
        parser.error("--out is required")
    args.out.mkdir(parents=True, exist_ok=True)
    spec = json.loads(args.scenarios.read_text())
    scenarios = spec["scenarios"]
    for scenario in scenarios:
        scenario["steps"] = expand(scenario["steps"], spec.get("macros", {}))
    load_demo()
    if args.only:
        scenarios = [s for s in scenarios if s["name"] in args.only]
    shots, failures = [], []
    child = pid = None
    for scenario in scenarios:
        print(f"scenario {scenario['name']}", flush=True)
        try:
            if not scenario.get("continue") or pid is None:
                if pid is not None:
                    stop(child, pid)
                child, pid = launch(args.release)
                time.sleep(scenario.get("boot", 2.5))
                place(pid, scenario.get("window", WINDOW))
                time.sleep(0.5)
            play(pid, scenario["steps"], args.out, shots)
        except Exception as error:  # keep going; report at the end
            failures.append(f"{scenario['name']}: {error}")
            print(f"  FAILED {error}", flush=True)
            if pid is not None:
                try:
                    drive(pid, "shot", "--out", args.out / f"_failed-{scenario['name']}.png", check=False)
                except Exception:
                    pass
    if pid is not None and not args.keep_open:
        stop(child, pid)
    elif pid is not None:
        print(f"app left running: pid {pid}")
    (args.out / "_capture.json").write_text(json.dumps({"shots": shots, "failures": failures}, indent=2) + "\n")
    print(f"{len(shots)} shots, {len(failures)} failures -> {args.out}")
    sys.exit(1 if failures else 0)


if __name__ == "__main__":
    main()
