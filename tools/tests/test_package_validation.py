"""Exercise runtime publication and relocation without launching Office."""
import importlib.util
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

spec = importlib.util.spec_from_file_location("package_validation", Path(__file__).resolve().parents[1] / "package-validation.py")
package = importlib.util.module_from_spec(spec)
spec.loader.exec_module(package)


class RuntimePackageTests(unittest.TestCase):
    def fixture(self, root):
        repository = root / "repository"
        binaries = root / "binaries"
        binaries.mkdir()
        tracked = []
        for destination in package.RUNTIME_ASSETS:
            source = Path("excel-shrink-cli") / destination
            path = repository / source
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_bytes(b"runtime fixture")
            tracked.append(source.as_posix())
        for name in ("deltasieve.exe", "excel-shrink-cli.exe"):
            (binaries / name).write_bytes(b"path-only fixture, never executed")
        return repository, binaries, tracked

    def test_package_survives_relocation_and_rejects_corruption(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            repository, binaries, tracked = self.fixture(root)
            output = root / "release"
            manifest = package.package(repository, binaries, output, tracked, {"commit": "fixture"})
            moved = root / "moved"
            output.rename(moved)
            self.assertEqual(package.verify(moved), manifest)
            (moved / "deltasieve.exe").write_bytes(b"corrupt")
            with self.assertRaisesRegex(ValueError, "changed"):
                package.verify(moved)

    def test_existing_output_and_unexpected_members_are_preserved_and_rejected(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            repository, binaries, tracked = self.fixture(root)
            output = root / "release"
            package.package(repository, binaries, output, tracked, {})
            sentinel = output / "unexpected.txt"
            sentinel.write_text("preserve", encoding="utf-8")
            with self.assertRaises(FileExistsError):
                package.package(repository, binaries, output, tracked, {})
            self.assertEqual(sentinel.read_text(), "preserve")
            with self.assertRaisesRegex(ValueError, "unexpected"):
                package.verify(output)

    def test_only_runtime_members_are_selected(self):
        selected = package.runtime_members([
            "excel-shrink-cli/office-capture/web/resident.js",
            "excel-shrink-cli/office-capture/web/resident.test.js",
            "excel-shrink-cli/office-capture/tests/private.json",
            "excel-shrink-cli/office-capture/node_modules/private.js",
            "excel-shrink-cli/src/regression/archive_processes.ps1",
            "private/corpus.xlsx",
        ])
        self.assertEqual(set(selected), {Path("office-capture/web/resident.js"), Path("src/regression/archive_processes.ps1")})

    def test_interrupted_copy_leaves_an_explicit_incomplete_package(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            repository, binaries, tracked = self.fixture(root)
            output = root / "release"
            with patch.object(package.shutil, "copyfile", side_effect=OSError("fixture disk failure")):
                with self.assertRaisesRegex(OSError, "fixture disk failure"):
                    package.package(repository, binaries, output, tracked, {})
            self.assertTrue((output / ".incomplete").is_file())
            self.assertFalse((output / "release-manifest.json").exists())
            with self.assertRaisesRegex(ValueError, "incomplete"):
                package.verify(output)


if __name__ == "__main__":
    unittest.main()
