"""Validate and summarize pen-probe JSON without guessing hardware capabilities.

Also compares two "Copy timings" texts from the app's Pen path timings panel:

    python tools/pen-trace/trace_tool.py timings before.txt after.txt
"""
from __future__ import annotations

import argparse
import json
import math
import pathlib
import re
import statistics
import sys

ROOT_KEYS = {'schema_version', 'name', 'origin', 'device', 'clock', 'timestamp_resolution_ns',
             'viewport', 'stop_reason', 'input_devices', 'events', 'timing'}
MOTION_KEYS = {'kind', 'action', 'button_state', 'action_button', 'flags', 'meta_state', 'source',
               'device_id', 'down_time_ns', 'edge_flags', 'x_precision', 'y_precision',
               'classification', 'received_ns', 'frames'}
KEY_KEYS = {'kind', 'action', 'key_code', 'scan_code', 'repeat', 'flags', 'meta_state',
            'device_id', 'source', 'time_ns', 'down_time_ns'}
REQUIRED = ([f'line-slow-{i:02d}' for i in range(1, 3)] + [f'line-fast-{i:02d}' for i in range(1, 4)]
            + [f'{kind}-{i:02d}' for kind in ('circle', 'word-ink', 'flick') for i in range(1, 4)]
            + [f'{kind}-{i:02d}' for kind in ('hover', 'palm', 'button', 'tilt-right', 'tilt-bottom') for i in range(1, 3)])
NAMES = set(REQUIRED) | {'air-command-on', 'air-command-off', 'air-actions', 'eraser', 'thermal'}


def exact(value, keys):
    if not isinstance(value, dict) or set(value) != set(keys):
        raise ValueError('Missing or unexpected JSON fields; unreviewed metadata is not publishable')


def number(value, low=0, high=2**63 - 1, integer=True):
    if isinstance(value, bool) or not isinstance(value, int if integer else (int, float)):
        raise ValueError('Invalid numeric field')
    if not math.isfinite(value) or not low <= value <= high:
        raise ValueError('Numeric field outside bounds')


def load(path):
    path = pathlib.Path(path)
    if path.stat().st_size > 32 * 1024 * 1024:
        raise ValueError('Trace exceeds 32 MiB bound')
    def pairs(items):
        result = {}
        for key, value in items:
            if key in result:
                raise ValueError('Duplicate JSON key')
            result[key] = value
        return result
    trace = json.loads(path.read_text(encoding='utf-8'), object_pairs_hook=pairs)
    validate(trace)
    return trace


def validate(trace):
    exact(trace, ROOT_KEYS)
    if trace['schema_version'] != 1 or trace['clock'] != 'android_uptime_ns':
        raise ValueError('Unsupported trace format or clock')
    if trace['origin'] not in ('owner', 'synthetic', 'replay'):
        raise ValueError('Unknown provenance')
    name = trace['name']
    if not isinstance(name, str) or len(name) > 64 or not all(c in 'abcdefghijklmnopqrstuvwxyz0123456789-' for c in name) or not name:
        raise ValueError('Invalid trace name')
    if trace['origin'] == 'owner' and name not in NAMES:
        raise ValueError('Owner trace name must be one of the recording tasks')
    if trace['stop_reason'] not in ('saved', 'interrupted_background', 'sample_limit', 'event_limit', 'replay_complete'):
        raise ValueError('Trace not stopped or unknown stop reason')
    if trace['timestamp_resolution_ns'] not in (1, 1000000):
        raise ValueError('Unknown timestamp resolution')
    exact(trace['device'], {'manufacturer', 'model', 'sdk'})
    for key in ('manufacturer', 'model'):
        value = trace['device'][key]
        if not isinstance(value, str) or not value or len(value) > 80 or not all(c.isascii() and (c.isalnum() or c in ' _-.') for c in value):
            raise ValueError('Unexpected device metadata')
    number(trace['device']['sdk'], 29, 100)
    exact(trace['viewport'], {'width', 'height'})
    for dimension in trace['viewport'].values():
        number(dimension, 1, 16384)
    if not isinstance(trace['input_devices'], list) or len(trace['input_devices']) > 64:
        raise ValueError('Invalid device census')
    for device in trace['input_devices']:
        exact(device, {'device_id', 'sources', 'declares_stylus', 'virtual', 'ranges'})
        number(device['device_id'], -1, 2**31 - 1); number(device['sources'], 0, 2**31 - 1)
        if type(device['declares_stylus']) is not bool or type(device['virtual']) is not bool:
            raise ValueError('Invalid capability flag')
        if not isinstance(device['ranges'], list) or len(device['ranges']) > 256:
            raise ValueError('Invalid motion ranges')
        for axis in device['ranges']:
            exact(axis, {'axis', 'source', 'min', 'max', 'flat', 'fuzz', 'resolution'})
            number(axis['axis'], 0, 63); number(axis['source'], 0, 2**31 - 1)
            for field in ('min', 'max', 'flat', 'fuzz', 'resolution'):
                number(axis[field], -1e9, 1e9, False)
    events = trace['events']
    if not isinstance(events, list) or not 1 <= len(events) <= 30000:
        raise ValueError('Trace must contain 1..30000 events')
    previous = {}
    sample_count = 0
    for event in events:
        if event.get('kind') == 'key':
            exact(event, KEY_KEYS)
            for key in KEY_KEYS - {'kind'}:
                number(event[key], -1 if key == 'device_id' else 0)
            if event['action'] not in (0, 1, 2):
                raise ValueError('Invalid key action')
            times = [event['time_ns']]
        else:
            exact(event, MOTION_KEYS)
            if event['kind'] != 'motion':
                raise ValueError('Unknown event kind')
            for key in MOTION_KEYS - {'kind', 'frames', 'x_precision', 'y_precision'}:
                number(event[key], -1 if key == 'device_id' else 0)
            for key in ('x_precision', 'y_precision'):
                number(event[key], 0, 1e6, False)
            action = event['action'] & 255
            if action not in range(13):
                raise ValueError('Unknown motion action')
            frames = event['frames']
            if not isinstance(frames, list) or not 1 <= len(frames) <= 30000:
                raise ValueError('Invalid history')
            if len(frames) > 1 and action not in (2, 7):
                raise ValueError('History allowed only for move or hover-move')
            times = []
            properties = None
            for index, frame in enumerate(frames):
                exact(frame, {'time_ns', 'historical', 'pointers'})
                number(frame['time_ns'])
                if type(frame['historical']) is not bool or frame['historical'] != (index < len(frames) - 1):
                    raise ValueError('Invalid historical marker')
                pointers = frame['pointers']
                if not isinstance(pointers, list) or not 1 <= len(pointers) <= 16:
                    raise ValueError('Invalid pointer count')
                current = []
                for pointer in pointers:
                    exact(pointer, {'id', 'tool', 'axes'})
                    number(pointer['id'], 0, 31); number(pointer['tool'], 0, 5)
                    current.append((pointer['id'], pointer['tool']))
                    if not isinstance(pointer['axes'], list) or len(pointer['axes']) != 64:
                        raise ValueError('Exactly 64 axes required per sample')
                    for value in pointer['axes']:
                        number(value, -1e9, 1e9, False)
                    sample_count += 1
                if len({p[0] for p in current}) != len(current) or (properties is not None and current != properties):
                    raise ValueError('Duplicate or changed pointer properties inside a batch')
                properties = current
                if action in (5, 6) and event['action'] >> 8 >= len(current):
                    raise ValueError('Action pointer index out of bounds')
                times.append(frame['time_ns'])
        stream = (event['kind'], event['device_id'], event['source'])
        if times != sorted(times) or times[0] < previous.get(stream, -1) or event['down_time_ns'] > times[0]:
            raise ValueError('Non-monotonic event/down time')
        previous[stream] = times[-1]
    if sample_count > 30000:
        raise ValueError('Sample bound exceeded')
    if not isinstance(trace['timing'], list) or len(trace['timing']) > 30000:
        raise ValueError('Invalid timing list')
    for row in trace['timing']:
        exact(row, {'kind', 'input_ns', 'callback_ns', 'frame_ns'})
        if row['kind'] not in ('front_buffer_draw_callback', 'choreographer_callback'):
            raise ValueError('Unknown latency method')
        number(row['input_ns']); number(row['callback_ns'])
        if row['frame_ns'] is not None:
            number(row['frame_ns'])
    return sample_count


def summarize(trace):
    samples = []
    intervals = {'contact': [], 'hover': []}
    previous = {}
    for event in trace['events']:
        if event['kind'] != 'motion':
            continue
        action = event['action'] & 255
        group = 'hover' if action in (7, 9, 10) else 'contact'
        for frame in event['frames']:
            for pointer in frame['pointers']:
                if pointer['tool'] not in (2, 4):
                    continue
                key = (event['device_id'], pointer['id'], group)
                now = frame['time_ns']
                if action in (0, 5, 9):
                    previous.pop(key, None)
                if key in previous and now > previous[key]:
                    intervals[group].append(now - previous[key])
                previous[key] = now
                if action in (1, 3, 6, 10):
                    previous.pop(key, None)
                samples.append((event, frame, pointer))
    contact = [p['axes'] for e, f, p in samples if e['action'] & 255 in (0, 1, 2, 5, 6)]
    pressure = sorted({axes[2] for axes in contact})
    def axis_range(axis):
        values = [p['axes'][axis] for e, f, p in samples]
        return [min(values), max(values)] if values else None
    latency = {}
    for kind in ('front_buffer_draw_callback', 'choreographer_callback'):
        values = sorted((r['callback_ns'] - r['input_ns']) / 1e6 for r in trace['timing'] if r['kind'] == kind and r['callback_ns'] >= r['input_ns'])
        latency[kind] = {'count': len(values), 'median_ms': statistics.median(values) if values else None,
                         'p95_ms': values[math.ceil(len(values) * .95) - 1] if values else None}
    return {'name': trace['name'], 'origin': trace['origin'], 'device': trace['device'],
            'stop_reason': trace['stop_reason'], 'motion_samples': validate(trace), 'stylus_samples': len(samples),
            'observed_pressure_values': len(pressure), 'smallest_observed_pressure_step': min((b - a for a, b in zip(pressure, pressure[1:])), default=None),
            'tilt_radians': axis_range(25), 'orientation_radians': axis_range(8), 'distance': axis_range(24),
            'rates': {group: {'interval_count': len(values), 'median_interval_ns': statistics.median(values) if values else None,
                              'hz': 1e9 / statistics.median(values) if values else None} for group, values in intervals.items()},
            'button_states': sorted({e['button_state'] for e, f, p in samples}),
            'air_action_key_codes': sorted({e['key_code'] for e in trace['events'] if e['kind'] == 'key' and 131 <= e['key_code'] <= 138}),
            'latency_estimates': latency, 'presentation_latency_measured': False,
            'capability_note': 'Observed values only. Zero axis values do not establish absent hardware support; inspect declared ranges and owner calibration.'}


def acceptance(traces):
    names = set()
    for trace in traces:
        validate(trace)
        if trace['origin'] != 'owner' or trace['stop_reason'] != 'saved':
            raise ValueError('Acceptance requires complete owner traces')
        if trace['device']['manufacturer'].lower() != 'samsung' or not trace['device']['model'].startswith('SM-S918'):
            raise ValueError('Acceptance requires the S23 Ultra; another phone is software evidence only')
        if summarize(trace)['stylus_samples'] == 0:
            raise ValueError('Owner trace contains no stylus samples')
        names.add(trace['name'])
    missing = sorted(set(REQUIRED) - names)
    if missing:
        raise ValueError(f'Missing {len(missing)} required owner recording categories: ' + ', '.join(missing))
    return {'owner_traces': len(traces), 'required_categories': len(REQUIRED), 'recording_acceptance': 'passed',
            'replay_and_thermal_acceptance': 'separate checks required'}


TIMING_KEYS = ('canvas_ink_drawn', 'canvas_frame_ready', 'canvas_draw_time', 'canvas_first_ink',
               'canvas_stroke_start', 'canvas_save', 'canvas_refresh', 'remote_sent', 'remote_echo')
TIMING_ROW = re.compile(r'([a-z_]+) n=([0-9]{1,9}) p50_ms=([0-9]{1,6}\.[0-9]) p95_ms=([0-9]{1,6}\.[0-9]) max_ms=([0-9]{1,6}\.[0-9])')
SWITCHES_ROW = re.compile(r'switches=(none|[A-Za-z]{3,40}(,[A-Za-z]{3,40}){0,31})')
# Pen-downs that asked for ink: how many were drawn, how many were refused, and
# each refusal reason that occurred with its count.
PEN_DOWN_REASONS = ('backlog', 'busy', 'modal', 'loading', 'stream', 'lists', 'failed')
PEN_DOWNS_ROW = re.compile(r'pen_downs accepted=([0-9]{1,9}) refused=([0-9]{1,9})((?: [a-z_]{3,16}=[0-9]{1,9}){0,7})')
# Targets from the product's performance table. The app's timings end when the
# frame is handed to the display, so staying under a target here is necessary,
# not sufficient: the camera or input-to-present measurement decides.
TIMING_TARGETS_P95_MS = {'canvas_ink_drawn': 25.0, 'canvas_frame_ready': 25.0, 'canvas_first_ink': 25.0, 'remote_echo': 80.0}
# A change smaller than this, or than a tenth of the earlier value, is not called better or worse.
TIMING_NOISE_MS = 1.0
# One value per stroke for these; one per ink frame or pen sample for the rest.
TIMING_PER_STROKE = ('canvas_first_ink', 'canvas_stroke_start', 'canvas_save', 'canvas_refresh')


def parse_timings(text):
    """The exact text "Copy timings" produces: fixed names, counts and durations only."""
    if len(text) > 16384:
        raise ValueError('Timings text exceeds 16 KiB bound')
    lines = text.strip().splitlines()
    if len(lines) < 2 or lines[0] != 'pen_path_timings v1':
        raise ValueError('Not a copied pen path timings text')
    switches = SWITCHES_ROW.fullmatch(lines[-1])
    if not switches:
        raise ValueError('Missing or malformed switches line')
    rows = {}
    pen_downs = None
    for line in lines[1:-1]:
        downs = PEN_DOWNS_ROW.fullmatch(line)
        if downs:
            reasons = dict(pair.split('=') for pair in downs.group(3).split())
            if pen_downs is not None or len(reasons) != len(downs.group(3).split()) or any(key not in PEN_DOWN_REASONS for key in reasons):
                raise ValueError('Unexpected pen-down line; only the fixed reasons are accepted, once each')
            reasons = {key: int(value) for key, value in reasons.items()}
            if any(value < 1 for value in reasons.values()) or sum(reasons.values()) != int(downs.group(2)):
                raise ValueError('Inconsistent pen-down line')
            pen_downs = {'accepted': int(downs.group(1)), 'refused': int(downs.group(2)), 'reasons': reasons}
            continue
        match = TIMING_ROW.fullmatch(line)
        if not match or match.group(1) not in TIMING_KEYS or match.group(1) in rows:
            raise ValueError('Unexpected timing line; only the fixed timing names are accepted')
        count = int(match.group(2)); p50, p95, worst = (float(match.group(i)) for i in (3, 4, 5))
        if count < 1 or not p50 <= p95 <= worst:
            raise ValueError('Inconsistent timing line')
        rows[match.group(1)] = {'count': count, 'p50_ms': p50, 'p95_ms': p95, 'max_ms': worst}
    on = [] if switches.group(1) == 'none' else switches.group(1).split(',')
    if len(set(on)) != len(on):
        raise ValueError('Duplicate switch name')
    result = {'timings': rows, 'switches': sorted(on)}
    if pen_downs is not None:
        result['pen_downs'] = pen_downs
    return result


def compare_timings(before, after=None):
    """One capture: each timing against its target. Two: also the change, and
    for the switches that differ whether to keep them on."""
    def judged(key, row):
        target = TIMING_TARGETS_P95_MS.get(key)
        return dict(row, target_p95_ms=target, within_target=None if target is None else row['p95_ms'] <= target)
    def capture(value):
        shown = {'switches': value['switches'], 'timings': {k: judged(k, v) for k, v in value['timings'].items()}}
        if 'pen_downs' in value:
            shown['pen_downs'] = value['pen_downs']
        return shown
    result = {'before': capture(before)}
    if after is None:
        return result
    result['after'] = capture(after)
    changes = {}
    worse = better = False
    for key in TIMING_KEYS:
        a, b = before['timings'].get(key), after['timings'].get(key)
        if a is None or b is None:
            continue
        delta = round(b['p95_ms'] - a['p95_ms'], 1)
        noise = max(TIMING_NOISE_MS, a['p95_ms'] * .1)
        verdict = 'better' if delta < -noise else 'worse' if delta > noise else 'same'
        enough = min(a['count'], b['count']) >= (20 if key in TIMING_PER_STROKE else 100)
        changes[key] = {'p50_change_ms': round(b['p50_ms'] - a['p50_ms'], 1), 'p95_change_ms': delta,
                        'verdict': verdict if enough else 'too_few_samples'}
        if enough:
            worse = worse or verdict == 'worse'
            better = better or verdict == 'better'
    result['changes'] = changes
    result['switches_changed'] = sorted(set(before['switches']) ^ set(after['switches']))
    result['recommendation'] = ('switch_back_off' if worse else 'leave_on' if better else 'no_measurable_difference') \
        if changes else 'no_common_timings'
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument('command', choices=['validate', 'summarize', 'acceptance', 'timings'])
    parser.add_argument('path', type=pathlib.Path)
    parser.add_argument('other', type=pathlib.Path, nargs='?', help='timings only: the later capture to compare with')
    args = parser.parse_args()
    if args.command == 'timings':
        before = parse_timings(args.path.read_text(encoding='utf-8'))
        after = parse_timings(args.other.read_text(encoding='utf-8')) if args.other else None
        print(json.dumps(compare_timings(before, after), indent=2, allow_nan=False))
        return
    if args.other is not None:
        raise ValueError('A second path is accepted only by the timings command')
    paths = sorted(args.path.glob('*.json')) if args.path.is_dir() else [args.path]
    traces = [load(path) for path in paths]
    if args.command == 'acceptance':
        result = acceptance(traces)
    elif not traces:
        raise ValueError('No trace JSON files found')
    elif args.command == 'validate':
        result = {'traces': len(traces), 'samples': sum(validate(trace) for trace in traces)}
    else:
        result = [summarize(trace) for trace in traces]
    print(json.dumps(result, indent=2, allow_nan=False))


if __name__ == '__main__':
    try:
        main()
    except (ValueError, OSError, KeyError, TypeError, OverflowError) as error:
        print(f'TRACE: FAIL ({error})', file=sys.stderr)
        sys.exit(1)
