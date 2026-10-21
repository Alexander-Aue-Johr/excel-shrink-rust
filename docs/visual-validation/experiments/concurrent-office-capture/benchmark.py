"""Compare fixed worksheet captures with one and two independent Excel instances."""
import argparse
from concurrent.futures import ThreadPoolExecutor
import json
from pathlib import Path
import statistics
import subprocess
import sys
import time


def windows_path(path):
    path = str(Path(path).resolve())
    if sys.platform == "win32":
        return path
    return subprocess.check_output(["wslpath", "-w", path], text=True).strip()


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--launcher", required=True, type=Path)
    parser.add_argument("--workbook", required=True, type=Path)
    parser.add_argument("--config", required=True, type=Path)
    parser.add_argument("--output", required=True, type=Path)
    parser.add_argument("--repeats", type=int, default=2)
    args = parser.parse_args()
    args.output.mkdir(parents=True, exist_ok=True)
    command = ["powershell.exe", "-STA", "-NoLogo", "-NoProfile", "-ExecutionPolicy", "Bypass",
               "-File", windows_path(args.launcher), "-WorkbookPath", windows_path(args.workbook),
               "-ConfigPath", windows_path(args.config), "-StartupTimeoutSeconds", "120",
               "-CaptureTimeoutSeconds", "180"]
    results = []

    def capture(directory):
        with (args.output / f"{directory}.log").open("wb") as log:
            started = time.monotonic()
            subprocess.run(command + ["-OutputDir", windows_path(args.output / directory)],
                           check=True, stdout=log, stderr=subprocess.STDOUT, timeout=330)
            result = json.loads((args.output / directory / "result.json").read_text())
            if result.get("error") or not result.get("captures"):
                raise RuntimeError(f"Incomplete capture in {directory}")
            return time.monotonic() - started

    # Alternate order to reduce a consistent warm-cache advantage.
    for repeat in range(args.repeats):
        for workers in ((1, 2) if repeat % 2 == 0 else (2, 1)):
            started = time.monotonic()
            with ThreadPoolExecutor(max_workers=workers) as executor:
                seconds = list(executor.map(capture,
                    [f"r{repeat}-j{workers}-{i}" for i in range(2)]))
            result = {"repeat": repeat + 1, "workers": workers,
                      "wall_seconds": time.monotonic() - started,
                      "capture_seconds": seconds}
            results.append(result)
            print(json.dumps(result), flush=True)
            (args.output / "results.json").write_text(json.dumps(results, indent=2) + "\n")
    serial = statistics.median(r["wall_seconds"] for r in results if r["workers"] == 1)
    parallel = statistics.median(r["wall_seconds"] for r in results if r["workers"] == 2)
    print(json.dumps({"serial_seconds": serial, "parallel_seconds": parallel,
                      "throughput_ratio": serial / parallel}), flush=True)


if __name__ == "__main__":
    main()
