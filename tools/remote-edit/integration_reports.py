"""Pure bounded parser for real-path text receipts; never executes a device/app."""
import json,re,sys
from pathlib import Path
UUID=re.compile(r'[a-f0-9]{8}-[a-f0-9]{4}-[47][a-f0-9]{3}-[89ab][a-f0-9]{3}-[a-f0-9]{12}\Z')
KEYS={'remote_render_scope','remote_render_owner','remote_render_ticket','remote_render_frame','remote_render_pts_us','remote_render_codec','remote_render_timing','remote_render_capabilities'}
def require(value,reason):
    if not value:raise ValueError(reason)
def integer(value,maximum=(1<<64)-1):
    require(isinstance(value,str) and re.fullmatch(r'[1-9][0-9]{0,19}',value),'integer_shape')
    n=int(value);require(0<n<=maximum,'integer_bound');return n
def validate(records,scenario="balanced"):
    require(scenario in {"balanced","pause-held","background-held","disconnect-held"},"scenario")
    require(isinstance(records,list) and len(records)==3,'render_census')
    scopes=[];owners=[];captures=[];last_callback=0
    for r in records:
        require(isinstance(r,dict) and set(r)==KEYS and all(isinstance(v,str) and len(v)<=2048 for v in r.values()),'record_shape')
        parts=r['remote_render_scope'].split(',');require(len(parts)==5,'scope_shape')
        epoch=integer(parts[0]);require(UUID.fullmatch(parts[1]) and UUID.fullmatch(parts[3]),'scope_uuid')
        integer(parts[2]);integer(parts[4],(1<<32)-1)
        require(UUID.fullmatch(r['remote_render_owner']),'owner_uuid')
        integer(r['remote_render_ticket']);integer(r['remote_render_frame']);integer(r['remote_render_pts_us'],(1<<63)-1)
        require(re.fullmatch(r'[A-Za-z0-9_.-]{1,128}',r['remote_render_codec']),'codec_name')
        times=r['remote_render_timing'].split(',');require(len(times)==2,'timing_shape')
        rendered=integer(times[0],(1<<63)-1);callback=integer(times[1],(1<<63)-1)
        require(rendered<=callback and callback>last_callback,'local_callback_order');last_callback=callback
        require(re.fullmatch(r'hardware=true,low_latency_advertised=(?:true|false),requested=(?:true|false),configure_accepted=(?:true|false)',r['remote_render_capabilities']),'capability_shape')
        scopes.append((epoch,*parts[1:]));owners.append(r['remote_render_owner']);captures.append(parts[1])
    require(len(set(scopes))==3 and len(set(owners))==3 and len(set(captures))==3,'fresh_owner_capture_scope')
    if scenario=='disconnect-held':require(len({s[0] for s in scopes})==3,'carrier_loss_and_reconnect_epochs')
    else:require(scopes[0][0]==scopes[1][0] and scopes[1][0]!=scopes[2][0],'background_and_reconnect_epochs')
    # This parser establishes shape/correlation only. Actual native-ticket
    # authority is established by the source-bound controller producer before
    # it publishes the read-only observation, and by the real-path case.
    return {'schema':1,'render_records':3,'fresh_decoder_owners':3,'fresh_capture_scopes':3,'text_correlation_only':True,'physical_pen_fidelity':False,'editor_effect':False,'latency_acceptance':False}
# Closed numeric reasons keep failed validation actionable without printing an
# arbitrary exception, path, environment, endpoint, or rejected field value.
REASONS={name:index+1 for index,name in enumerate((
    'render_census','record_shape','scope_shape','scope_uuid','owner_uuid',
    'integer_shape','integer_bound','codec_name','timing_shape',
    'local_callback_order','capability_shape','fresh_owner_capture_scope',
    'background_and_reconnect_epochs','arguments','receipt_file'))}
EVIDENCE_PATTERNS={
    'remote_render_scope':r'[0-9]{1,20},[a-f0-9-]{36},[0-9]{1,20},[a-f0-9-]{36},[0-9]{1,10}',
    'remote_render_owner':r'[a-f0-9-]{36}',
    'remote_render_ticket':r'[0-9]{1,20}',
    'remote_render_frame':r'[0-9]{1,20}',
    'remote_render_pts_us':r'-?[0-9]{1,20}',
    'remote_render_codec':r'[A-Za-z0-9_.-]{1,128}',
    'remote_render_timing':r'-?[0-9]{1,20},-?[0-9]{1,20}',
    'remote_render_capabilities':r'hardware=(?:true|false),low_latency_advertised=(?:true|false),requested=(?:true|false),configure_accepted=(?:true|false)',
}
def bounded_evidence(records):
    # This is observation retention, not admission. Invalid values are withheld;
    # exact validation below still sees the original records and fails closed.
    if not isinstance(records,list):return []
    return [{key:(row.get(key) if isinstance(row,dict) and isinstance(row.get(key),str)
                  and re.fullmatch(pattern,row[key]) else '[withheld]')
             for key,pattern in EVIDENCE_PATTERNS.items()} for row in records[:3]]
def validation_report(records,scenario="balanced"):
    result={'schema':2,'status':'rejected','reason_code':99,
            'observed_count':len(records) if isinstance(records,list) else 0,
            'observed_records':bounded_evidence(records),'records_validated':False}
    try:
        result['validation']=validate(records,scenario)
    except (ValueError,TypeError,KeyError) as error:
        result['reason_code']=REASONS.get(str(error),99)
    else:
        result.update(status='passed',reason_code=0,records_validated=True)
    return result

def main(arguments):
    try:
        require(len(arguments) in (1,2),'arguments');p=Path(arguments[0]);require(p.is_file() and not p.is_symlink() and p.stat().st_size<=65536,'receipt_file')
        result=validation_report(json.loads(p.read_text(encoding='utf-8-sig')),arguments[1] if len(arguments)==2 else 'balanced')
    except (ValueError,OSError,TypeError,KeyError) as error:
        result={'schema':2,'status':'rejected','reason_code':REASONS.get(str(error),99),
                'observed_count':0,'observed_records':[],'records_validated':False}
    print(json.dumps(result,separators=(',',':')))
    return 0 if result['records_validated'] else 1
if __name__=='__main__':
    sys.exit(main(sys.argv[1:]))
