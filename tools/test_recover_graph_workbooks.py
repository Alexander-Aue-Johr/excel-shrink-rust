import io
from pathlib import Path
import tempfile
import unittest
import xml.etree.ElementTree as ET
import zipfile
from unittest import mock

import recover_graph_workbooks as recovery


class GraphRecoveryTests(unittest.TestCase):
    def test_carrier_preserves_payload_and_uses_internal_relationships(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            template = root / "template.pptx"
            with zipfile.ZipFile(template, "w") as archive:
                archive.writestr("ppt/slides/slide1.xml", f'''<p:sld xmlns:p="{recovery.P}">
                    <p:cSld><p:spTree/></p:cSld></p:sld>''')
                archive.writestr("ppt/slides/_rels/slide1.xml.rels",
                                 f'<Relationships xmlns="{recovery.REL}"/>')
                archive.writestr("[Content_Types].xml", f'<Types xmlns="{recovery.CT}"/>')
            payload = b"Exact original bytes\0\xff"
            recovery.create_carrier(template, root / "carrier.pptx", payload)
            with zipfile.ZipFile(root / "carrier.pptx") as archive:
                self.assertEqual(archive.read("ppt/embeddings/graph.bin"), payload)
                slide = ET.fromstring(archive.read("ppt/slides/slide1.xml"))
                objects = list(slide.iter(f"{{{recovery.P}}}oleObj"))
                self.assertEqual(len(objects), 1)
                self.assertEqual(objects[0].get("progId"), "MSGraph.Chart.8")
                relations = ET.fromstring(archive.read("ppt/slides/_rels/slide1.xml.rels"))
                self.assertTrue(all(relation.get("TargetMode") != "External" for relation in relations))

    def test_extract_rejects_missing_or_ambiguous_workbooks(self):
        with tempfile.TemporaryDirectory() as temporary:
            path = Path(temporary) / "converted.pptx"
            for count in (0, 2):
                with zipfile.ZipFile(path, "w") as archive:
                    for index in range(count):
                        archive.writestr(f"ppt/embeddings/{index}.xlsx", b"untrusted")
                with self.assertRaisesRegex(ValueError, "Expected one"):
                    recovery.extracted_workbook(path)

    def test_extract_validates_workbook_package(self):
        with tempfile.TemporaryDirectory() as temporary:
            path = Path(temporary) / "converted.pptx"
            data = io.BytesIO()
            with zipfile.ZipFile(data, "w") as workbook:
                workbook.writestr("xl/workbook.xml", "<workbook/>")
            with zipfile.ZipFile(path, "w") as archive:
                archive.writestr("ppt/embeddings/data.xlsx", data.getvalue())
            self.assertEqual(recovery.extracted_workbook(path), data.getvalue())

    def test_workspace_rejects_changed_candidate(self):
        import argparse
        import json

        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            (root / "reports").mkdir()
            (root / "payload.bin").write_bytes(b"changed")
            row = {"path": "payload.bin", "bytes": 7, "sha256": "0" * 64}
            (root / "reports/candidates.jsonl").write_text(json.dumps(row) + "\n")
            args = argparse.Namespace(candidates=None, extraction_workspace=root)
            with recovery.candidate_payloads(args) as candidates:
                with self.assertRaisesRegex(ValueError, "checksum"):
                    list(candidates)

    def test_preparation_finishes_before_graph_recovery(self):
        import argparse
        import types

        order = []
        pipeline = types.ModuleType("prepare_enron")
        pipeline.Preparation = lambda *arguments: argparse.Namespace(arguments=arguments)
        pipeline.prepare_corpus = lambda request: order.append("extract")
        arguments = ["recover_graph_workbooks.py", "--pipeline", ".", "--source-url",
                     "https://archive.org/details/example", "--seven-zip", "7z.exe",
                     "--extraction-workspace", "workspace", "--output", "derived"]
        with mock.patch.dict("sys.modules", {"prepare_enron": pipeline}), \
                mock.patch("sys.argv", arguments), \
                mock.patch.object(recovery, "recover", side_effect=lambda args: order.append("graph")):
            recovery.main()
        self.assertEqual(order, ["extract", "graph"])

    def test_only_transient_office_busy_errors_are_retried(self):
        import types

        class ComError(Exception):
            def __init__(self, hresult):
                self.hresult = hresult

        modules = {"pywintypes": types.SimpleNamespace(com_error=ComError),
                   "pythoncom": types.SimpleNamespace(PumpWaitingMessages=lambda: None)}
        with mock.patch.dict("sys.modules", modules), mock.patch.object(recovery.time, "sleep"):
            operation = mock.Mock(side_effect=[ComError(-2147418111), "ready"])
            self.assertEqual(recovery.com_call(operation), "ready")
            self.assertEqual(operation.call_count, 2)
            operation = mock.Mock(side_effect=ComError(-1))
            with self.assertRaises(ComError):
                recovery.com_call(operation)
            self.assertEqual(operation.call_count, 1)

    def test_blocked_job_resumes_but_memory_guard_does_not(self):
        import json

        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            (root / "status.json").write_text(json.dumps({"successful_total": 1}))
            arguments = ["recover_graph_workbooks.py", "--candidates", "inputs.zip",
                         "--output", str(root)]
            with mock.patch("sys.argv", arguments), mock.patch.object(
                    recovery, "recover", side_effect=[recovery.GraphJobBlocked("blocked"), None]) as run:
                recovery.main()
                self.assertEqual(run.call_count, 2)
            with mock.patch("sys.argv", arguments), mock.patch.object(
                    recovery, "recover", side_effect=RuntimeError("RAM reserve reached")) as run:
                with self.assertRaisesRegex(RuntimeError, "RAM reserve"):
                    recovery.main()
                self.assertEqual(run.call_count, 1)


if __name__ == "__main__":
    unittest.main()
