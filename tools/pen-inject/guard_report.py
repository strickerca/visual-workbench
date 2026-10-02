"""Fail closed unless all 100 live trials and the two native recorders completed."""
import csv
import json
from pathlib import Path
import sys


def report(root):
    root = Path(root)
    with (root/'commands.csv').open(encoding='utf-8', newline='') as file:
        reader = csv.DictReader(file)
        if reader.fieldnames != ['trial', 'mutation', 'baseline_sent', 'changed_attempt_blocked', 'resume_attempt_blocked']:
            raise ValueError('Unexpected trial schema')
        rows = list(reader)
    if len(rows) != 100:
        raise ValueError('Exactly 100 completed live trials required')
    for i, row in enumerate(rows):
        if row != dict(trial=str(i), mutation=['move', 'resize', 'minimize', 'focus'][i % 4],
                       baseline_sent='true', changed_attempt_blocked='true', resume_attempt_blocked='true'):
            raise ValueError('Incomplete or failed guard trial')
    records = {}
    for role in ('target', 'sink'):
        records[role] = json.loads((root/role/'receiver.json').read_text())
        if records[role].get('recorder_healthy') is not True:
            raise ValueError('A native recorder failed')
        with (root/role/'received.csv').open(newline='') as file:
            samples = list(csv.DictReader(file))
        if len(samples) != records[role]['native_samples']:
            raise ValueError('Native receipt/CSV count mismatch')
        if role == 'target' and sum(int(row['pointer_flags']) & 0x10000 != 0 for row in samples) != 100:
            raise ValueError('Missing native baseline DOWN evidence for one or more trials')
    stray = records['sink']['native_samples']
    return {'live_trials': 100, 'mutations': {'move': 25, 'resize': 25, 'minimize': 25, 'focus': 25},
            'guard_rejections': 100, 'latched_rejections': 100, 'sink_stray_samples': stray,
            'observed_sink_target_met': stray == 0,
            'scope': 'Only the selected native target/sink are observed; this does not prove zero input to every other desktop window.'}


if __name__ == '__main__':
    try:
        root = Path(sys.argv[1])
        result = report(root)
        with (root/'guard-comparison.json').open('x', encoding='utf-8') as file:
            json.dump(result, file, indent=2)
            file.write('\n')
        print(json.dumps(result, indent=2))
        sys.exit(0 if result['observed_sink_target_met'] else 1)
    except (ValueError, OSError, KeyError, IndexError, TypeError) as error:
        print(f'Guard analysis: FAIL ({error})', file=sys.stderr)
        sys.exit(1)
