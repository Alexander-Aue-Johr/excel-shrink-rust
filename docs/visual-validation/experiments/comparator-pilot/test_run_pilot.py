import unittest
from pathlib import Path

from run_pilot import command_for, successful_exit


class PilotTests(unittest.TestCase):
    def test_exit_counts_are_distinguished_from_failures(self):
        for code in (0, 1, 400, -4, -101, -102, 2**32 - 4):
            self.assertTrue(successful_exit("diffenginex", code))
        for code in (-1, -2, -3, -5, -9, 2**32 - 1):
            self.assertFalse(successful_exit("diffenginex", code))
        self.assertTrue(successful_exit("xlcompare", 0))
        self.assertFalse(successful_exit("xlcompare", 1))

    def test_commands_keep_paths_as_individual_arguments_and_request_reports(self):
        paths = [Path(name) for name in ("vendor.exe", "left file.xlsx", "right file.xlsx", "report.xlsx")]
        self.assertEqual(command_for("xlcompare", *paths),
                         ["vendor.exe", "left file.xlsx", "right file.xlsx", "-r:report.xlsx"])
        command = command_for("diffenginex", *paths)
        self.assertIn("/inbook1:left file.xlsx", command)
        self.assertIn("/report:report.xlsx", command)
        self.assertIn("/show:false", command)


if __name__ == "__main__":
    unittest.main()
