"""Phone-only correlation for the explicit retained owned-editor fixture.

This never fabricates a receiver journal or establishes an editor pixel effect.
The editor parent must separately prove the same retained document's pixels.
"""
import json
import sys
from controller_input_reports import read, validate_phone, rejection_reason


def report(inputs, views):
    result = {'schema': 1, 'operation': 'owned-editor-phone-correlation',
              'status': 'rejected', 'rejection_reason': 'unclassified',
              'phone_records_validated': False, 'receiver_observed': False,
              'editor_effect': False, 'physical_pen_fidelity': False,
              'latency_acceptance': False}
    try:
        result['phone_correlation'] = validate_phone(inputs, views)
        result.update(status='passed', rejection_reason='none', phone_records_validated=True)
    except (ValueError, TypeError, KeyError, OverflowError) as error:
        result['rejection_reason'] = rejection_reason(error)
    return result


def main(arguments):
    if len(arguments) != 2:
        return 2
    result = report(read(arguments[0], 65536), read(arguments[1], 65536))
    print(json.dumps(result, separators=(',', ':')))
    return 0 if result['status'] == 'passed' else 1


if __name__ == '__main__':
    sys.exit(main(sys.argv[1:]))
