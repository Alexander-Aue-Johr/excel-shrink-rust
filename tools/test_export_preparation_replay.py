"""Verify portable replay packaging without Excel or a running Observatory."""

import gzip
import importlib.util
import json
from pathlib import Path
import tempfile
import unittest

spec = importlib.util.spec_from_file_location("export_replay", Path(__file__).with_name("export-preparation-replay.py"))
exporter = importlib.util.module_from_spec(spec)
spec.loader.exec_module(exporter)


class ExportTests(unittest.TestCase):
    def test_relative_assets_exact_events_and_new_destination(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            run = root / "source"
            run.mkdir()
            rows = [{"sequence": index, "elapsed_micros": index * 1000,
                     "elapsed_millis": index, "operation": "sheet-shrink", "event_type": kind}
                    for index, kind in enumerate(("run.started", "job.started", "job.completed", "run.completed"))]
            events = gzip.compress(("\n".join(json.dumps(row) for row in rows) + "\n").encode())
            (run / "events.jsonl.gz").write_bytes(events)
            metadata = {"replay": "events.jsonl.gz", "corpus": "fixture", "config": {"inputs": ["C:/private/source"], "workers": 1},
                        "summary": {"total": 1, "processed": 1, "preserved": 0, "failed": 0, "skipped": 0},
                        "ui_terminal_status": "completed", "code_revision": "fixture", "binary_sha256": "fixture"}
            (run / "distribution-run.json").write_text(json.dumps(metadata))
            target = root / "export"
            exporter.export(run, target)
            self.assertEqual((target / "events.jsonl.gz").read_bytes(), events)
            self.assertEqual((run / "events.jsonl.gz").read_bytes(), events)
            html = (target / "index.html").read_text()
            self.assertIn('src="./app.js"', html)
            self.assertIn('name="observatory-replay"', html)
            manifest = json.loads((target / "recording.json").read_text())
            self.assertEqual(manifest["source"]["defaults"]["inputs"], ["fixture"])
            self.assertEqual(manifest["event_count"], 4)
            with self.assertRaises(FileExistsError):
                exporter.export(run, target)
            rows[-1]["event_type"] = "work.started"
            (run / "events.jsonl.gz").write_bytes(gzip.compress(("\n".join(json.dumps(row) for row in rows)).encode()))
            with self.assertRaisesRegex(ValueError, "terminal"):
                exporter.export(run, root / "invalid")
            self.assertFalse((root / "invalid").exists())


if __name__ == "__main__":
    unittest.main()
