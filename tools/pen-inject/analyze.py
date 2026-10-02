"""Compare native WM_POINTER receipts with the completed T0.05 injection journal."""
from __future__ import annotations
import argparse
import collections
import csv
import hashlib
import json
import math
from pathlib import Path
import sys

COMMAND = 'sequence stroke due_ms actual_ms phase x y pressure tilt_x tilt_y rotation pen_flags pointer_flags'.split()
RECEIVED = 'sequence message pointer_id frame_id time_ms performance_count x y pressure tilt_x tilt_y rotation pen_flags pen_mask pointer_flags history_count received_ms'.split()


def load_csv(path, fields):
    path = Path(path)
    if path.stat().st_size > 8 * 1024 * 1024:
        raise ValueError('CSV exceeds 8 MiB limit')
    with path.open(encoding='utf-8', newline='') as file:
        reader = csv.DictReader(file)
        if reader.fieldnames != fields:
            raise ValueError('Unexpected CSV schema')
        rows = list(reader)
    if not 1 <= len(rows) <= 30000:
        raise ValueError('Empty or oversized sample journal')
    for index, row in enumerate(rows):
        if set(row) != set(fields) or None in row.values():
            raise ValueError('Malformed CSV row')
        for field in fields:
            if field == 'phase':
                if row[field] not in ('hover', 'down', 'move', 'up', 'leave'):
                    raise ValueError('Unknown pointer phase')
            else:
                row[field] = float(row[field]) if field in ('actual_ms', 'received_ms') else int(row[field])
                if not math.isfinite(row[field]) or abs(row[field]) > 2**64:
                    raise ValueError('Invalid numeric value')
        if row['sequence'] != index or not 0 <= row['pressure'] <= 1024 or not 0 <= row['rotation'] <= 359:
            raise ValueError('Invalid sequence/pen value')
        if not -90 <= row['tilt_x'] <= 90 or not -90 <= row['tilt_y'] <= 90:
            raise ValueError('Invalid tilt')
    return rows


def phase(flags):
    if flags & 0x40000:
        return 'up'
    if flags & 0x10000:
        return 'down'
    if flags & 4:
        return 'move'
    return 'hover' if flags & 2 else 'leave'


def pearson(pairs):
    if len(pairs) < 3:
        return None
    xmean = sum(x for x, _ in pairs) / len(pairs)
    ymean = sum(y for _, y in pairs) / len(pairs)
    xx = sum((x-xmean)**2 for x, _ in pairs)
    yy = sum((y-ymean)**2 for _, y in pairs)
    if not xx or not yy:
        return None
    return sum((x-xmean)*(y-ymean) for x, y in pairs) / math.sqrt(xx*yy)


def compare(commands, received):
    # Match by position and lifecycle only, never by the pressure/tilt being tested.
    buckets = collections.defaultdict(collections.deque)
    for row in received:
        buckets[(row['x'], row['y'], phase(row['pointer_flags']))].append(row)
    missing, pairs, differences = [], [], collections.Counter()
    order = []
    for row in commands:
        queue = buckets[(row['x'], row['y'], row['phase'])]
        if not queue:
            missing.append(row['sequence'])
            continue
        actual = queue.popleft()
        order.append(actual['sequence'])
        if row['stroke'] == 1 and row['phase'] in ('down', 'move'):
            pairs.append((row['pressure'], actual['pressure']))
        for field in ('pressure', 'tilt_x', 'tilt_y', 'rotation', 'pen_flags'):
            if row[field] != actual[field]:
                differences[field] += 1
        if actual['pen_mask'] & 15 != 15:
            differences['pen_mask'] += 1
        # NEW/PRIMARY/CONFIDENCE and frame/device IDs are OS-owned. Compare lifecycle and buttons.
        relevant = 0x70000 | 0x8000 | 2 | 4 | 0x10 | 0x20
        if row['pointer_flags'] & relevant != actual['pointer_flags'] & relevant:
            differences['pointer_flags'] += 1
    ordered = order == sorted(order) and len(set(order)) == len(order)
    r = pearson(pairs)
    intervals = []
    for previous, current in zip(commands, commands[1:]):
        if previous['phase'] in ('down', 'move'):
            intervals.append(current['actual_ms'] - previous['actual_ms'])
    expected_pressure = sum(row['stroke'] == 1 and row['phase'] in ('down', 'move') for row in commands)
    return {
        'command_samples': len(commands), 'received_samples': len(received),
        'matched_samples': len(order), 'missing_sequences': missing,
        'unmatched_received_samples': sum(len(queue) for queue in buckets.values()),
        'received_order_preserved': ordered, 'field_mismatches': dict(differences),
        'pressure_pairs': len(pairs), 'pressure_expected': expected_pressure, 'pressure_pearson_r': r,
        'pressure_target_met': r is not None and r >= .95 and len(pairs) == expected_pressure and expected_pressure == 65,
        'all_command_fields_matched': not missing and not differences and ordered,
        'max_contact_interval_ms': max(intervals, default=None),
        'keepalive_target_met': bool(intervals) and all(0 < gap <= 50 for gap in intervals),
        'guard_trials': 'separate live test required', 'editor_compatibility': 'separate owner test required',
    }


def analyze(command_path, receiver_dir):
    command_path, receiver_dir = Path(command_path), Path(receiver_dir)
    manifest = json.loads(command_path.with_suffix('.session.json').read_text(encoding='utf-8'))
    receiver = json.loads((receiver_dir / 'receiver.json').read_text(encoding='utf-8'))
    target = json.loads((receiver_dir / 'target.json').read_text(encoding='utf-8'))
    if manifest.get('completed') is not True or receiver.get('recorder_healthy') is not True:
        raise ValueError('Incomplete injection or failed recorder; preserve receipts and diagnose')
    if target.get('source') != 'native_wm_pointer' or any(manifest.get(k) != target.get(k) for k in ('hwnd', 'pid', 'dpi')):
        raise ValueError('Sender/receiver target identity does not match')
    commands = load_csv(command_path, COMMAND)
    received_path = receiver_dir / 'received.csv'
    received = load_csv(received_path, RECEIVED)
    if len(commands) != manifest.get('samples') or len(received) != receiver.get('native_samples'):
        raise ValueError('Journal counts do not match completion receipts')
    if len(commands) != 472 or collections.Counter(row['stroke'] for row in commands) != {1: 68, 2: 68, 3: 68, 4: 68, 5: 68, 6: 64, 7: 68}:
        raise ValueError('Incomplete scripted sweep')
    if any(b['actual_ms'] <= a['actual_ms'] or b['due_ms'] <= a['due_ms'] for a, b in zip(commands, commands[1:])):
        raise ValueError('Non-monotonic injection journal')
    result = compare(commands, received)
    result['mode'] = manifest['mode']
    result['harness_acceptance'] = bool(manifest['mode'] == 'normal' and result['pressure_target_met']
        and result['all_command_fields_matched'] and result['keepalive_target_met'] and result['unmatched_received_samples'] == 0)
    result['input_sha256'] = {'commands': hashlib.sha256(command_path.read_bytes()).hexdigest(),
                              'received': hashlib.sha256(received_path.read_bytes()).hexdigest()}
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('commands', type=Path)
    parser.add_argument('receiver', type=Path)
    parser.add_argument('--out', type=Path, required=True)
    args = parser.parse_args()
    result = analyze(args.commands, args.receiver)
    with args.out.open('x', encoding='utf-8') as file:
        json.dump(result, file, indent=2, allow_nan=False)
        file.write('\n')
    print(json.dumps(result, indent=2, allow_nan=False))
    return 0 if result['harness_acceptance'] else 1


if __name__ == '__main__':
    try:
        sys.exit(main())
    except (ValueError, OSError, KeyError, TypeError) as error:
        print(f'Injection analysis: FAIL ({error})', file=sys.stderr)
        sys.exit(1)
