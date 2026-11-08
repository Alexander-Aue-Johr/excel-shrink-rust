"""Check resource-counter calibration and durable unsuccessful-trial receipts."""

import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

from process_metrics import run_measured


class ProcessMetricsTests(unittest.TestCase):
    def test_cpu_and_held_memory_are_observed(self):
        code = """
import time
memory = bytearray(64 * 1024 * 1024)
until = time.perf_counter() + 0.2
while time.perf_counter() < until:
    sum(range(10000))
time.sleep(0.1)
"""
        with tempfile.TemporaryDirectory(prefix="process-metrics-") as temporary:
            result = run_measured([sys.executable, "-c", code], Path(temporary), 10)
            self.assertEqual(result["exit_code"], 0)
            self.assertFalse(result["timed_out"])
            if os.name == "nt":
                metrics = result["resources"]
                self.assertEqual(metrics["status"], "measured")
                self.assertGreater(metrics["cpu_user_seconds"] + metrics["cpu_kernel_seconds"], 0.02)
                self.assertGreaterEqual(metrics["sampled_peak_private_bytes"], 64 * 1024 * 1024)
                self.assertGreater(metrics["memory_samples"], 0)

    def test_failure_keeps_its_receipt(self):
        with tempfile.TemporaryDirectory(prefix="process-metrics-") as temporary:
            root = Path(temporary)
            with self.assertRaises(subprocess.CalledProcessError):
                run_measured([sys.executable, "-c", "raise SystemExit(7)"], root, 10)
            self.assertEqual(json.loads((root / "process-metrics.json").read_text())["exit_code"], 7)

    def test_timeout_keeps_its_receipt_and_terminates_the_child(self):
        with tempfile.TemporaryDirectory(prefix="process-metrics-") as temporary:
            root = Path(temporary)
            with self.assertRaises(subprocess.TimeoutExpired):
                run_measured([sys.executable, "-c", "import time; time.sleep(20)"], root, 0.2)
            result = json.loads((root / "process-metrics.json").read_text())
            self.assertTrue(result["timed_out"])
            self.assertIsNotNone(result["exit_code"])


if __name__ == "__main__":
    unittest.main()
