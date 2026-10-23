"""Benchmark eligibility must fail closed; these tests never launch Excel."""
import copy
import hashlib
import importlib.util
import json
from pathlib import Path
from types import SimpleNamespace
import subprocess
import sys
import tempfile
import unittest
from unittest import mock
import zipfile

from PIL import Image

ROOT = Path(__file__).resolve().parents[1]
spec = importlib.util.spec_from_file_location("study_runtime", ROOT / "compare_study_runtime.py")
bench = importlib.util.module_from_spec(spec)
spec.loader.exec_module(bench)


def fixture(output):
    output.mkdir(parents=True,exist_ok=True)
    (output / "records").mkdir(exist_ok=True)
    Image.new("RGB",(2,2),(255,255,255)).save(output / "before.png")
    Image.new("RGB",(2,2),(255,0,0)).save(output / "after.png")
    profiles = [{"profile":name,"config_hash":name,"checks_complete":True,"checked_sheets":1,
                 "unchanged_sheets":1,"status":"visually unchanged","sheets":[],
                 "stored_value_differences":0,"recalculated_value_differences":0,"value_differences":0}
                for name in bench.PROFILES]
    profiles[0].update(status="visual differences", unchanged_sheets=0, sheets=[{
        "index":1,"name":"Sheet1","status":"changed","row_heights":[],
        "image_differences":[{"range":"A1:B2","region":[0,0,2,2],"original_image":"before.png","shrunk_image":"after.png"}]
    }])
    sha256=hashlib.sha256(b"original").hexdigest()
    record={"identity":"archive:0:file.xlsx","source_sha256":sha256,"baseline_sha256":sha256,
            "complete":True,"status":"differences","results":profiles,
            "whitespace_diagnosis":{"status":"complete","profiles":[{
                "profile":"too-aggressive-cleaning", "base_config_hash":"base", "control_config_hash":"control",
                "sheets":[{"index":1,"category":"whitespace_explained","checks_complete":True,"original_evidence_reproduced":True}],
                "control":{"profile":"too-aggressive-cleaning without whitespace deletion", "checks_complete":True,
                           "checked_sheets":1,"status":"visually unchanged","sheets":[]}
            }]}}
    return record


def persist(output,record):
    (output / "records" / "record.json").write_text(json.dumps(record))


def signature_hash(record,output):
    return hashlib.sha256(bench.canonical(bench.record_signature(record,output)).encode()).hexdigest()


class StudyRuntimeTests(unittest.TestCase):
    def test_missing_profile_incomplete_sheet_and_skips_are_not_eligible(self):
        with tempfile.TemporaryDirectory() as directory:
            record=fixture(Path(directory))
            missing=copy.deepcopy(record)
            missing["results"].pop()
            with self.assertRaisesRegex(ValueError,"missing or duplicate profile"):
                bench.validate_record(missing)
            missing=copy.deepcopy(record)
            missing["results"][1].update(status="not compared",checks_complete=False)
            with self.assertRaisesRegex(ValueError,"missing required comparison"):
                bench.validate_record(missing)
            failed=copy.deepcopy(record)
            failed["status"]="skipped"
            with self.assertRaisesRegex(ValueError,"technically failed workbook"):
                bench.validate_record(failed)
            failed=copy.deepcopy(record)
            failed["results"][0]["sheets"][0]["image_differences"][0].pop("shrunk_image")
            with self.assertRaisesRegex(ValueError,"both PNG"):
                bench.validate_record(failed)

    def test_exit_one_is_accepted_only_for_complete_differences_with_exact_coverage(self):
        with tempfile.TemporaryDirectory() as directory:
            output=Path(directory)
            record=fixture(output)
            persist(output,record)
            expected={record["identity"]:{"source_sha256":record["source_sha256"]}}
            records,signature=bench.validate_output(output,expected,1)
            self.assertEqual(len(signature),1)
            self.assertEqual(records[0]["status"],"differences")
            with self.assertRaisesRegex(ValueError,"coverage differs"):
                bench.validate_output(output,{**expected,"missing":{}},1)
            record["status"]="unchanged"
            persist(output,record)
            with self.assertRaisesRegex(ValueError,"exit code 1"):
                bench.validate_output(output,expected,1)

    def test_pixel_hash_ignores_png_encoding_but_detects_tampered_pixels(self):
        with tempfile.TemporaryDirectory() as directory:
            output=Path(directory)
            record=fixture(output)
            before=signature_hash(record,output)
            with Image.open(output / "after.png") as image:
                image.save(output / "encoded-again.png",compress_level=0)
            record["results"][0]["sheets"][0]["image_differences"][0]["shrunk_image"]="encoded-again.png"
            self.assertEqual(before,signature_hash(record,output))
            Image.new("RGB",(2,2),(0,255,0)).save(output / "encoded-again.png")
            self.assertNotEqual(before,signature_hash(record,output))

    def test_metrics_and_provenance_do_not_hide_changed_categories_or_coverage(self):
        with tempfile.TemporaryDirectory() as directory:
            output=Path(directory)
            record=fixture(output)
            before=signature_hash(record,output)
            record.update(provenance={"machine":"other"},compared_by="another-build",run_selection={"rank":99})
            record["results"][0].update(shrink_seconds=100,original_open_seconds=10,original_memory_bytes=999)
            record["whitespace_diagnosis"].update(producer="other",provenance={"machine":"other"})
            self.assertEqual(before,signature_hash(record,output))
            record["whitespace_diagnosis"]["status"]="not_investigated"
            self.assertNotEqual(before,signature_hash(record,output))
            record["whitespace_diagnosis"]["status"]="complete"
            record["whitespace_diagnosis"]["profiles"][0]["sheets"][0]["category"]="other"
            self.assertNotEqual(before,signature_hash(record,output))
            record["whitespace_diagnosis"]["profiles"][0]["sheets"][0]["category"]="whitespace_explained"
            record["results"][0]["checked_sheets"]=2
            self.assertNotEqual(before,signature_hash(record,output))

    def test_unchanged_stronger_profile_can_leave_weaker_profiles_not_compared(self):
        with tempfile.TemporaryDirectory() as directory:
            record=fixture(Path(directory))
            record.update(status="unchanged",whitespace_diagnosis={"status":"not_investigated"})
            record["results"][0].update(status="visually unchanged",sheets=[],unchanged_sheets=1)
            for profile in record["results"][1:]:
                profile.update(status="not compared",checks_complete=False,sheets=[])
            bench.validate_record(record)

    def test_missing_png_is_an_error_instead_of_equal_empty_evidence(self):
        with tempfile.TemporaryDirectory() as directory:
            output=Path(directory)
            record=fixture(output)
            (output / "before.png").unlink()
            with self.assertRaises(FileNotFoundError):
                bench.record_signature(record,output)

    def test_pairs_with_semantic_mismatch_or_failed_runs_never_get_a_speedup(self):
        runs=[{"label":"baseline","status":"complete","wall_seconds":10,"semantic_sha256":"a"},
              {"label":"candidate","status":"complete","wall_seconds":5,"semantic_sha256":"b"}]
        mismatch=bench.pair_result(1,runs,None)
        self.assertEqual(mismatch["status"],"invalid")
        self.assertNotIn("speedup",mismatch)
        runs[1]["semantic_sha256"]="a"
        good=bench.pair_result(1,runs,None)
        self.assertEqual(good["speedup"],2)
        self.assertEqual(bench.summary([mismatch,good])["median_speedup"],2)
        self.assertEqual(bench.summary([mismatch])["median_speedup"],None)
        self.assertEqual(bench.pair_result(2,runs,"different-repeat")["status"],"invalid")
        runs[1]["status"]="invalid"
        self.assertEqual(bench.pair_result(2,runs,None)["status"],"invalid")

    def test_expected_members_preserve_archive_indices_and_apply_limit_per_corpus(self):
        with tempfile.TemporaryDirectory() as directory:
            path=Path(directory) / "sample.zip"
            with zipfile.ZipFile(path,"w") as archive:
                archive.writestr("readme.txt",b"ignored")
                archive.writestr("first.xlsx",b"one")
                archive.writestr("second.XLS",b"two")
            expected=bench.expected_members([("Sample",path)],1)
            self.assertEqual(len(expected),1)
            member=next(iter(expected.values()))
            self.assertEqual(member["archive_index"],1)
            self.assertEqual(member["source_sha256"],hashlib.sha256(b"one").hexdigest())

    def test_mocked_run_uses_absolute_fresh_paths_and_measures_whole_cli_wall_time(self):
        with tempfile.TemporaryDirectory() as directory:
            output=Path(directory).resolve()
            cli=output / "fake-cli.exe"
            cli.write_bytes(b"not executed")
            expected={"archive:0:file.xlsx":{"source_sha256":hashlib.sha256(b"original").hexdigest()}}
            args=SimpleNamespace(limit=None,workbook_timeout_seconds=60,startup_timeout_seconds=30,repeats=2)
            def completed(command,**kwargs):
                study=Path(command[command.index("--output-root")+1])
                self.assertTrue(study.is_absolute())
                self.assertTrue(Path(kwargs["env"]["EXCEL_SHRINK_REGRESSION_CACHE_DIR"]).is_relative_to(output))
                record=fixture(study)
                persist(study,record)
                return subprocess.CompletedProcess(command,1)
            with mock.patch.object(bench.subprocess,"run",side_effect=completed) as run, mock.patch.object(bench.time,"perf_counter",side_effect=[10.0,12.5]):
                result=bench.run_one(cli,"baseline",1,[("Sample",output / "sample.zip")],output,expected,args)
            self.assertEqual(run.call_count,1)
            self.assertEqual(result["status"],"complete")
            self.assertEqual(result["wall_seconds"],2.5)
            self.assertTrue((output / "repeat-01-baseline" / "console.log").exists())
            self.assertTrue((output / "repeat-01-baseline" / "semantic-signature.json").exists())


if __name__ == "__main__":
    unittest.main()
