"""Prepare selected Excel controls, then summarize their ZIP directory sizes."""
import argparse
from pathlib import Path
import subprocess
import sys


if __name__ == "__main__":
    parser = argparse.ArgumentParser()
    parser.add_argument("--root", type=Path, required=True)
    args = parser.parse_args()
    preparation = subprocess.run([
        sys.executable, str(args.root / "code/run.py"), "--root", str(args.root)])
    subprocess.run([
        sys.executable, str(Path(__file__).with_name("analyze-prepared-sizes.py")),
        "--comparison-run", str(args.root), "--output", str(args.root / "size-comparison")], check=True)
    raise SystemExit(preparation.returncode)
