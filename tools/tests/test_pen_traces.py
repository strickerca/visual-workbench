"""Trace integrity, hardware provenance and numerical summary regression tests."""
import copy
import importlib.util
import pathlib
import unittest

SPEC = importlib.util.spec_from_file_location('trace_tool', pathlib.Path(__file__).parents[1] / 'pen-trace/trace_tool.py')
trace_tool = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(trace_tool)


def fixture():
    def event(action, time, pressure):
        axes = [0.0] * 64
        axes[0:4] = [30.0, 40.0, pressure, .1]
        axes[8] = -.7; axes[25] = .4
        return dict(kind='motion', action=action, button_state=0, action_button=0, flags=0,
                    meta_state=0, source=16386, device_id=0, down_time_ns=1000000000,
                    edge_flags=0, x_precision=1.0, y_precision=1.0, classification=0,
                    received_ns=time + 1000000, frames=[dict(time_ns=time, historical=False,
                        pointers=[dict(id=0, tool=2, axes=axes)])])
    return dict(schema_version=1, name='line-slow-01', origin='synthetic', device=dict(manufacturer='synthetic', model='fixture', sdk=30),
                clock='android_uptime_ns', timestamp_resolution_ns=1000000, viewport=dict(width=100, height=100),
                stop_reason='saved', input_devices=[], events=[event(0, 1000000000, .2), event(2, 1010000000, .4), event(1, 1020000000, .6)], timing=[])


class PenTraces(unittest.TestCase):
    def test_count_rate_pressure_and_sign(self):
        result = trace_tool.summarize(fixture())
        self.assertEqual(result['motion_samples'], 3)
        self.assertEqual(result['rates']['contact']['hz'], 100)
        self.assertAlmostEqual(result['smallest_observed_pressure_step'], .2)
        self.assertEqual(result['orientation_radians'], [-.7, -.7])

    def test_histories_are_counted(self):
        trace = fixture()
        previous = copy.deepcopy(trace['events'][1]['frames'][0])
        previous['time_ns'] -= 5000000; previous['historical'] = True
        trace['events'][1]['frames'].insert(0, previous)
        self.assertEqual(trace_tool.validate(trace), 4)

    def test_no_interval_across_strokes(self):
        trace = fixture()
        second = copy.deepcopy(trace['events'])
        for event in second:
            event['down_time_ns'] += 10000000000
            event['frames'][0]['time_ns'] += 10000000000
        trace['events'] += second
        self.assertEqual(trace_tool.summarize(trace)['rates']['contact']['interval_count'], 4)

    def test_finger_pressure_is_not_stylus(self):
        trace = fixture()
        for event in trace['events']:
            event['frames'][0]['pointers'][0]['tool'] = 1
        result = trace_tool.summarize(trace)
        self.assertEqual(result['stylus_samples'], 0)
        self.assertIsNone(result['rates']['contact']['hz'])

    def test_reject_unknown_identifying_metadata(self):
        trace = fixture(); trace['device']['serial'] = 'synthetic'
        with self.assertRaises(ValueError): trace_tool.validate(trace)

    def test_reject_nonfinite_axes(self):
        trace = fixture(); trace['events'][0]['frames'][0]['pointers'][0]['axes'][2] = float('nan')
        with self.assertRaises(ValueError): trace_tool.validate(trace)

    def test_reject_nonmonotonic_time(self):
        trace = fixture(); trace['events'][1]['frames'][0]['time_ns'] = 900000000
        with self.assertRaises(ValueError): trace_tool.validate(trace)

    def test_key_delivery_can_interleave_motion_history(self):
        trace = fixture()
        key = dict(kind='key', action=0, key_code=131, scan_code=0, repeat=0, flags=0,
                   meta_state=0, device_id=0, source=257, time_ns=1015000000, down_time_ns=1015000000)
        trace['events'].insert(1, key)
        self.assertEqual(trace_tool.validate(trace), 3)

    def test_reject_duplicate_pointer_ids(self):
        trace = fixture(); pointers = trace['events'][0]['frames'][0]['pointers']; pointers.append(copy.deepcopy(pointers[0]))
        with self.assertRaises(ValueError): trace_tool.validate(trace)

    def test_synthetic_cannot_satisfy_acceptance(self):
        with self.assertRaises(ValueError): trace_tool.acceptance([fixture()] * 24)

    def test_other_phone_cannot_satisfy_acceptance(self):
        trace = fixture(); trace['origin'] = 'owner'
        with self.assertRaises(ValueError): trace_tool.acceptance([trace] * 24)

    def test_duplicate_categories_do_not_satisfy_acceptance(self):
        trace = fixture(); trace['origin'] = 'owner'; trace['device'] = dict(manufacturer='samsung', model='SM-S918B', sdk=36)
        with self.assertRaises(ValueError): trace_tool.acceptance([trace] * 24)

    def test_empty_acceptance_fails(self):
        with self.assertRaises(ValueError): trace_tool.acceptance([])

    BEFORE = ('pen_path_timings v1\n'
              'canvas_ink_drawn n=640 p50_ms=9.0 p95_ms=31.0 max_ms=58.0\n'
              'canvas_first_ink n=40 p50_ms=14.0 p95_ms=22.0 max_ms=30.0\n'
              'canvas_refresh n=120 p50_ms=180.0 p95_ms=260.0 max_ms=400.0\n'
              'switches=none\n')
    AFTER = ('pen_path_timings v1\n'
             'canvas_ink_drawn n=700 p50_ms=6.0 p95_ms=12.0 max_ms=21.0\n'
             'canvas_first_ink n=42 p50_ms=13.0 p95_ms=21.5 max_ms=29.0\n'
             'canvas_refresh n=130 p50_ms=4.0 p95_ms=9.0 max_ms=15.0\n'
             'remote_sent n=3 p50_ms=0.4 p95_ms=0.9 max_ms=0.9\n'
             'pen_downs accepted=42 refused=0\n'
             'switches=canvasInkOnly\n')

    def test_copied_timings_are_read_and_judged_against_the_targets(self):
        one = trace_tool.compare_timings(trace_tool.parse_timings(self.BEFORE))
        ink = one['before']['timings']['canvas_ink_drawn']
        self.assertEqual((ink['count'], ink['p95_ms'], ink['target_p95_ms'], ink['within_target']), (640, 31.0, 25.0, False))
        self.assertIsNone(one['before']['timings']['canvas_refresh']['within_target'])
        self.assertNotIn('changes', one)

    def test_two_captures_give_the_change_and_what_to_do_with_the_switch(self):
        result = trace_tool.compare_timings(trace_tool.parse_timings(self.BEFORE), trace_tool.parse_timings(self.AFTER))
        self.assertEqual(result['switches_changed'], ['canvasInkOnly'])
        # Pen-downs that asked for ink are carried through as counted.
        self.assertNotIn('pen_downs', result['before'])
        self.assertEqual(result['after']['pen_downs'], {'accepted': 42, 'refused': 0, 'reasons': {}})
        refused = trace_tool.parse_timings(self.AFTER.replace('refused=0', 'refused=3 backlog=1 modal=2'))['pen_downs']
        self.assertEqual(refused, {'accepted': 42, 'refused': 3, 'reasons': {'backlog': 1, 'modal': 2}})
        self.assertEqual(result['changes']['canvas_ink_drawn'], {'p50_change_ms': -3.0, 'p95_change_ms': -19.0, 'verdict': 'better'})
        self.assertEqual(result['changes']['canvas_refresh']['verdict'], 'better')
        # Half a millisecond on 22 ms is within the noise.
        self.assertEqual(result['changes']['canvas_first_ink']['verdict'], 'same')
        self.assertNotIn('remote_sent', result['changes'])
        few = trace_tool.compare_timings(trace_tool.parse_timings(self.BEFORE.replace('n=640', 'n=64').replace('n=40', 'n=12')),
                                         trace_tool.parse_timings(self.AFTER))
        self.assertEqual(few['changes']['canvas_ink_drawn']['verdict'], 'too_few_samples')
        self.assertEqual(few['changes']['canvas_first_ink']['verdict'], 'too_few_samples')
        self.assertTrue(result['after']['timings']['canvas_ink_drawn']['within_target'])
        self.assertEqual(result['recommendation'], 'leave_on')
        # The other way round the same numbers say: switch it back off.
        self.assertEqual(trace_tool.compare_timings(trace_tool.parse_timings(self.AFTER), trace_tool.parse_timings(self.BEFORE))['recommendation'], 'switch_back_off')
        self.assertEqual(trace_tool.compare_timings(trace_tool.parse_timings(self.BEFORE), trace_tool.parse_timings(self.BEFORE))['recommendation'], 'no_measurable_difference')

    def test_timings_text_with_anything_but_fixed_names_and_numbers_is_refused(self):
        for bad in ('', 'canvas_save n=1 p50_ms=1.0 p95_ms=1.0 max_ms=1.0\nswitches=none\n',
                    self.BEFORE.replace('canvas_first_ink', 'my_document_name'),
                    self.BEFORE.replace('switches=none', 'switches=none\nnote=hello'),
                    self.BEFORE.replace('p95_ms=31.0', 'p95_ms=3.0'),
                    self.BEFORE.replace('switches=none', 'switches=a b'),
                    self.BEFORE.replace('canvas_first_ink n=40', 'canvas_ink_drawn n=40'),
                    # A pen-down line names only the fixed reasons, once each, and adds up.
                    self.AFTER.replace('refused=0', 'refused=1 my_reason=1'),
                    self.AFTER.replace('refused=0', 'refused=2 modal=1 modal=1'),
                    self.AFTER.replace('refused=0', 'refused=3 modal=1'),
                    self.AFTER.replace('refused=0', 'refused=0 modal=0'),
                    self.AFTER.replace('pen_downs accepted=42 refused=0\n', 'pen_downs accepted=42 refused=0\npen_downs accepted=1 refused=0\n'),
                    self.BEFORE + 'x' * 20000):
            with self.assertRaises(ValueError): trace_tool.parse_timings(bad)

    def test_incomplete_trace_fails_acceptance(self):
        trace = fixture(); trace['origin'] = 'owner'; trace['stop_reason'] = 'interrupted_background'
        with self.assertRaises(ValueError): trace_tool.acceptance([trace])

    def test_complete_owner_category_set(self):
        traces = []
        for name in trace_tool.REQUIRED:
            trace = fixture(); trace['origin'] = 'owner'; trace['name'] = name
            trace['device'] = dict(manufacturer='samsung', model='SM-S918B', sdk=36)
            traces.append(trace)
        self.assertEqual(trace_tool.acceptance(traces)['required_categories'], 24)


if __name__ == '__main__': unittest.main()
