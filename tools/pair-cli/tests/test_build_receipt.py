"""A failed or mixed-source build must never authorize a hardware run."""
import contextlib
import importlib.util
import io
import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch


SPEC = importlib.util.spec_from_file_location('pair_build_receipt', Path(__file__).parents[1] / 'build_receipt.py')
BUILD = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(BUILD)


class BuildBindingTest(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory(prefix='vw-pair-binding-')
        self.addCleanup(self.directory.cleanup)
        self.root = Path(self.directory.name)
        for name in BUILD.BINARIES:
            path = self.root / name
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_bytes(b'synthetic executable fixture')
        self.sources = {'synthetic.rs': 'initial source hash'}
        self.patches = [patch.object(BUILD, 'ROOT', self.root), patch.object(BUILD, 'RECEIPT', self.root / '.local/build.json'), patch.object(BUILD, 'START', self.root / '.local/start.json'), patch.object(BUILD, 'sources', side_effect=lambda: dict(self.sources))]
        for item in self.patches:
            item.start()
            self.addCleanup(item.stop)

    def run_mode(self, mode):
        with patch('sys.argv', ['build_receipt.py', mode]), contextlib.redirect_stdout(io.StringIO()):
            BUILD.main()

    def test_cached_executables_can_bind_to_unchanged_prebuild_source(self):
        self.run_mode('begin')
        self.run_mode('record')
        self.run_mode('check')
        value = json.loads(BUILD.RECEIPT.read_text())
        self.assertTrue(value['source_snapshot_captured_before_build'])
        self.assertEqual(value['source_sha256'], self.sources)

    def test_source_changed_between_targets_cannot_publish_success(self):
        self.run_mode('begin')
        self.sources['synthetic.rs'] = 'changed between target builds'
        with self.assertRaisesRegex(ValueError, 'changed between target builds'):
            self.run_mode('record')
        self.assertFalse(BUILD.RECEIPT.exists())

    def test_failed_replacement_build_invalidates_old_success(self):
        self.run_mode('begin')
        self.run_mode('record')
        self.run_mode('begin')
        self.assertFalse(BUILD.RECEIPT.exists())
        with self.assertRaises(FileNotFoundError):
            self.run_mode('check')

    def test_replaced_binary_or_new_source_invalidates_hardware_admission(self):
        self.run_mode('begin')
        self.run_mode('record')
        (self.root / BUILD.BINARIES[1]).write_bytes(b'different executable fixture')
        with self.assertRaisesRegex(ValueError, 'Source or binaries changed'):
            self.run_mode('check')
        (self.root / BUILD.BINARIES[1]).write_bytes(b'synthetic executable fixture')
        self.sources['new_source.rs'] = 'new source hash'
        with self.assertRaisesRegex(ValueError, 'Source or binaries changed'):
            self.run_mode('check')


if __name__ == '__main__':
    unittest.main()
