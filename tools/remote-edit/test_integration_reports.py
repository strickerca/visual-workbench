import copy,importlib.util,unittest
from pathlib import Path
spec=importlib.util.spec_from_file_location('integration_reports',Path(__file__).with_name('integration_reports.py'))
reports=importlib.util.module_from_spec(spec);spec.loader.exec_module(reports)
def fixture():
    rows=[]
    for i in range(3):
        cap=f'018bcfe5-6800-7000-8000-{i+1:012x}';owner=f'00000000-0000-4000-8000-{i+1:012x}'
        rows.append({'remote_render_scope':f'{1 if i<2 else 3},{cap},{i+1},018bcfe5-6800-7000-8000-{i+10:012x},1',
            'remote_render_owner':owner,'remote_render_ticket':str(i+1),'remote_render_frame':'1','remote_render_pts_us':str(100+i),
            'remote_render_codec':'c2.qti.hevc.decoder','remote_render_timing':f'{100+i*100},{150+i*100}',
            'remote_render_capabilities':'hardware=true,low_latency_advertised=false,requested=true,configure_accepted=true'})
    return rows
class RealReceiptParserTests(unittest.TestCase):
    def rejected(self,rows):
        with self.assertRaises(ValueError):reports.validate(rows)
    def test_exact_three_scoped_records_allow_unadvertised_requested_low_latency(self):
        self.assertEqual(3,reports.validate(fixture())['fresh_capture_scopes'])
        self.assertFalse(reports.validate(fixture())['latency_acceptance'])
    def test_duplicate_decoder_owner_refuses_even_with_different_tickets(self):
        r=fixture();r[1]['remote_render_owner']=r[0]['remote_render_owner'];self.rejected(r)
    def test_same_capture_nonce_refuses_even_with_new_generation(self):
        r=fixture();p=r[1]['remote_render_scope'].split(',');p[1]=r[0]['remote_render_scope'].split(',')[1];r[1]['remote_render_scope']=','.join(p);self.rejected(r)
    def test_bad_or_overflowing_pts_and_ticket_refuse(self):
        for key,value in [('remote_render_pts_us','0'),('remote_render_pts_us',str(1<<63)),('remote_render_ticket',str(1<<64))]:
            r=fixture();r[0][key]=value;self.rejected(r)
    def test_queue_or_reverse_callback_clock_is_not_render_record(self):
        r=fixture();r[0]['remote_render_timing']='150,100';self.rejected(r)
        r=fixture();r[0]['remote_render_timing']='0,100';self.rejected(r)
    def test_reconnect_must_change_epoch_after_background_preserves_epoch(self):
        r=fixture();r[2]['remote_render_scope']=r[2]['remote_render_scope'].replace('3,','1,',1);self.rejected(r)
    def test_extra_app_acceptance_claim_or_software_decoder_refuses(self):
        r=fixture();r[0]['applied_input']='7';self.rejected(r)
        r=fixture();r[0]['remote_render_capabilities']=r[0]['remote_render_capabilities'].replace('hardware=true','hardware=false');self.rejected(r)
    def test_failed_validation_retains_three_bounded_observations(self):
        rows=fixture();rows[1]['remote_render_timing']='250,200'
        result=reports.validation_report(rows)
        self.assertEqual('rejected',result['status']);self.assertFalse(result['records_validated'])
        self.assertEqual(10,result['reason_code']);self.assertEqual(3,result['observed_count'])
        self.assertEqual(rows,result['observed_records'])
    def test_success_keeps_exact_validator_and_marks_observations_validated(self):
        result=reports.validation_report(fixture())
        self.assertTrue(result['records_validated']);self.assertEqual(0,result['reason_code'])
        self.assertEqual(reports.validate(fixture()),result['validation'])
    def test_rejected_arbitrary_fields_are_withheld_and_extra_keys_never_escape(self):
        rows=fixture();rows[0]['remote_render_codec']='private/path secret';rows[0]['extra']='private'
        result=reports.validation_report(rows)
        self.assertEqual(2,result['reason_code']);self.assertFalse(result['records_validated'])
        self.assertEqual('[withheld]',result['observed_records'][0]['remote_render_codec'])
        self.assertNotIn('extra',result['observed_records'][0])
    def test_malformed_census_retention_is_bounded_without_admitting(self):
        result=reports.validation_report(fixture()*100)
        self.assertEqual(300,result['observed_count']);self.assertEqual(3,len(result['observed_records']))
        self.assertEqual(1,result['reason_code']);self.assertFalse(result['records_validated'])
    def test_unrecognized_exception_details_are_never_reported(self):
        original=reports.validate
        try:
            def fail(_):raise ValueError('private detail / arbitrary value')
            reports.validate=fail
            result=reports.validation_report(fixture())
            self.assertEqual(99,result['reason_code']);self.assertNotIn('private',str(result))
        finally:reports.validate=original
if __name__=='__main__':unittest.main()
