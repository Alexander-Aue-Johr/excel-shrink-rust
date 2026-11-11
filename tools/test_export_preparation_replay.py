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
    def test_path_projection_preserves_sheet_names_and_measurements(self):
        event = {"output": r"I:\private\outputs\book.xlsx", "nested": [r"\\host\share\other.xlsx"],
                 "sheet": "A:B", "part": "xl/worksheets/sheet1.xml", "bytes": 123,
                 "error": "Cannot open C:/private/book.xlsx"}
        projected = exporter.redact_local_paths(event)
        self.assertEqual(projected["output"], "book.xlsx")
        self.assertEqual(projected["nested"], ["other.xlsx"])
        self.assertEqual(projected["sheet"], "A:B")
        self.assertEqual(projected["part"], event["part"])
        self.assertEqual(projected["bytes"], 123)
        self.assertEqual(projected["error"], event["error"])
        self.assertEqual(event["output"], r"I:\private\outputs\book.xlsx")

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
            rows[2]["payload"] = {"output": r"I:\private\outputs\book.xlsx"}
            private_events = gzip.compress(("\n".join(json.dumps(row) for row in rows) + "\n").encode())
            (run / "events.jsonl.gz").write_bytes(private_events)
            public_target = root / "public"
            exporter.export(run, public_target, redact_paths=True)
            public_rows = [json.loads(line) for line in gzip.decompress((public_target / "events.jsonl.gz").read_bytes()).splitlines()]
            self.assertEqual(public_rows[2]["payload"]["output"], "book.xlsx")
            self.assertEqual((run / "events.jsonl.gz").read_bytes(), private_events)
            public_manifest = json.loads((public_target / "recording.json").read_text())
            self.assertNotEqual(public_manifest["events_sha256"], public_manifest["source_events_sha256"])
            self.assertEqual(public_manifest["publication_status"], "review_required")
            with self.assertRaises(FileExistsError):
                exporter.export(run, target)
            rows[-1]["event_type"] = "work.started"
            (run / "events.jsonl.gz").write_bytes(gzip.compress(("\n".join(json.dumps(row) for row in rows)).encode()))
            with self.assertRaisesRegex(ValueError, "terminal"):
                exporter.export(run, root / "invalid")
            self.assertFalse((root / "invalid").exists())


if __name__ == "__main__":
    unittest.main()
