"""Keep per-workbook Office temporary logs bounded, including failed attempts."""
import hashlib
import importlib.util
import json
from pathlib import Path
import shutil
import tempfile
from types import SimpleNamespace
import unittest
from unittest.mock import patch
import zipfile


spec = importlib.util.spec_from_file_location(
    "prepare_excel_corpus", Path(__file__).resolve().parents[1] / "prepare-excel-corpus.py")
prepare = importlib.util.module_from_spec(spec)
spec.loader.exec_module(prepare)


class TemporaryWorkspaceTests(unittest.TestCase):
    def check_attempt(self, native_failure):
        with tempfile.TemporaryDirectory() as temporary_root:
            root = Path(temporary_root)
            working = root / "work"
            working.mkdir()
            records = root / "records"
            records.mkdir()
            original = b"workbook fixture"
            bundle = root / "input.zip"
            with zipfile.ZipFile(bundle, "w") as archive:
                archive.writestr("original.xlsx", original)
            case = {"id": "fixture", "bundle": str(bundle)}
            prepare.write_json(records / "fixture.json", {
                "member_name": "fixture.xls",
                "baseline_sha256": hashlib.sha256(original).hexdigest()})
            config = {
                "working_root": str(working), "records": str(records),
                "binary": "fixture-shrinker", "launcher": "fixture-launcher",
                "cp_cache": str(root / "cp"), "control_cache": str(root / "controls"),
                "workbook_timeout_seconds": 1800}

            def execute(command, log, timeout, environment, cwd):
                local_temp = Path(environment["TEMP"])
                self.assertEqual(local_temp.parent, working / "temp")
                self.assertEqual(environment["TMP"], str(local_temp))
                diagnostics = local_temp / "Diagnostics/EXCEL"
                diagnostics.mkdir(parents=True, exist_ok=True)
                (diagnostics / "Primary.log").write_bytes(b"Office diagnostic log")
                log.write_text("Fixture command completed\n", encoding="utf8")
                if command[0] == "fixture-shrinker":
                    output = Path(command[2])
                    output.mkdir()
                    shutil.copy2(command[1], output / "original.xlsx")
                    return
                output = Path(command[command.index("-Output") + 1])
                output.mkdir()
                if native_failure:
                    prepare.write_json(output / "resident-O/session/launcher-error.json",
                                       {"error": "Fixture native failure"})
                    raise RuntimeError("Fixture native failure")
                prepare.write_json(output / "preparation.json", {
                    "status": "artifacts_prepared", "default_state": "observed"})
                prepare.write_json(output / "operations.json", [])
                (output / "original.xlsx").write_bytes(original)

            with patch.object(prepare, "execute", side_effect=execute), patch.object(
                    prepare.shutil, "disk_usage", return_value=SimpleNamespace(free=100 * 1024**3)):
                result = prepare.prepare_one(case, root, config, "main", 300)
            self.assertEqual(result["status"], "failed" if native_failure else "prepared")
            self.assertEqual(list((working / "temp").iterdir()), [])
            self.assertFalse(Path(result["execution_directory"]).exists())
            retained = Path(result["attempt"])
            self.assertTrue((retained / "inputs/original.xlsx").is_file())
            self.assertTrue((retained / "excel.log").is_file())
            if native_failure:
                error = retained / "artifacts/resident-O/session/launcher-error.json"
                self.assertEqual(json.loads(error.read_bytes())["error"], "Fixture native failure")
            else:
                self.assertEqual(prepare.checksum(Path(result["artifacts"][0]["file"])),
                                 hashlib.sha256(original).hexdigest())

    def test_success_cleans_office_logs_after_retaining_artifacts(self):
        self.check_attempt(native_failure=False)

    def test_failure_cleans_office_logs_after_retaining_error(self):
        self.check_attempt(native_failure=True)


if __name__ == "__main__":
    unittest.main()
