"""Control evidence must be real and easy to find in the benchmark report."""
import importlib.util
from pathlib import Path
import struct
import sys
import tempfile
import unittest
import zlib

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT))
spec = importlib.util.spec_from_file_location('quartet_runner', ROOT / 'run.py')
runner = importlib.util.module_from_spec(spec)
spec.loader.exec_module(runner)


def png(rgb):
    def chunk(kind, payload):
        return (struct.pack('>I', len(payload)) + kind + payload +
                struct.pack('>I', zlib.crc32(kind + payload)))
    return (b'\x89PNG\r\n\x1a\n' +
            chunk(b'IHDR', struct.pack('>IIBBBBB', 1, 1, 8, 2, 0, 0, 0)) +
            chunk(b'IDAT', zlib.compress(b'\0' + bytes(rgb))) + chunk(b'IEND', b''))


class ReportTests(unittest.TestCase):
    def test_control_needs_bounds_and_both_images_and_report_prioritizes_it(self):
        with tempfile.TemporaryDirectory() as directory:
            output = Path(directory)
            workbook = 'example.xlsx'
            controls = {workbook: {'cell': 'G6'}}
            control = {'range': 'G6:G6', 'evidence': 's1-t0',
                       'difference': {'bounds': [0, 0, 1, 1]}}
            extra = {'range': 'K1:K1', 'evidence': 's1-t1',
                     'difference': {'bounds': None}}
            comparisons = [{'profile': profile,
                            'result': {'differences': [control.copy(), extra.copy()]}}
                           for profile in ('default', 'maximum', 'too-aggressive')]
            row = {'repeat': 0, 'active_groups': 1, 'status': 'complete',
                   'measured_seconds': 2.0, 'workbooks': [{
                       'status': 'complete', 'workbook': workbook,
                       'capture': {'complete': True, 'clients': {},
                                   'quartet': {'comparisons': comparisons}}}]}
            for profile in ('default', 'maximum', 'too-aggressive'):
                folder = output / 'r0-c1' / 'example' / profile
                folder.mkdir(parents=True)
                for name in ('s1-t0', 's1-t1'):
                    (folder / f'{name}-original.png').write_bytes(png((255, 255, 255)))
                    (folder / f'{name}-shrunk.png').write_bytes(png((255, 0, 0)))

            runner.validate_controls(row, controls, output)
            runner.publish(output, [row], controls)
            report = (output / 'report.html').read_text()
            self.assertLess(report.index('Kontrollzellen mit Bildbelegen'),
                            report.index('Weitere Bildunterschiede'))
            self.assertIn('<details>', report)
            self.assertEqual(report.count('<details>'), 3)
            self.assertIn('1 weiterer Befund', report)
            self.assertIn('s1-t0-original.png', report)
            self.assertIn('s1-t0-shrunk.png', report)
            self.assertIn('s1-t1-original.png', report)

            first_control = comparisons[0]['result']['differences'][0]
            first_control['difference'] = {'bounds': None}
            with self.assertRaisesRegex(ValueError, 'lacks a pixel difference'):
                runner.validate_controls(row, controls, output)
            first_control['difference'] = {'bounds': [0, 0, 1, 1]}
            (output / 'r0-c1' / 'example' / 'default' / 's1-t0-shrunk.png').unlink()
            with self.assertRaisesRegex(ValueError, 'two PNG images'):
                runner.validate_controls(row, controls, output)


if __name__ == '__main__':
    unittest.main()
