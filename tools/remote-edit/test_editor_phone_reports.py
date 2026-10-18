import copy
import unittest
from controller_input_reports import validate
from editor_phone_reports import report
from test_controller_input_reports import ControllerInputReportsTest


class EditorPhoneReportsTest(unittest.TestCase):
    def fixture(self):
        return ControllerInputReportsTest().fixture()

    def test_phone_ack_and_fade_never_claim_receiver_or_editor_effect(self):
        inputs, views, _, _ = self.fixture()
        value = report(inputs, views)
        self.assertEqual('passed', value['status'])
        self.assertTrue(value['phone_records_validated'])
        self.assertFalse(value['receiver_observed'])
        self.assertFalse(value['editor_effect'])
        self.assertFalse(value['phone_correlation']['receiver_observed'])

    def test_same_strict_scope_ack_and_ghost_requirements(self):
        for key, value in [('remote_input_sequences', '1,2'),
                           ('remote_input_echo', '2,1,0'),
                           ('remote_input_ticket', '5'),
                           ('remote_input_timing', '1000000000,1100000000,1110000000,1110000000,1500000000'),
                           ('remote_input_route', 'editor_effect=true')]:
            with self.subTest(key=key):
                inputs, views, _, _ = copy.deepcopy(self.fixture())
                inputs[0][key] = value
                self.assertEqual('rejected', report(inputs, views)['status'])

    def test_existing_harness_validator_still_requires_real_receiver(self):
        inputs, views, counts, events = self.fixture()
        for bad_counts, bad_events in [([0] * 6, []), ([1, 1, 1, 1, 0, 0], events)]:
            with self.subTest(counts=bad_counts):
                with self.assertRaises(ValueError):
                    validate(inputs, views, bad_counts, bad_events)
        self.assertEqual(3, validate(inputs, views, counts, events)['actual_local_admissions'])


if __name__ == '__main__':
    unittest.main()
