#!/usr/bin/env python3
"""Check the resolved native graph, including every GPUI source revision."""
import json
from pathlib import Path
import subprocess
import sys

ROOT = Path(__file__).resolve().parents[2]
REVISION = "506beb34de3f433707b7ebe8d8ad2d80f856af6c"
result = subprocess.run(
    ["cargo", "+1.98.1", "metadata", "--locked", "--format-version", "1"],
    cwd=ROOT / "apps/native", capture_output=True, text=True, check=True,
)
metadata = json.loads(result.stdout)
resolved = {node["id"] for node in metadata["resolve"]["nodes"]}
packages = [package for package in metadata["packages"] if package["id"] in resolved]
for package in packages:
    name = package["name"]
    if name in {"tauri", "tauri-runtime", "tauri-runtime-wry", "wry", "tao"} or "webkit" in name:
        sys.exit(f"Forbidden native host dependency: {name}")
gpui = [package for package in packages if package["name"] == "gpui"]
if len(gpui) != 1 or not (gpui[0].get("source") or "").endswith("#" + REVISION):
    sys.exit(f"Expected one GPUI revision {REVISION}")
print(f"PASS: one GPUI revision {REVISION}; no Tauri/Wry/Tao/WebKit packages")
