import copy,json,unittest
from controller_lifecycle_reports import validate,ready,ready_if_delivered,ROUTE
import test_controller_input_reports as balanced

class HeldLifecycleReportsTest(unittest.TestCase):
    def fixture(self,scenario='pause-held'):
        _,views,counts,events=balanced.ControllerInputReportsTest().fixture()
        if scenario=='disconnect-held':
            for i,v in enumerate(views):v['remote_render_scope']=str(i+1)+v['remote_render_scope'][1:]
        for i,e in enumerate(events):
            e['performance_count']=0
            e['received_qpc']={'schema':1,'counter':[100,110,130][i],'frequency':10000000}
        pack=lambda values:(''.join(json.dumps(e)+'\n' for e in values)).encode()
        prefix=ready(pack(events[:2]),'a'*32,scenario)
        trigger={'schema':1,'run_id':'a'*32,'scenario':scenario,'counter':120,'frequency':10000000,'kind':'phone_carrier_abort_gate' if scenario=='disconnect-held' else 'phone_lifecycle_gate'}
        records=[{'remote_lifecycle_scenario':scenario,'remote_lifecycle_binding':views[0]['remote_render_scope']+',01a10700-0000-7000-8000-000000000028','remote_lifecycle_sequences':'1,2','remote_lifecycle_phone_up_admitted':'false','remote_lifecycle_native_prefix_sha256':prefix['sha256'],'remote_lifecycle_native_prefix_bytes':str(prefix['bytes']),'remote_lifecycle_stale_tail_refused':'true','remote_lifecycle_route':ROUTE,'remote_lifecycle_phone_settled':'true','remote_lifecycle_return_granted':'false'}]
        parts=records[0]['remote_lifecycle_binding'].split(',')
        witness={'schema':1,'run_id':'a'*32,'scenario':scenario,'binding':{'scope':{'connection_epoch':int(parts[0]),'capture_session_id':parts[1],'source_generation':int(parts[2]),'target_token':parts[3],'geometry_revision':int(parts[4])},'input_session_id':parts[5]},'cause':{'pause-held':'owner_pause','background-held':'background','disconnect-held':'connection_retired'}[scenario],'counter':125,'frequency':10000000}
        witness['pen_release']={'binding':copy.deepcopy(witness['binding']),'held_before_release':True,'release_started_qpc':126,'release_completed_qpc':129,'qpc_frequency':10000000}
        return records,views,counts,pack(events),prefix,trigger,witness
    def refused(self,change):
        v=copy.deepcopy(self.fixture());change(*v[:6])
        with self.assertRaises((ValueError,KeyError)):validate(*v)
    def test_pause_actual_native_release_after_trigger(self):
        self.assertTrue(validate(*self.fixture())['native_safety_release_after_trigger'])
    def test_background_actual_native_release_after_trigger(self):
        self.assertFalse(validate(*self.fixture('background-held'))['phone_up_admitted'])
    def test_disconnect_requires_actual_loss_and_later_reconnect_epochs(self):
        self.assertFalse(validate(*self.fixture('disconnect-held'))['automatic_regrant'])
    def test_phone_up_not_accepted_as_safety_release(self):self.refused(lambda r,v,c,j,p,t:r[0].update(remote_lifecycle_phone_up_admitted='true'))
    def test_lost_stale_tail_proof_refused(self):self.refused(lambda r,v,c,j,p,t:r[0].update(remote_lifecycle_stale_tail_refused='false'))
    def test_pending_phone_refused(self):self.refused(lambda r,v,c,j,p,t:r[0].update(remote_lifecycle_phone_settled='false'))
    def test_automatic_regrant_refused(self):self.refused(lambda r,v,c,j,p,t:r[0].update(remote_lifecycle_return_granted='true'))
    def test_release_at_trigger_refused(self):self.refused(lambda r,v,c,j,p,t:t.update(counter=130))
    def test_watchdog_release_before_trigger_refused(self):self.refused(lambda r,v,c,j,p,t:t.update(counter=140))
    def test_trigger_before_native_move_refused(self):self.refused(lambda r,v,c,j,p,t:t.update(counter=109))
    def test_mismatched_clock_frequency_refused(self):self.refused(lambda r,v,c,j,p,t:t.update(frequency=1000))
    def test_foreign_run_refused(self):self.refused(lambda r,v,c,j,p,t:t.update(run_id='b'*32))
    def test_wrong_fault_kind_refused(self):self.refused(lambda r,v,c,j,p,t:t.update(kind='phone_carrier_abort_gate'))
    def test_changed_prefix_digest_refused(self):self.refused(lambda r,v,c,j,p,t:r[0].update(remote_lifecycle_native_prefix_sha256='0'*64))
    def test_no_native_move_prefix_refused(self):
        v=self.fixture();line=v[3].splitlines(keepends=True)[0]
        with self.assertRaises(ValueError):ready(line,'a'*32,'pause-held')
    def test_already_released_prefix_refused(self):
        v=self.fixture()
        with self.assertRaises(ValueError):ready(v[3],'a'*32,'pause-held')
    def test_balanced_is_not_a_held_scenario(self):
        v=self.fixture()
        with self.assertRaises(ValueError):ready(v[3],'a'*32,'balanced')
    def test_bool_cannot_substitute_clock_schema(self):self.refused(lambda r,v,c,j,p,t:t.update(schema=True))
    def test_bool_cannot_substitute_receiver_count(self):self.refused(lambda r,v,c,j,p,t:c.__setitem__(0,True))
    def test_pointer_info_zero_remains_unknown_with_actual_receive_clock(self):
        result=validate(*self.fixture());self.assertFalse(result['physical_pen_fidelity']);self.assertFalse(result['latency_acceptance'])
    def test_empty_or_delayed_move_is_pending_not_invalid(self):
        v=self.fixture();self.assertIsNone(ready_if_delivered(b'','a'*32,'pause-held'))
        down=v[3].splitlines(keepends=True)[0];self.assertIsNone(ready_if_delivered(down,'a'*32,'pause-held'))
        self.assertEqual(v[4],ready_if_delivered(v[3][:v[4]['bytes']],'a'*32,'pause-held'))
    def test_release_while_waiting_prefix_refuses_instead_of_retrying(self):
        with self.assertRaises(ValueError):ready_if_delivered(self.fixture()[3],'a'*32,'pause-held')
    def test_watchdog_after_gate_before_actual_phone_cause_is_refused(self):
        v=list(self.fixture());v[6]['counter']=135
        with self.assertRaises(ValueError):validate(*v)
    def test_wrong_native_release_cause_refused(self):
        v=list(self.fixture());v[6]['cause']='input_expired'
        with self.assertRaises(ValueError):validate(*v)
    def test_foreign_native_release_binding_refused(self):
        v=list(self.fixture());v[6]['binding']['input_session_id']='01a10700-0000-7000-8000-000000000099'
        with self.assertRaises(ValueError):validate(*v)
    def test_missing_native_cause_witness_refused(self):
        v=list(self.fixture());v[6]=None
        with self.assertRaises(ValueError):validate(*v)
    def test_device_released_before_cause_with_late_receiver_up_is_refused(self):
        v=list(self.fixture());v[6]['pen_release'].update(release_started_qpc=124,release_completed_qpc=127)
        with self.assertRaises(ValueError):validate(*v)
    def test_already_neutral_device_is_not_a_held_release(self):
        v=list(self.fixture());v[6]['pen_release']['held_before_release']=False
        with self.assertRaises(ValueError):validate(*v)
    def test_foreign_helper_release_cannot_complete_current_witness(self):
        v=list(self.fixture());v[6]['pen_release']['binding']['input_session_id']='01a10700-0000-7000-8000-000000000099'
        with self.assertRaises(ValueError):validate(*v)
    def test_device_release_clock_must_complete_and_match_frequency(self):
        for change in ({'release_completed_qpc':125},{'qpc_frequency':0},{'release_started_qpc':0}):
            v=list(self.fixture());v[6]['pen_release'].update(change)
            with self.assertRaises(ValueError):validate(*v)
    def test_missing_helper_terminal_proof_never_passes(self):
        v=list(self.fixture());v[6]['pen_release']=None
        with self.assertRaises(ValueError):validate(*v)
    def test_held_release_may_have_proven_noncontact_update_after_up(self):
        values=list(self.fixture());events=[json.loads(line)for line in values[3].splitlines()];hover=copy.deepcopy(events[-1]);hover.update(index=1,message=0x245,count=2,pointer_flags=0x22002);hover['received_qpc']['counter']=131;events.append(hover);values[2][1]=2;values[3]=''.join(json.dumps(e)+'\n'for e in events).encode()
        self.assertTrue(validate(*values)['native_safety_release_after_trigger'])
    def test_held_prefix_noncontact_move_cannot_prove_held_input(self):
        values=self.fixture();events=[json.loads(line)for line in values[3].splitlines()];events[1]['pointer_flags']=0x22002
        with self.assertRaises(ValueError):ready(''.join(json.dumps(e)+'\n'for e in events[:2]).encode(),'a'*32,'pause-held')
    def test_actual_nine_field_failure_retains_exact_closed_reason(self):
        from controller_lifecycle_reports import validation_report
        v=list(self.fixture());del v[0][0]['remote_lifecycle_native_prefix_sha256']
        result=validation_report(*v)
        self.assertEqual('lifecycle_shape',result['rejection_reason']);self.assertFalse(result['records_validated'])
        self.assertEqual(v[4]['sha256'],result['held_evidence']['prefix']['sha256'])
        self.assertEqual(v[5]['counter'],result['held_evidence']['trigger']['counter'])
        self.assertEqual(v[6]['pen_release']['release_started_qpc'],result['held_evidence']['witness']['pen_release']['release_started_qpc'])
    def test_wrong_cause_failure_keeps_observations_not_acceptance(self):
        from controller_lifecycle_reports import validation_report
        v=list(self.fixture());v[6]['cause']='input_expired';result=validation_report(*v)
        self.assertEqual('actual_requested_host_cause',result['rejection_reason']);self.assertFalse(result['held_evidence']['acceptance'])
    def test_unknown_exception_does_not_publish_private_body(self):
        from controller_lifecycle_reports import closed_reason
        self.assertEqual('unclassified',closed_reason(ValueError('C:/private/path')))
    def test_projection_redacts_invalid_known_values_and_unknown_fields(self):
        from controller_lifecycle_reports import observations
        v=list(self.fixture());v[0][0]['remote_lifecycle_native_prefix_sha256']='https://private.invalid';v[4]['run_id']='private';v[5]['counter']=True;v[6]['cause']='private';v[6]['secret']='private'
        result=observations(v[0],v[4],v[5],v[6]);text=json.dumps(result)
        self.assertNotIn('private',text);self.assertNotIn('secret',text);self.assertGreaterEqual(result['withheld_fields'],5)
    def test_projection_bounds_records_and_never_claims_validation(self):
        from controller_lifecycle_reports import observations
        v=self.fixture();result=observations(v[0]*9,v[4],v[5],v[6]);self.assertEqual(4,len(result['records']));self.assertEqual(5,result['omitted_count']);self.assertFalse(result['records_validated'])
    def test_complete_valid_report_still_requires_strict_validator(self):
        from controller_lifecycle_reports import validation_report
        result=validation_report(*self.fixture());self.assertEqual('passed',result['status']);self.assertTrue(result['records_validated']);self.assertFalse(result['held_evidence']['records_validated'])
    def test_malformed_ready_cli_is_closed_failure(self):
        from controller_lifecycle_reports import ready_main
        from unittest.mock import patch
        from contextlib import redirect_stdout
        import io,sys
        output=io.StringIO()
        with patch.object(sys,'argv',['report.py','ready']),redirect_stdout(output):code=ready_main()
        self.assertEqual(1,code);self.assertEqual('arguments',json.loads(output.getvalue())['rejection_reason'])
if __name__=='__main__':unittest.main()
