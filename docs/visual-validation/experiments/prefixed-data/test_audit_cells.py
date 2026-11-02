"""Independent checks for the experiment's payload classification."""
import io
import tempfile
import unittest
import zipfile
from pathlib import Path
from audit_cells import audit, cells

MAIN = 'http://schemas.openxmlformats.org/spreadsheetml/2006/main'


class PayloadAuditTests(unittest.TestCase):
    def test_explicit_numeric_default_and_prefix_do_not_change_values(self):
        before = f'<c xmlns="{MAIN}" r="A1" t="n"><v>12</v></c>'
        after = f'<p:c xmlns:p="{MAIN}" r="A1"><p:v>12</p:v></p:c>'
        self.assertEqual(list(cells(io.BytesIO(before.encode()))), list(cells(io.BytesIO(after.encode()))))

    def test_only_empty_unstyled_deletions_are_classified_as_harmless(self):
        with tempfile.TemporaryDirectory() as directory:
            paths = [Path(directory) / f'{side}.xlsx' for side in ('before', 'after')]
            bodies = ['<c r="A1" t="n"/><c r="B1" s="2"/><c r="C1"><v>1</v></c>', '<c r="C1"><v>2</v></c>']
            for path, body in zip(paths, bodies):
                with zipfile.ZipFile(path, 'w') as archive:
                    archive.writestr('xl/worksheets/sheet.xml', f'<worksheet xmlns="{MAIN}"><sheetData><row r="1">{body}</row></sheetData></worksheet>')
            result = audit(*paths)
            self.assertEqual(result['removed_empty_unstyled_cells'], 1)
            self.assertEqual(result['other_changes'], 2)


if __name__ == '__main__':
    unittest.main()