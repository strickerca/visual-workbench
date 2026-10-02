import importlib.util
import json
from pathlib import Path
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[2]
spec = importlib.util.spec_from_file_location('vdd_source', ROOT / 'drivers/sudovda/check_source.py')
source = importlib.util.module_from_spec(spec)
spec.loader.exec_module(source)

class VddSourceTests(unittest.TestCase):
    def test_current_vendor_binds_and_build_is_blocked(self):
        self.assertEqual(source.check(), 11)
        with self.assertRaisesRegex(ValueError, 'licensing/provenance'):
            source.check(build=True)

    def test_tampering_and_extra_files_fail_closed(self):
        import hashlib
        with tempfile.TemporaryDirectory(prefix='vw-vdd-source-') as directory:
            root = Path(directory)
            vendor = root / 'vendor'
            vendor.mkdir()
            file = vendor / 'test.h'
            file.write_bytes(b'reviewed source')
            manifest = {'schema':1,'commit':source.COMMIT,'complete_vendor':True,
                        'files':{'test.h':hashlib.sha256(file.read_bytes()).hexdigest()},
                        'held_for_license_provenance':{}}
            (root / 'upstream.json').write_text(json.dumps(manifest), encoding='utf-8')
            self.assertEqual(source.check(root, build=True), 1)
            file.write_bytes(b'changed source')
            with self.assertRaisesRegex(ValueError, 'hash mismatch'):
                source.check(root)
            file.write_bytes(b'reviewed source')
            (vendor / 'unreviewed.h').write_bytes(b'extra')
            with self.assertRaisesRegex(ValueError, 'Unlisted'):
                source.check(root)

    def test_vendor_path_escape_rejected(self):
        with tempfile.TemporaryDirectory(prefix='vw-vdd-source-') as directory:
            root = Path(directory)
            (root / 'upstream.json').write_text(json.dumps({'schema':1,'commit':source.COMMIT,
                'files':{'../escape.h':'0'*64}}), encoding='utf-8')
            with self.assertRaisesRegex(ValueError, 'Unsafe'):
                source.check(root)

    @unittest.skipUnless(__import__('sys').platform == 'win32', 'Windows PowerShell runner')
    def test_mutating_probe_requires_owner_window_before_device_access(self):
        import subprocess
        for scenario in ('normal', 'watchdog'):
            result = subprocess.run(['powershell.exe', '-NoProfile', '-ExecutionPolicy', 'Bypass',
                '-File', str(ROOT / 'tools/vdd-probe/run.ps1'), '-Scenario', scenario],
                cwd=ROOT, capture_output=True, text=True, timeout=60)
            self.assertNotEqual(result.returncode, 0)
            self.assertIn('Reserve this desktop', result.stdout + result.stderr)
            self.assertNotIn('t008-vdd-', result.stdout + result.stderr)

if __name__ == '__main__':
    unittest.main()
