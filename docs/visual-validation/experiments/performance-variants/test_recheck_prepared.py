"""Exercise recheck identity and fixed-destination extraction without Excel."""

import hashlib
import io
import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch
import zipfile

from compare_variants import member_identities, sha256
from recheck_prepared import recheck


def workbook(value):
    buffer = io.BytesIO()
    with zipfile.ZipFile(buffer, "w") as archive:
        archive.writestr("xl/worksheets/sheet1.xml", value)
    return buffer.getvalue()


class RecheckTests(unittest.TestCase):
    def test_unchanged_and_changed_members_preserve_the_prepared_bundle(self):
        for changed in (False, True):
            with self.subTest(changed=changed), tempfile.TemporaryDirectory() as directory:
                root = Path(directory)
                identity = "source:0:book.xls"
                group = hashlib.sha256(identity.encode()).hexdigest()[:20]
                bundle = root / f"{group}.zip"
                original = workbook(b"original")
                previous = workbook(b"previous")
                outside_name = f"escape-{root.name}.xlsx"
                converted_member = f"../../{outside_name}"
                metadata = {
                    "version": 2, "identity": identity, "source_sha256": "source",
                    "converted_member": converted_member,
                    "converted_sha256": hashlib.sha256(original).hexdigest(),
                    "baseline_sha256": hashlib.sha256(original).hexdigest(),
                    "prepared_by": "producer",
                    "variants": [{"profile": "default", "member": "default.xlsx",
                                  "sha256": hashlib.sha256(previous).hexdigest(),
                                  "config_hash": "configuration"}],
                }
                with zipfile.ZipFile(bundle, "w") as archive:
                    archive.writestr("preparation.json", json.dumps(metadata))
                    archive.writestr(converted_member, original)
                    archive.writestr("original.xlsx", original)
                    archive.writestr("default.xlsx", previous)
                before = sha256(bundle)

                def fake_shrink(binary, source, destination):
                    self.assertEqual(source.read_bytes(), original)
                    destination.mkdir()
                    output = destination / "converted.xlsx"
                    output.write_bytes(workbook(b"changed") if changed else previous)
                    return 0.1, member_identities(output)

                with patch("recheck_prepared.shrink", side_effect=fake_shrink):
                    result = recheck(bundle, root / "runner.exe", root / "work")
                self.assertEqual(sha256(bundle), before)
                self.assertEqual(result["equal_package_members"], not changed)
                self.assertEqual(result["changed_members"],
                                 ["xl/worksheets/sheet1.xml"] if changed else [])
                self.assertFalse((root.parent / outside_name).exists())

    def test_wrong_group_identity_fails_before_extraction(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            bundle = root / "wrong-group.zip"
            with zipfile.ZipFile(bundle, "w") as archive:
                archive.writestr("preparation.json", json.dumps({"version": 2, "identity": "other"}))
            with self.assertRaisesRegex(ValueError, "identity"):
                recheck(bundle, root / "runner.exe", root / "work")
            self.assertEqual(list((root / "work").iterdir()), [])


if __name__ == "__main__":
    unittest.main()
