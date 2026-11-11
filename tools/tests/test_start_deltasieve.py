"""Check the ordinary starter's argument handoff without building or opening Office."""

import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest


STARTER = Path(__file__).resolve().parents[1] / "start-deltasieve.ps1"


@unittest.skipUnless(shutil.which("powershell.exe"), "Windows PowerShell is required")
class StarterTests(unittest.TestCase):
    def test_configuration_files_prefill_the_form_without_starting_a_study(self):
        with tempfile.TemporaryDirectory(prefix="starter handoff ") as temporary:
            root = Path(temporary)
            receipt = root / "arguments.json"
            binary = root / "fake runner.ps1"
            binary.write_text(
                "@{ arguments = @($args); temporary = $env:TEMP } | "
                "ConvertTo-Json -Depth 4 | "
                "Set-Content -LiteralPath $env:STARTER_TEST_RECEIPT\nexit 0\n",
                encoding="utf-8",
            )
            configuration = root / "comparison config.json"
            study = root / "study config.json"
            configuration.write_text("{}", encoding="utf-8")
            study.write_text("{}", encoding="utf-8")
            data = root / "working data"
            result = subprocess.run(
                [
                    "powershell.exe", "-NoProfile", "-ExecutionPolicy", "Bypass",
                    "-File", str(STARTER), "-BinaryPath", str(binary),
                    "-DataRoot", str(data), "-Config", str(configuration),
                    "-StudyConfig", str(study), "-Port", "18884", "-NoOpen",
                ],
                env={**os.environ, "STARTER_TEST_RECEIPT": str(receipt)},
                capture_output=True,
                text=True,
                timeout=30,
            )
            self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
            observed = json.loads(receipt.read_text(encoding="utf-8-sig"))
            self.assertEqual(observed["arguments"], [
                "observatory", "--port", "18884", "--config", str(configuration),
                "--study-config", str(study), "--no-open",
            ])
            self.assertEqual(Path(observed["temporary"]), data / "temp")
            self.assertEqual(configuration.read_text(encoding="utf-8"), "{}")
            self.assertEqual(study.read_text(encoding="utf-8"), "{}")
            self.assertFalse((data / "launch.json").exists())


if __name__ == "__main__":
    unittest.main()
