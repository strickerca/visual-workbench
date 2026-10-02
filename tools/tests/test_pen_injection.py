"""Offline analysis regressions; these fixtures are not Windows injection evidence."""
import copy
import csv
import importlib.util
import json
from pathlib import Path
import tempfile
import unittest

SPEC = importlib.util.spec_from_file_location('injection_analysis', Path(__file__).parents[1] / 'pen-inject' / 'analyze.py')
analysis = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(analysis)
GUARD_SPEC = importlib.util.spec_from_file_location('guard_analysis', Path(__file__).parents[1] / 'pen-inject' / 'guard_report.py')
guard_analysis = importlib.util.module_from_spec(GUARD_SPEC)
GUARD_SPEC.loader.exec_module(guard_analysis)


def ramp():
    commands, received = [], []
    for i in range(65):
        flags = (0x10000 if i == 0 else 0x20000) | 2 | 4 | 0x10
        common = dict(sequence=i, x=-900+i*8, y=120, pressure=i*16,
                      tilt_x=0, tilt_y=0, rotation=0, pen_flags=0, pointer_flags=flags)
        commands.append(dict(common, stroke=1, due_ms=i*40, actual_ms=i*40+.1,
                             phase='down' if i == 0 else 'move'))
        received.append(dict(common, message=581, pointer_id=17, frame_id=i+100,
                             time_ms=1000+i*40, performance_count=100000+i*400,
                             pen_mask=15, history_count=1, received_ms=i*40+1))
    return commands, received


class PenInjectionAnalysisTests(unittest.TestCase):
    def test_perfect_synthetic_ramp_is_math_evidence_only(self):
        result = analysis.compare(*ramp())
        self.assertAlmostEqual(result['pressure_pearson_r'], 1)
        self.assertTrue(result['pressure_target_met'])
        self.assertNotIn('harness_acceptance', result)

    def test_constant_received_pressure_fails(self):
        c, r = ramp()
        for row in r:
            row['pressure'] = 512
        result = analysis.compare(c, r)
        self.assertIsNone(result['pressure_pearson_r'])
        self.assertFalse(result['pressure_target_met'])

    def test_missing_samples_cannot_pass_on_survivor_correlation(self):
        c, r = ramp()
        result = analysis.compare(c, r[1:])
        self.assertAlmostEqual(result['pressure_pearson_r'], 1)
        self.assertFalse(result['pressure_target_met'])
        self.assertEqual(result['missing_sequences'], [0])

    def test_match_does_not_hide_pressure_errors(self):
        c, r = ramp()
        r[20]['pressure'] = 1000
        result = analysis.compare(c, r)
        self.assertEqual(result['matched_samples'], 65)
        self.assertEqual(result['field_mismatches'], {'pressure': 1})

    def test_tilt_rotation_barrel_eraser_mask_and_cancel_are_checked(self):
        c, r = ramp()
        for field in ('tilt_x', 'tilt_y', 'rotation', 'pen_flags', 'pen_mask', 'pointer_flags'):
            changed = copy.deepcopy(r)
            changed[4][field] ^= 0x8000 if field == 'pointer_flags' else 1
            self.assertIn(field, analysis.compare(c, changed)['field_mismatches'])

    def test_reordered_samples_report_order_loss(self):
        c, r = ramp()
        r[5]['sequence'], r[6]['sequence'] = r[6]['sequence'], r[5]['sequence']
        self.assertFalse(analysis.compare(c, r)['received_order_preserved'])

    def test_keepalive_miss_is_reported(self):
        c, r = ramp()
        for row in c[10:]:
            row['actual_ms'] += 1200
        result = analysis.compare(c, r)
        self.assertFalse(result['keepalive_target_met'])
        self.assertGreater(result['max_contact_interval_ms'], 1000)

    def test_os_owned_primary_flag_does_not_change_pen_equality(self):
        c, r = ramp()
        for row in r:
            row['pointer_flags'] |= 0x2000
        self.assertTrue(analysis.compare(c, r)['all_command_fields_matched'])

    def test_extra_received_samples_are_visible(self):
        c, r = ramp()
        r.append(dict(r[2], sequence=65, x=9999))
        self.assertEqual(analysis.compare(c, r)['unmatched_received_samples'], 1)

    def test_empty_csv_and_nonfinite_clock_rejected(self):
        with tempfile.TemporaryDirectory() as folder:
            path = Path(folder) / 'commands.csv'
            path.write_text(','.join(analysis.COMMAND)+'\n')
            with self.assertRaises(ValueError):
                analysis.load_csv(path, analysis.COMMAND)
            c, _ = ramp()
            c[0]['actual_ms'] = 'nan'
            path.write_text(','.join(analysis.COMMAND)+'\n'+','.join(str(c[0][k]) for k in analysis.COMMAND)+'\n')
            with self.assertRaises(ValueError):
                analysis.load_csv(path, analysis.COMMAND)

    def test_incomplete_receiver_blocks_before_csv_analysis(self):
        with tempfile.TemporaryDirectory() as folder:
            root = Path(folder)
            (root/'commands.session.json').write_text(json.dumps({'completed': True}))
            (root/'receiver.json').write_text(json.dumps({'recorder_healthy': False}))
            (root/'target.json').write_text('{}')
            with self.assertRaisesRegex(ValueError, 'Incomplete'):
                analysis.analyze(root/'commands.csv', root)


class GuardReportTests(unittest.TestCase):
    def make_receipts(self, root, trials=100, stray=0, downs=100, healthy=True):
        with (root/'commands.csv').open('w', newline='') as file:
            writer = csv.writer(file)
            writer.writerow(['trial', 'mutation', 'baseline_sent', 'changed_attempt_blocked', 'resume_attempt_blocked'])
            for i in range(trials):
                writer.writerow([i, ['move', 'resize', 'minimize', 'focus'][i % 4], 'true', 'true', 'true'])
        for role, count in [('target', downs), ('sink', stray)]:
            (root/role).mkdir()
            (root/role/'receiver.json').write_text(json.dumps({'native_samples': count, 'recorder_healthy': healthy}))
            with (root/role/'received.csv').open('w', newline='') as file:
                writer = csv.writer(file)
                writer.writerow(['pointer_flags'])
                for _ in range(count):
                    writer.writerow([0x10000])

    def test_complete_fixture_reports_only_observed_sink(self):
        with tempfile.TemporaryDirectory() as folder:
            root = Path(folder)
            self.make_receipts(root)
            result = guard_analysis.report(root)
            self.assertTrue(result['observed_sink_target_met'])
            self.assertIn('does not prove', result['scope'])

    def test_99_trials_cannot_pass(self):
        with tempfile.TemporaryDirectory() as folder:
            root = Path(folder)
            self.make_receipts(root, trials=99)
            with self.assertRaisesRegex(ValueError, '100'):
                guard_analysis.report(root)

    def test_missing_baseline_delivery_cannot_pass(self):
        with tempfile.TemporaryDirectory() as folder:
            root = Path(folder)
            self.make_receipts(root, downs=99)
            with self.assertRaisesRegex(ValueError, 'baseline'):
                guard_analysis.report(root)

    def test_stray_or_failed_recorder_cannot_pass(self):
        with tempfile.TemporaryDirectory() as folder:
            root = Path(folder)
            self.make_receipts(root, stray=1)
            self.assertFalse(guard_analysis.report(root)['observed_sink_target_met'])
            (root/'sink'/'receiver.json').write_text(json.dumps({'native_samples': 1, 'recorder_healthy': False}))
            with self.assertRaisesRegex(ValueError, 'recorder'):
                guard_analysis.report(root)

    def test_mismatched_target_blocks_before_csv_analysis(self):
        with tempfile.TemporaryDirectory() as folder:
            root = Path(folder)
            (root/'commands.session.json').write_text(json.dumps({'completed': True, 'hwnd': 1, 'pid': 2, 'dpi': 168}))
            (root/'receiver.json').write_text(json.dumps({'recorder_healthy': True}))
            (root/'target.json').write_text(json.dumps({'source': 'native_wm_pointer', 'hwnd': 3, 'pid': 2, 'dpi': 168}))
            with self.assertRaisesRegex(ValueError, 'identity'):
                analysis.analyze(root/'commands.csv', root)
