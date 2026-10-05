"""Closed held-contact causal receipt parser. No input, device or process effects."""
import hashlib,json,re,sys,time
from pathlib import Path
from integration_reports import validate as views_validate,require,integer,UUID
from controller_input_reports import EVENT_KEYS,uint,unique,read,receiver_event_shape,validate_provenance,pointer_stroke
SCENARIOS={'pause-held','background-held','disconnect-held'}
KEYS={'remote_lifecycle_scenario','remote_lifecycle_binding','remote_lifecycle_sequences','remote_lifecycle_phone_up_admitted','remote_lifecycle_native_prefix_sha256','remote_lifecycle_native_prefix_bytes','remote_lifecycle_stale_tail_refused','remote_lifecycle_route','remote_lifecycle_phone_settled','remote_lifecycle_return_granted'}
ROUTE='surface_touch,software_generated=true,host_safety_release=true,physical_pen_fidelity=false'
def journal(raw):
    require(isinstance(raw,bytes) and 0<len(raw)<=262144 and raw.endswith(b'\n'),'complete_journal')
    values=[json.loads(line,object_pairs_hook=unique)for line in raw.decode('utf-8').splitlines()]
    require(1<=len(values)<=20000,'journal_bound');counts=[0]*6;last=0;frequency=None
    for e in values:
        require(receiver_event_shape(e,required_clock=True),'event_shape');validate_provenance(e)
        i=uint(e['index'],5);counts[i]+=1
        require(uint(e['count'],20000)==counts[i] and i<3 and uint(e['message'],(1<<32)-1)==[0x246,0x245,0x247][i],'pointer_order')
        require(e['pointer_available'] is True and uint(e['pen_flags'],7)==0,'pointer_info');uint(e['pressure'],1024);uint(e['performance_count'],(1<<64)-1)
        c=e['received_qpc'];require(isinstance(c,dict) and set(c)=={'schema','counter','frequency'} and uint(c['schema'],1)==1,'clock_shape')
        q=uint(c['counter'],(1<<63)-1);f=uint(c['frequency'],(1<<63)-1)
        require(q>0 and q>=last and f>0 and (frequency is None or f==frequency),'actual_receive_clock');last=q;frequency=f
    pointer_stroke(values,complete=counts[2]>0,allow_cancelled_up=True)
    return values,counts,frequency

def ready(raw,run,scenario):
    require(re.fullmatch('[a-f0-9]{32}',run) and scenario in SCENARIOS,'scenario_binding')
    events,counts,freq=journal(raw)
    require(counts[0]==1 and counts[1]>=1 and counts[2:]==[0,0,0,0] and events[0]['index']==0,'down_move_no_up_before_trigger')
    return {'schema':1,'run_id':run,'scenario':scenario,'bytes':len(raw),'sha256':hashlib.sha256(raw).hexdigest(),'counts':counts,'frequency':freq,'last_received_qpc':events[-1]['received_qpc']['counter']}

def ready_if_delivered(raw,run,scenario):
    if not raw:return None
    events,counts,_=journal(raw)
    require(counts[0]<=1 and counts[2:]==[0,0,0,0] and events[0]['index']==0,'held_prefix_already_invalid_or_released')
    return ready(raw,run,scenario) if counts[0]==1 and counts[1]>=1 else None

def validate(records,views,counts,raw,prefix,trigger,witness):
    require(isinstance(records,list) and len(records)==1,'lifecycle_census');r=records[0]
    require(isinstance(r,dict) and set(r)==KEYS and all(isinstance(v,str) and len(v)<=2048 for v in r.values()),'lifecycle_shape')
    scenario=r['remote_lifecycle_scenario'];require(scenario in SCENARIOS,'scenario');views_validate(views,scenario)
    binding=r['remote_lifecycle_binding'].split(',');require(len(binding)==6 and binding[:5]==views[0]['remote_render_scope'].split(',') and UUID.fullmatch(binding[5]),'initial_grant_binding')
    require(r['remote_lifecycle_sequences']=='1,2' and r['remote_lifecycle_phone_up_admitted']=='false','no_phone_up')
    require(r['remote_lifecycle_stale_tail_refused']=='true' and r['remote_lifecycle_phone_settled']=='true' and r['remote_lifecycle_return_granted']=='false' and r['remote_lifecycle_route']==ROUTE,'retirement_and_no_regrant')
    n=integer(r['remote_lifecycle_native_prefix_bytes'],262144)
    require(prefix==ready(raw[:n],prefix['run_id'],scenario) and prefix['sha256']==r['remote_lifecycle_native_prefix_sha256'],'unchanged_native_held_prefix')
    require(isinstance(trigger,dict) and set(trigger)=={'schema','run_id','scenario','counter','frequency','kind'},'trigger_shape')
    require(uint(trigger['schema'],1)==1 and trigger['run_id']==prefix['run_id'] and trigger['scenario']==scenario and trigger['kind']==('phone_carrier_abort_gate' if scenario=='disconnect-held' else 'phone_lifecycle_gate'),'trigger_binding')
    counter=uint(trigger['counter'],(1<<63)-1);require(counter>prefix['last_received_qpc'] and uint(trigger['frequency'],(1<<63)-1)==prefix['frequency'],'same_host_qpc_clock')
    require(isinstance(counts,list) and len(counts)==6,'receiver_counts');counts=[uint(v,20000)for v in counts]
    require(isinstance(witness,dict) and set(witness)=={'schema','run_id','scenario','binding','cause','counter','frequency','pen_release'},'actual_host_cause_shape')
    require(uint(witness['schema'],1)==1 and witness['run_id']==prefix['run_id'] and witness['scenario']==scenario,'actual_host_cause_run')
    expected_cause={'pause-held':'owner_pause','background-held':'background','disconnect-held':'connection_retired'}[scenario]
    require(witness['cause']==expected_cause,'actual_requested_host_cause')
    b=witness['binding'];require(isinstance(b,dict) and set(b)=={'scope','input_session_id'},'cause_binding_shape')
    scope=b['scope'];require(isinstance(scope,dict) and set(scope)=={'connection_epoch','capture_session_id','source_generation','target_token','geometry_revision'},'cause_scope_shape')
    actual_binding=[str(uint(scope['connection_epoch'],(1<<64)-1)),scope['capture_session_id'],str(uint(scope['source_generation'],(1<<64)-1)),scope['target_token'],str(uint(scope['geometry_revision'],(1<<32)-1)),b['input_session_id']]
    require(actual_binding==binding,'actual_host_cause_binding')
    cause_counter=uint(witness['counter'],(1<<63)-1)
    require(cause_counter>counter and uint(witness['frequency'],(1<<63)-1)==prefix['frequency'],'actual_cause_after_gate_same_clock')
    release=witness['pen_release'];require(isinstance(release,dict) and set(release)=={'binding','held_before_release','release_started_qpc','release_completed_qpc','qpc_frequency'},'native_release_shape')
    require(json.dumps(release['binding'],sort_keys=True)==json.dumps(b,sort_keys=True) and release['held_before_release'] is True,'exact_native_release_held_binding')
    release_started=uint(release['release_started_qpc'],(1<<63)-1);release_completed=uint(release['release_completed_qpc'],(1<<63)-1)
    require(release_started>cause_counter and release_completed>=release_started and uint(release['qpc_frequency'],(1<<63)-1)==prefix['frequency'],'actual_device_release_after_requested_cause')
    events,actual,freq=journal(raw);require(actual==counts and actual[0]==1 and actual[1]>=1 and actual[2:]==[1,0,0,0],'actual_native_safety_release')
    releases=[e for e in events if e['index']==2];require(len(releases)==1 and releases[0]['received_qpc']['counter']>release_started,'release_strictly_after_trigger')
    return {'schema':1,'scenario':scenario,'phone_admissions':2,'phone_up_admitted':False,'native_safety_release_after_trigger':True,'native_safety_release_after_actual_host_cause':True,'native_device_destruction_after_actual_host_cause':True,'stale_tail_refused':True,'automatic_regrant':False,'text_correlation_only':True,'physical_pen_fidelity':False,'latency_acceptance':False}

# Closed projections are observations, never input/retirement authority.
from controller_input_reports import REASONS as INPUT_REASONS
REASONS=INPUT_REASONS|set('complete_journal journal_bound event_shape pointer_order pointer_info clock_shape actual_receive_clock scenario_binding down_move_no_up_before_trigger held_prefix_already_invalid_or_released lifecycle_census lifecycle_shape scenario initial_grant_binding no_phone_up retirement_and_no_regrant unchanged_native_held_prefix trigger_shape trigger_binding same_host_qpc_clock receiver_counts actual_host_cause_shape actual_host_cause_run actual_requested_host_cause cause_binding_shape cause_scope_shape actual_host_cause_binding actual_cause_after_gate_same_clock native_release_shape exact_native_release_held_binding actual_device_release_after_requested_cause actual_native_safety_release release_strictly_after_trigger journal_file journal_file_bytes actual_native_prefix_deadline fresh_ready_receipt'.split())
def closed_reason(error):
    value=error.args[0] if len(error.args)==1 and isinstance(error.args[0],str) else None
    return value if value in REASONS else 'unclassified'

def observations(records,prefix,trigger,witness):
    withheld=0
    def scalar(value,kind):
        nonlocal withheld
        ok=False
        if kind=='uint':ok=type(value) is int and 0<=value<(1<<64)
        elif kind=='counts':ok=isinstance(value,list) and len(value)==6 and all(type(x)is int and 0<=x<=20000 for x in value)
        elif kind=='bool':ok=type(value) is bool
        elif kind=='uuid':ok=isinstance(value,str) and UUID.fullmatch(value)
        elif kind=='run':ok=isinstance(value,str) and re.fullmatch('[a-f0-9]{32}',value)
        elif kind=='sha':ok=isinstance(value,str) and re.fullmatch('[a-f0-9]{64}',value)
        elif isinstance(kind,set):ok=isinstance(value,str) and value in kind
        elif isinstance(kind,str):ok=isinstance(value,str) and re.fullmatch(kind,value)
        if ok:return value
        withheld+=1;return None
    def obj(value,shape):
        nonlocal withheld
        if not isinstance(value,dict):
            if value is not None:withheld+=1
            return None
        withheld+=len(set(value)-set(shape));out={}
        for key,kind in shape.items():
            if key not in value:continue
            out[key]=obj(value[key],kind) if isinstance(kind,dict) else scalar(value[key],kind)
        return out
    scope={'connection_epoch':'uint','capture_session_id':'uuid','source_generation':'uint','target_token':'uuid','geometry_revision':'uint'}
    binding={'scope':scope,'input_session_id':'uuid'}
    clocks={'counter':'uint','frequency':'uint'}
    identity={'schema':'uint','run_id':'run','scenario':SCENARIOS}
    phone={k:('true|false') for k in KEYS}
    phone.update(remote_lifecycle_scenario=SCENARIOS,remote_lifecycle_binding=r'[0-9]{1,20},[a-f0-9-]{36},[0-9]{1,20},[a-f0-9-]{36},[0-9]{1,10},[a-f0-9-]{36}',remote_lifecycle_sequences='[0-9]{1,20},[0-9]{1,20}',remote_lifecycle_native_prefix_sha256='sha',remote_lifecycle_native_prefix_bytes='[0-9]{1,6}',remote_lifecycle_route={ROUTE})
    rows=records if isinstance(records,list) else []
    result={'schema':1,'records':[obj(x,phone)for x in rows[:4]],'observed_count':len(rows),'omitted_count':max(0,len(rows)-4),'prefix':obj(prefix,dict(identity,bytes='uint',sha256='sha',frequency='uint',last_received_qpc='uint',counts='counts')),'trigger':obj(trigger,dict(identity,**clocks,kind={'phone_lifecycle_gate','phone_carrier_abort_gate'})),'witness':obj(witness,dict(identity,**clocks,binding=binding,cause={'owner_pause','background','connection_retired','input_expired','host_revoked'},pen_release={'binding':binding,'held_before_release':'bool','release_started_qpc':'uint','release_completed_qpc':'uint','qpc_frequency':'uint'})),'records_validated':False,'acceptance':False}
    result['withheld_fields']=withheld
    return result

def validation_report(records,views,counts,raw,prefix,trigger,witness):
    out={'schema':3,'status':'rejected','rejection_reason':'unclassified','records_validated':False,'held_evidence':observations(records,prefix,trigger,witness)}
    try:value=validate(records,views,counts,raw,prefix,trigger,witness)
    except (ValueError,TypeError,KeyError,OverflowError) as error:out['rejection_reason']=closed_reason(error)
    else:out.update(status='passed',rejection_reason='none',records_validated=True,validation=value)
    return out

def main(arguments):
    records=prefix=trigger=witness=None
    out={'schema':3,'status':'rejected','rejection_reason':'unclassified','records_validated':False}
    try:
        observe=len(arguments)==5 and arguments[0]=='observe'
        require(observe or len(arguments)==7,'arguments')
        paths=arguments[1:] if observe else [arguments[0],*arguments[4:7]]
        values=[];first_error=None
        for path,limit in zip(paths,(65536,2048,2048,4096)):
            try:values.append(read(path,limit))
            except (ValueError,OSError,TypeError,KeyError,OverflowError) as error:
                values.append(None)
                if first_error is None:first_error=error
        records,prefix,trigger,witness=values
        out['held_evidence']=observations(records,prefix,trigger,witness)
        if observe:out.update(status='observed',rejection_reason='not_validated')
        else:
            if first_error is not None:raise first_error
            p=Path(arguments[3]);require(p.is_file() and not p.is_symlink(),'journal_file')
            with p.open('rb') as stream:raw=stream.read(262145)
            require(len(raw)<=262144,'journal_file_bytes')
            out=validation_report(records,read(arguments[1],65536),read(arguments[2],128),raw,prefix,trigger,witness)
    except (ValueError,OSError,TypeError,KeyError,OverflowError) as error:out['rejection_reason']=closed_reason(error)
    if 'held_evidence' not in out:out['held_evidence']=observations(records,prefix,trigger,witness)
    print(json.dumps(out,separators=(',',':')))
    return 0 if out['status'] in ('passed','observed') else 1

def ready_main():
    try:
        require(len(sys.argv)==6 and sys.argv[1]=='ready','arguments')
        if len(sys.argv)==6 and sys.argv[1]=='ready':
            p=Path(sys.argv[2]);deadline=time.monotonic()+5;next_report=0
            while True:
                require(p.is_file() and not p.is_symlink() and p.stat().st_size<=262144,'journal_file')
                with p.open('rb') as stream:raw=stream.read(262145)
                require(len(raw)<=262144,'journal_file_bytes');raw=raw[:raw.rfind(b'\n')+1]
                proof=ready_if_delivered(raw,sys.argv[4],sys.argv[5])
                if proof is not None:break
                now=time.monotonic();require(now<deadline,'actual_native_prefix_deadline')
                if now>=next_report:print('WAITING_ACTUAL_NATIVE_DOWN_MOVE',flush=True);next_report=now+1
                time.sleep(.01)
            out=Path(sys.argv[3]);require(not out.exists(),'fresh_ready_receipt');out.write_text(json.dumps(proof),encoding='utf-8');print('HELD_NATIVE_PREFIX_READY')
    except (ValueError,OSError,TypeError,KeyError,OverflowError) as error:
        print(json.dumps({'schema':3,'status':'rejected','rejection_reason':closed_reason(error),'records_validated':False,'held_evidence':observations(None,None,None,None)}));return 1
    return 0

if __name__=='__main__':sys.exit(ready_main() if len(sys.argv)>1 and sys.argv[1]=='ready' else main(sys.argv[1:]))
