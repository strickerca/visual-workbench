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
