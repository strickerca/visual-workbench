import copy,unittest
from controller_input_reports import validate,unique,ROUTE
class ControllerInputReportsTest(unittest.TestCase):
    def fixture(self):
        def ident(n):return f'01a10700-0000-7000-8000-{n:012x}'
        views=[]
        for i in range(3):
            views.append({'remote_render_scope':f'{1 if i<2 else 2},{ident(i+1)},1,{ident(10)},1','remote_render_owner':ident(i+20),'remote_render_ticket':str(5+i),'remote_render_frame':str(10+i),'remote_render_pts_us':str(100000+i*1000),'remote_render_codec':'c2.qti.hevc.decoder','remote_render_timing':f'{900000000+i*1000},{910000000+i*1000}','remote_render_capabilities':'hardware=true,low_latency_advertised=false,requested=true,configure_accepted=true'})
        inputs=[{'remote_input_binding':views[0]['remote_render_scope']+','+ident(40),'remote_input_sequences':'1,3','remote_input_owner':views[0]['remote_render_owner'],'remote_input_frame':'12','remote_input_ticket':'9','remote_input_pts_us':'120000','remote_input_timing':'1000000000,1100000000,1110000000,1110000000,1270000000','remote_input_echo':'3,0,0','remote_input_route':ROUTE}]
        events=[{'message':[0x246,0x245,0x247][i],'index':i,'count':1,'pointer_available':True,'pressure':512 if i<2 else 0,'pen_flags':0,'performance_count':1000+i,'pointer_flags':[0x12017,0x22016,0x42002][i],'pointer_id':1}for i in range(3)]
        return inputs,views,[1,1,1,0,0,0],events
    def refused(self,change):
        values=copy.deepcopy(self.fixture());change(*values)
        with self.assertRaises(ValueError):validate(*values)
    def test_exact_software_controller_scope_receiver_and_fade_correlation(self):
        result=validate(*self.fixture());self.assertEqual(3,result['actual_local_admissions']);self.assertTrue(result['text_correlation_only']);self.assertFalse(result['physical_pen_fidelity'])
    def test_foreign_input_scope_refused(self):self.refused(lambda a,b,c,d:a[0].update(remote_input_binding=b[2]['remote_render_scope']+',01a10700-0000-7000-8000-000000000028'))
    def test_old_decoder_owner_refused(self):self.refused(lambda a,b,c,d:a[0].update(remote_input_owner=b[1]['remote_render_owner']))
    def test_nonconsecutive_or_predicted_sequence_census_refused(self):self.refused(lambda a,b,c,d:a[0].update(remote_input_sequences='1,4'))
    def test_old_covering_frame_refused(self):self.refused(lambda a,b,c,d:a[0].update(remote_input_frame=b[0]['remote_render_frame']))
    def test_reused_native_ticket_refused(self):self.refused(lambda a,b,c,d:a[0].update(remote_input_ticket=b[0]['remote_render_ticket']))
    def test_old_media_pts_refused(self):self.refused(lambda a,b,c,d:a[0].update(remote_input_pts_us=b[0]['remote_render_pts_us']))
    def test_hard_expiry_is_not_ack_fade(self):self.refused(lambda a,b,c,d:a[0].update(remote_input_timing='1000000000,1100000000,1110000000,1110000000,1500000000'))
    def test_disappearance_before_actual_ack_fade_refused(self):self.refused(lambda a,b,c,d:a[0].update(remote_input_timing='1000000000,1100000000,1110000000,1110000000,1259999999'))
    def test_missing_echo_coverage_refused(self):self.refused(lambda a,b,c,d:a[0].update(remote_input_echo='2,1,0'))
    def test_missing_native_up_refused(self):self.refused(lambda a,b,c,d:c.__setitem__(2,0))
    def test_extra_mouse_category_refused(self):self.refused(lambda a,b,c,d:c.__setitem__(4,1))
    def test_complete_receiver_journal_census_required(self):self.refused(lambda a,b,c,d:d.pop())
    def test_native_pointer_info_required(self):self.refused(lambda a,b,c,d:d[1].update(pointer_available=False))
    def test_actual_prefix_order_required(self):self.refused(lambda a,b,c,d:d[0].update(count=2))
    def test_unavailable_receiver_qpc_stays_zero_and_creates_no_timestamp_authority(self):
        values=self.fixture()
        for event in values[3]:event['performance_count']=0
        result=validate(*values);self.assertFalse(result['receiver_qpc_available']);self.assertTrue(result['native_retained_ack_producer_required'])
    def test_actual_receiver_clock_is_optional_versioned_metadata_for_balanced(self):
        values=self.fixture()
        for i,event in enumerate(values[3]):event['received_qpc']={'schema':1,'counter':10000+i,'frequency':10000000}
        self.assertEqual(3,validate(*values)['actual_local_admissions'])
    def test_zero_receiver_clock_never_substitutes_pointer_performancecount(self):
        self.refused(lambda a,b,c,d:d[0].update(received_qpc={'schema':1,'counter':0,'frequency':10000000}))
    def test_duplicate_receipt_keys_refused(self):
        with self.assertRaises(ValueError):unique([('scope','a'),('scope','b')])
    def test_second_covering_frame_keeps_first_ack_fade_origin(self):
        values=self.fixture()
        values[0][0]['remote_input_timing']='1000000000,1200000000,1210000000,1110000000,1270000000'
        self.assertEqual(3,validate(*values)['actual_local_admissions'])
    def test_ack_origin_after_latest_callback_refused(self):
        self.refused(lambda a,b,c,d:a[0].update(remote_input_timing='1000000000,1100000000,1110000000,1120000000,1270000000'))
    def test_old_four_field_ambiguous_timing_refused(self):
        self.refused(lambda a,b,c,d:a[0].update(remote_input_timing='1000000000,1100000000,1110000000,1270000000'))
    def test_actual_noncontact_update_after_up_is_not_second_stroke(self):
        values=self.fixture();hover=copy.deepcopy(values[3][-1]);hover.update(index=1,message=0x245,count=2,pointer_flags=0x22002,performance_count=1003);values[3].append(hover);values[2][1]=2
        self.assertEqual(1,validate(*values)['post_up_noncontact_updates'])
    def test_pressure_zero_without_flags_is_not_hover_proof(self):
        self.refused(lambda a,b,c,d:d[1].pop('pointer_flags'))
    def test_post_up_contact_update_rejected(self):
        values=self.fixture();event=copy.deepcopy(values[3][1]);event.update(count=2,performance_count=1003,pressure=0);values[3].append(event);values[2][1]=2
        with self.assertRaises(ValueError):validate(*values)
    def test_foreign_pointer_identity_rejected(self):self.refused(lambda a,b,c,d:d[1].update(pointer_id=2))
    def test_pointer_flags_cannot_be_boolean(self):self.refused(lambda a,b,c,d:d[1].update(pointer_flags=True))
    def test_contact_down_requires_incontact_flag(self):self.refused(lambda a,b,c,d:d[0].update(pointer_flags=0x12003))
    def test_up_cannot_remain_incontact(self):self.refused(lambda a,b,c,d:d[2].update(pointer_flags=0x42006))
    def test_wrong_transition_flag_rejected(self):self.refused(lambda a,b,c,d:d[1].update(pointer_flags=0x42002))
    def test_cancelled_balanced_up_is_not_normal_completion(self):self.refused(lambda a,b,c,d:d[2].update(pointer_flags=0x4a002))
    def test_legacy_three_event_record_without_pointer_metadata_rejected(self):
        values=self.fixture()
        for event in values[3]:event.pop('pointer_flags');event.pop('pointer_id')
        with self.assertRaises(ValueError):validate(*values)
    def test_extra_down_cannot_follow_completed_contact(self):
        values=self.fixture();event=copy.deepcopy(values[3][0]);event.update(count=2,performance_count=1003);values[3].append(event);values[2][0]=2
        with self.assertRaises(ValueError):validate(*values)
    def test_open_handle_read_rejects_overlimit_when_directory_size_lags(self):
        import io,types
        from unittest.mock import patch
        from controller_input_reports import read
        class File:
            def is_file(self):return True
            def is_symlink(self):return False
            def stat(self):return types.SimpleNamespace(st_size=0)
            def open(self,mode):return io.BytesIO(b'x'*17)
        with patch('controller_input_reports.Path',return_value=File()):
            with self.assertRaises(ValueError):read('owned-journal',16,True)
    def test_delayed_mouse_contamination_is_not_removed_by_clock_gap(self):
        values=self.fixture();event={'message':0x201,'index':3,'count':1,'pointer_available':False,'pressure':None,'pen_flags':None,'performance_count':None,'pointer_flags':None,'pointer_id':None,'received_qpc':{'schema':1,'counter':2100000000,'frequency':10000000}};values[2][3]=1;values[3].append(event)
        with self.assertRaises(ValueError):validate(*values)
class ReceiverEvidenceTest(unittest.TestCase):
    def fixture(self):return ControllerInputReportsTest().fixture()
    def test_mouse_rejection_has_exact_reason_and_preserved_numeric_events(self):
        from controller_input_reports import validation_report
        a,b,c,d=self.fixture();c[3]=1;d.append({'message':0x201,'index':3,'count':1,'pointer_available':False,'pressure':None,'pen_flags':None,'performance_count':None,'pointer_flags':None,'pointer_id':None,'received_qpc':{'schema':1,'counter':99000,'frequency':10000000}})
        r=validation_report(a,b,c,d,b'closed-journal\n')
        self.assertEqual('balanced_native_pointer_receiver',r['rejection_reason']);self.assertFalse(r['records_validated']);self.assertEqual(3,r['receiver_evidence']['events'][-1]['index']);self.assertFalse(r['receiver_evidence']['acceptance'])
    def test_valid_contact_with_optional_actual_provenance_passes_existing_guards(self):
        from controller_input_reports import validation_report
        values=self.fixture()
        for row in values[3]:row['input_provenance']={'schema':1,'source_available':True,'device_type':2,'origin_id':4,'message_extra_info':'ff51570000000000'}
        r=validation_report(*values,raw=b'journal\n');self.assertTrue(r['records_validated']);self.assertFalse(r['receiver_evidence']['records_validated'])
    def test_unavailable_provenance_is_unknown_not_a_fabricated_device(self):
        from controller_input_reports import validate_provenance,event_observation
        e={'input_provenance':{'schema':1,'source_available':False,'device_type':None,'origin_id':None,'message_extra_info':'0000000000000000'}}
        validate_provenance(e);self.assertIsNone(event_observation(e,0)['input_provenance']['device_type'])
    def test_provenance_shape_cannot_admit_paths_or_false_query_results(self):
        from controller_input_reports import validation_report
        for extra in ('C:/private/data',True,'0'*17):
            values=self.fixture();values[3][0]['input_provenance']={'schema':1,'source_available':True,'device_type':2,'origin_id':4,'message_extra_info':extra}
            r=validation_report(*values);self.assertEqual('receiver_provenance_shape',r['rejection_reason']);self.assertIsNone(r['receiver_evidence']['events'][0]['input_provenance'])
    def test_unknown_fields_and_values_are_never_copied_to_output(self):
        import json
        from controller_input_reports import event_observation
        e={'message':'secret-path','pointer_available':'secret-serial','evil':'secret-QR','received_qpc':{'schema':1,'counter':'secret-address','frequency':0}}
        r=event_observation(e,0);self.assertNotIn('secret',json.dumps(r));self.assertEqual(1,r['unknown_field_count'])
    def test_first_and_last_records_are_retained_with_exact_omission_census(self):
        from controller_input_reports import receiver_evidence
        events=[{'index':i%6}for i in range(100)]
        r=receiver_evidence([0]*6,events,b'x\n');self.assertEqual(64,len(r['events']));self.assertEqual(36,r['omitted_event_count']);self.assertEqual([0,99],[r['events'][0]['ordinal'],r['events'][-1]['ordinal']])
    def test_unknown_exception_body_is_closed_unclassified(self):
        from controller_input_reports import rejection_reason
        self.assertEqual('unclassified',rejection_reason(ValueError('private arbitrary body')))
    def test_main_emits_evidence_even_if_input_json_is_invalid(self):
        import io,json
        from contextlib import redirect_stdout
        from unittest.mock import patch
        from controller_input_reports import main
        with patch('controller_input_reports.load_receiver',return_value=([1,1,1,0,0,0],self.fixture()[3],b'raw\n')),patch('controller_input_reports.read',side_effect=ValueError('private-body')):
            out=io.StringIO()
            with redirect_stdout(out):code=main(['input','views','counts','journal'])
        r=json.loads(out.getvalue());self.assertEqual(1,code);self.assertEqual(3,r['receiver_evidence']['event_count']);self.assertNotIn('private-body',out.getvalue())
    def test_receiver_only_mode_never_validates_input(self):
        import io,json
        from contextlib import redirect_stdout
        from unittest.mock import patch
        from controller_input_reports import main
        with patch('controller_input_reports.load_receiver',return_value=([1,1,1,0,0,0],self.fixture()[3],b'raw\n')):
            out=io.StringIO()
            with redirect_stdout(out):code=main(['observe-receiver','counts','journal'])
        r=json.loads(out.getvalue());self.assertEqual(0,code);self.assertEqual('observed',r['status']);self.assertFalse(r['records_validated'])
    def test_snapshot_read_failure_retains_explicit_incomplete_state(self):
        import io,json
        from contextlib import redirect_stdout
        from unittest.mock import patch
        from controller_input_reports import main
        with patch('controller_input_reports.load_receiver',side_effect=OSError('C:/private/journal')):
            out=io.StringIO()
            with redirect_stdout(out):code=main(['observe-receiver','counts','journal'])
        self.assertEqual(1,code);r=json.loads(out.getvalue());self.assertFalse(r['receiver_evidence']['snapshot_complete']);self.assertNotIn('private',out.getvalue())
    def test_success_result_preserves_strict_validation_and_observation_separation(self):
        from controller_input_reports import validation_report
        r=validation_report(*self.fixture(),raw=b'raw\n');self.assertEqual('passed',r['status']);self.assertEqual('none',r['rejection_reason']);self.assertEqual(3,r['validation']['actual_local_admissions']);self.assertFalse(r['receiver_evidence']['acceptance'])
if __name__=='__main__':unittest.main()
