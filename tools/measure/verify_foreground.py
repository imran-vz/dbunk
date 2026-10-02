#!/usr/bin/env python3
"""Prove interrupted typing captures are rejected, using two owned calibration windows."""
import argparse
from pathlib import Path
import subprocess
import time


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--out", required=True, type=Path)
    args = parser.parse_args()
    args.out.mkdir(parents=True, exist_ok=False)
    measure = Path(__file__).resolve().parent / ".build/release/measure"
    for guarded in (False, True):
        label = "guarded" if guarded else "unguarded"
        children = []
        try:
            target = subprocess.Popen([str(measure), "target"], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
            children.append(target)
            time.sleep(1)
            result = args.out / f"{label}.json"
            with (args.out / f"{label}.txt").open("w") as log:
                command = [str(measure), "latency", "--pid", str(target.pid), "--count", "40", "--warmup", "0", "--out", str(result)]
                if guarded:
                    command.append("--foreground")
                capture = subprocess.Popen(command, stdout=log, stderr=subprocess.STDOUT)
                children.append(capture)
                time.sleep(2)
                cover = subprocess.Popen([str(measure), "target"], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
                children.append(cover)
                code = capture.wait(timeout=15)
            output = (args.out / f"{label}.txt").read_text()
            if guarded:
                assert code == 2 and "target lost foreground or is obscured; run discarded" in output and not result.exists(), output
            else:
                assert code == 0 and result.exists(), output
            print(f"PASS: {label} exit={code}; samples_written={result.exists()}", flush=True)
        finally:
            for child in reversed(children):
                if child.poll() is None:
                    child.terminate()
                    try:
                        child.wait(timeout=3)
                    except subprocess.TimeoutExpired:
                        child.kill()
                        child.wait()


if __name__ == "__main__":
    main()
