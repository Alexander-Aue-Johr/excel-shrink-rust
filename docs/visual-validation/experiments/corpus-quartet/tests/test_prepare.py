"""Prepared shrink variants require complete, matching provenance."""
import importlib.util
import json
from pathlib import Path
import shutil
import sys
import tempfile
import unittest
from unittest.mock import patch

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT))
spec = importlib.util.spec_from_file_location('quartet_prepare_runner', ROOT / 'run.py')
runner = importlib.util.module_from_spec(spec)
spec.loader.exec_module(runner)


class PrepareTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        root = Path(temporary.name)
        self.corpus = root / 'corpus'
        self.corpus.mkdir()
        self.source = self.corpus / 'kernhaushalte.xlsx'
        self.source.write_bytes(b'original workbook')
        self.cli = root / 'shrink-cli'
        self.cli.write_bytes(b'cli version 1')
        self.prepared = root / 'prepared'
        self.calls = []

    def shrink(self, command, **_kwargs):
        self.calls.append(command)
        shutil.copyfile(command[1], Path(command[2]) / Path(command[1]).name)

    def prepare(self, only=None):
        with patch.object(runner.subprocess, 'run', self.shrink):
            return runner.prepare(self.corpus, self.cli, self.prepared, only)

    def test_only_selects_four_originals_before_any_shrink(self):
        for name in ('wanderungen.xlsx', 'stromsteuerstatistik.xlsx',
                     'verkehrsunfaelle.xlsx', 'unused.xlsx'):
            (self.corpus / name).write_bytes(name.encode())
        selected = self.prepare(['kernhaushalte', 'wanderungen',
                                 'stromsteuerstatistik', 'verkehrsunfaelle'])
        self.assertEqual([entry['name'] for entry in selected], [
            'kernhaushalte.xlsx', 'wanderungen.xlsx',
            'stromsteuerstatistik.xlsx', 'verkehrsunfaelle.xlsx'])
        self.assertEqual(len(self.calls), 12)
        self.assertFalse(any('unused.xlsx' in part for call in self.calls for part in call))
        manifest = json.loads((self.prepared / 'manifest.json').read_text())
        self.assertEqual(len(manifest['entries']), 4)
        self.assertEqual(self.prepare(['kernhaushalte', 'wanderungen',
                                       'stromsteuerstatistik', 'verkehrsunfaelle']), selected)
        self.assertEqual(len(self.calls), 12)

    def test_default_four_and_invalid_selection_fail_before_shrink(self):
        for name in ('wanderungen.xlsx', 'stromsteuerstatistik.xlsx',
                     'verkehrsunfaelle.xlsx', 'unused.xlsx'):
            (self.corpus / name).write_bytes(name.encode())
        with self.assertRaisesRegex(ValueError, 'exactly four'):
            self.prepare(['kernhaushalte'])
        with self.assertRaisesRegex(ValueError, 'must match one workbook'):
            self.prepare(['.xlsx', 'wanderungen', 'stromsteuerstatistik',
                          'verkehrsunfaelle'])
        self.assertEqual(self.calls, [])
        selected = self.prepare([])
        self.assertEqual([entry['name'] for entry in selected], [
            'kernhaushalte.xlsx', 'wanderungen.xlsx',
            'stromsteuerstatistik.xlsx', 'verkehrsunfaelle.xlsx'])
        self.assertEqual(len(self.calls), 12)
        self.assertFalse((self.prepared / 'default' / 'unused.xlsx').exists())

    def test_reuse_only_with_matching_original_cli_flags_and_hashes(self):
        first = self.prepare()
        self.assertEqual(len(self.calls), 3)
        self.assertEqual(self.prepare(), first)
        self.assertEqual(len(self.calls), 3)
        manifest_path = self.prepared / 'manifest.json'
        manifest = json.loads(manifest_path.read_text())
        self.assertEqual(manifest['cli_sha256'], runner.digest(self.cli))
        self.assertEqual(manifest['profile_flags'], runner.PROFILES)

        self.cli.write_bytes(b'cli version 2')
        with self.assertRaisesRegex(RuntimeError, 'matching CLI'):
            self.prepare()
        self.cli.write_bytes(b'cli version 1')
        manifest['profile_flags']['default'] = ['--unexpected']
        manifest_path.write_text(json.dumps(manifest))
        with self.assertRaisesRegex(RuntimeError, 'matching CLI'):
            self.prepare()
        manifest['profile_flags'] = runner.PROFILES
        manifest_path.write_text(json.dumps(manifest))
        self.source.write_bytes(b'changed original')
        with self.assertRaisesRegex(RuntimeError, 'original changed'):
            self.prepare()
        self.assertEqual(len(self.calls), 3)

    def test_existing_targets_without_complete_manifest_are_rejected(self):
        target = self.prepared / 'default' / self.source.name
        target.parent.mkdir(parents=True)
        target.write_bytes(b'stale shrink output')
        with self.assertRaisesRegex(RuntimeError, 'no complete manifest'):
            self.prepare()
        self.assertEqual(self.calls, [])

        (self.prepared / 'manifest.json').write_text('[]')
        with self.assertRaisesRegex(RuntimeError, 'matching CLI'):
            self.prepare()
        self.assertEqual(self.calls, [])


if __name__ == '__main__':
    unittest.main()
