"""Closed real-controller receipt parser. Shape only; native source owns ACK authority."""
import hashlib,json,re,sys
from pathlib import Path
from integration_reports import validate as validate_views,require,integer,UUID
KEYS={'remote_input_binding','remote_input_sequences','remote_input_owner','remote_input_frame','remote_input_ticket','remote_input_pts_us','remote_input_timing','remote_input_echo','remote_input_route'}
EVENT_KEYS={'message','index','count','pointer_available','pressure','pen_flags','performance_count','pointer_flags','pointer_id'}
ROUTE='surface_touch,software_generated=true,ghost_ack_fade=true,physical_pen_fidelity=false,editor_effect=false,latency_acceptance=false'
def uint(value,maximum):
    require(isinstance(value,int) and not isinstance(value,bool) and 0<=value<=maximum,'native_integer');return value
def validate_provenance(event):
    if 'input_provenance' not in event:return
    p=event['input_provenance']
    require(isinstance(p,dict) and set(p)=={'schema','source_available','device_type','origin_id','message_extra_info'},'receiver_provenance_shape')
    require(type(p['schema']) is int and p['schema']==1 and type(p['source_available']) is bool,'receiver_provenance_shape')
    require(isinstance(p['message_extra_info'],str) and re.fullmatch('[a-f0-9]{16}',p['message_extra_info']),'receiver_provenance_shape')
    for key in ('device_type','origin_id'):
        require((type(p[key]) is int and -(1<<31)<=p[key]<(1<<31)) if p['source_available'] else p[key] is None,'receiver_provenance_shape')

def receiver_event_shape(event,required_clock=False):
    if not isinstance(event,dict):return False
    keys=set(event)-{'input_provenance'}
    return keys==EVENT_KEYS|{'received_qpc'} if required_clock else keys in (EVENT_KEYS,EVENT_KEYS|{'received_qpc'})

def pointer_stroke(events,complete=True,allow_cancelled_up=False):
    # Windows may send a noncontact UPDATE after UP. Pressure alone cannot
    # establish hover: use actual GetPointerPenInfo flags and pointer identity.
    state='before';pointer=None;moves=0;hovers=0
    allowed=0x1|0x2|0x4|0x10|0x2000|0x4000|0x8000|0x10000|0x20000|0x40000
    for event in events:
        index=uint(event['index'],5);require(index<3,'no_mouse_contamination')
        flags=uint(event['pointer_flags'],(1<<32)-1);identity=uint(event['pointer_id'],65535)
        require(identity>0 and (pointer is None or identity==pointer),'exact_pointer_identity');pointer=identity
        require(flags&~allowed==0 and flags&0x70000==[0x10000,0x20000,0x40000][index],'pointer_transition_flags')
        require(not flags&0x8000 or (allow_cancelled_up and index==2),'cancelled_pointer')
        if index==0:
            require(state=='before' and flags&0x6==0x6,'one_contact_down');state='contact'
        elif index==2:
            require(state=='contact' and moves>=1 and not flags&0x4,'one_contact_up');state='after'
        elif state=='contact':
            require(flags&0x6==0x6,'contact_update_before_up');moves+=1
        else:
            require(state=='after' and not flags&0x14 and event['pressure']==0,'only_noncontact_update_after_up');hovers+=1
    require(state==('after' if complete else 'contact'),'complete_pointer_contact')
    return hovers

def validate(inputs,views,counts,events):
    validate_views(views)
    require(isinstance(inputs,list) and len(inputs)==1,'input_census')
    r=inputs[0];require(isinstance(r,dict) and set(r)==KEYS and all(isinstance(v,str) and len(v)<=2048 for v in r.values()),'input_shape')
    binding=r['remote_input_binding'].split(',');require(len(binding)==6,'input_binding')
    require(binding[:5]==views[0]['remote_render_scope'].split(','),'input_scope')
    require(UUID.fullmatch(binding[5]),'input_session')
    require(r['remote_input_owner']==views[0]['remote_render_owner'],'input_decoder_owner')
    require(r['remote_input_sequences']=='1,3','actual_three_sequence_census')
    require(integer(r['remote_input_frame'])>integer(views[0]['remote_render_frame']),'covering_frame')
    require(integer(r['remote_input_ticket'])!=integer(views[0]['remote_render_ticket']),'covering_ticket')
    require(integer(r['remote_input_pts_us'],(1<<63)-1)>integer(views[0]['remote_render_pts_us'],(1<<63)-1),'covering_pts')
    times=r['remote_input_timing'].split(',');require(len(times)==5,'input_timing')
    created,rendered,callback,first_ack,gone=[integer(v,(1<<63)-1)for v in times]
    require(created<=rendered<=callback and created<=first_ack<=callback<=gone and first_ack+150_000_000<=gone<created+500_000_000,'actual_ack_fade_before_creation_expiry')
    require(r['remote_input_echo']=='3,0,0','matching_echo_coverage')
    require(r['remote_input_route']==ROUTE,'software_input_limits')
    require(isinstance(counts,list) and len(counts)==6,'receiver_categories')
    counts=[uint(v,20000)for v in counts]
    require(sum(counts)<=20000 and counts[0]==1 and counts[1]>=1 and counts[2]==1 and counts[3:]==[0,0,0],'balanced_native_pointer_receiver')
    require(isinstance(events,list) and 3<=len(events)<=20000,'receiver_event_census')
    seen=[0]*6;previous_qpc=0
    for e in events:
        require(receiver_event_shape(e),'receiver_event_shape');validate_provenance(e)
        if 'received_qpc' in e:
            clock=e['received_qpc'];require(isinstance(clock,dict) and set(clock)=={'schema','counter','frequency'} and uint(clock['schema'],1)==1,'receiver_clock_shape')
            require(uint(clock['counter'],(1<<63)-1)>0 and uint(clock['frequency'],(1<<63)-1)>0,'receiver_clock_actual')
        index=uint(e['index'],5);seen[index]+=1
        require(uint(e['count'],20000)==seen[index],'receiver_count_order')
        require(index<3 and uint(e['message'],(1<<32)-1)==[0x246,0x245,0x247][index],'actual_pointer_message')
        require(e['pointer_available'] is True,'actual_pointer_info')
        uint(e['pressure'],1024);require(uint(e['pen_flags'],7)==0,'unaltered_pen_flags')
        qpc=uint(e['performance_count'],(1<<64)-1)
        # POINTER_INFO may leave PerformanceCount unavailable (zero). The
        # host's successful guarded injection QPC/retained frame owns ACK,
        # not this optional receiver field. Never manufacture a timestamp.
        if qpc:require(qpc>=previous_qpc,'native_pointer_qpc_order');previous_qpc=qpc
    require(seen==counts,'all_six_receiver_journal_categories')
    hovers=pointer_stroke(events)
    return {'schema':1,'post_up_noncontact_updates':hovers,'controller_input_records':1,'actual_local_admissions':3,'receiver_categories':6,'receiver_qpc_available':all(e['performance_count']>0 for e in events),'software_generated_input':True,'native_retained_ack_producer_required':True,'text_correlation_only':True,'physical_pen_fidelity':False,'editor_effect':False,'latency_acceptance':False}
def unique(pairs):
    result={}
    for key,value in pairs:
        require(key not in result,'duplicate_json_key');result[key]=value
    return result
def read(path,limit,journal=False):
    p=Path(path);require(p.is_file() and not p.is_symlink() and p.stat().st_size<=limit,'receipt_file')
    with p.open('rb') as stream:raw=stream.read(limit+1)
    require(len(raw)<=limit,'receipt_file_bytes');text=raw.decode('utf-8-sig')
    if journal:return [json.loads(line,object_pairs_hook=unique)for line in text.splitlines() if line]
    return json.loads(text,object_pairs_hook=unique)

# Fixed rejection labels only: exception messages/paths never enter evidence.
REASONS=set(('arguments receipt_file receipt_file_bytes duplicate_json_key native_integer no_mouse_contamination exact_pointer_identity pointer_transition_flags cancelled_pointer one_contact_down one_contact_up contact_update_before_up only_noncontact_update_after_up complete_pointer_contact input_census input_shape input_binding input_scope input_session input_decoder_owner actual_three_sequence_census covering_frame covering_ticket covering_pts input_timing actual_ack_fade_before_creation_expiry matching_echo_coverage software_input_limits receiver_categories balanced_native_pointer_receiver receiver_event_census receiver_event_shape receiver_clock_shape receiver_clock_actual receiver_count_order actual_pointer_message actual_pointer_info unaltered_pen_flags native_pointer_qpc_order all_six_receiver_journal_categories receiver_provenance_shape scenario render_census record_shape scope_shape scope_uuid owner_uuid integer_shape integer_bound codec_name timing_shape local_callback_order capability_shape fresh_owner_capture_scope background_and_reconnect_epochs carrier_loss_and_reconnect_epochs').split())

def rejection_reason(error):
    label=error.args[0] if len(error.args)==1 and isinstance(error.args[0],str) else None
    return label if label in REASONS else 'unclassified'

def safe_integer(value,maximum,minimum=0):
    return value if type(value) is int and minimum<=value<=maximum else None

def event_observation(event,ordinal):
    out={'ordinal':ordinal,'malformed':not isinstance(event,dict),'withheld_fields':[]}
    if not isinstance(event,dict):return out
    limits={'message':(1<<32)-1,'index':5,'count':20000,'pressure':1024,'pen_flags':7,'performance_count':(1<<64)-1,'pointer_flags':(1<<32)-1,'pointer_id':65535}
    for key,maximum in limits.items():
        value=event.get(key);out[key]=safe_integer(value,maximum)
        if value is not None and out[key] is None:out['withheld_fields'].append(key)
    value=event.get('pointer_available');out['pointer_available']=value if type(value) is bool else None
    if type(value) is not bool:out['withheld_fields'].append('pointer_available')
    if 'received_qpc' in event:
        q=event['received_qpc'];out['received_qpc']=None
        if isinstance(q,dict) and set(q)=={'schema','counter','frequency'} and type(q['schema']) is int and q['schema']==1 and safe_integer(q['counter'],(1<<63)-1) is not None and safe_integer(q['frequency'],(1<<63)-1) is not None:
            out['received_qpc']={key:q[key] for key in ('schema','counter','frequency')}
        else:out['withheld_fields'].append('received_qpc')
    if 'input_provenance' in event:
        out['input_provenance']=None
        try:validate_provenance(event)
        except (ValueError,KeyError,TypeError):out['withheld_fields'].append('input_provenance')
        else:out['input_provenance']={key:event['input_provenance'][key] for key in ('schema','source_available','device_type','origin_id','message_extra_info')}
    out['unknown_field_count']=len(set(event)-EVENT_KEYS-{'received_qpc','input_provenance'})
    return out

def receiver_evidence(counts,events,raw=None):
    rows=events if isinstance(events,list) else []
    indices=list(range(min(32,len(rows))))
    if len(rows)>32:indices+=list(range(max(32,len(rows)-32),len(rows)))
    safe_counts=counts if isinstance(counts,list) and len(counts)==6 and all(safe_integer(v,20000) is not None for v in counts) and sum(counts)<=20000 else None
    return {'schema':1,'snapshot_complete':raw is not None,'counts':safe_counts,'event_count':len(rows),'retained_event_count':len(indices),'omitted_event_count':len(rows)-len(indices),'events':[event_observation(rows[i],i)for i in indices],'journal_bytes':len(raw)if raw is not None else None,'journal_sha256':hashlib.sha256(raw).hexdigest()if raw is not None else None,'journal_ends_with_newline':raw.endswith(b'\n')if raw is not None else None,'records_validated':False,'acceptance':False}

def load_receiver(counts_path,journal_path):
    p=Path(journal_path);require(p.is_file() and not p.is_symlink(),'receipt_file')
    with p.open('rb') as stream:raw=stream.read(262145)
    require(len(raw)<=262144,'receipt_file_bytes')
    # Retain a bounded closed projection even when individual lines are invalid.
    events=[]
    for line in raw.decode('utf-8-sig').splitlines():
        if not line:continue
        try:events.append(json.loads(line,object_pairs_hook=unique))
        except (ValueError,TypeError):events.append(None)
    counts=read(counts_path,128)
    return counts,events,raw

def validation_report(inputs,views,counts,events,raw=None):
    result={'schema':2,'status':'rejected','rejection_reason':'unclassified','records_validated':False,'receiver_evidence':receiver_evidence(counts,events,raw)}
    try:validation=validate(inputs,views,counts,events)
    except (ValueError,TypeError,KeyError,OverflowError) as error:result['rejection_reason']=rejection_reason(error)
    else:result.update(status='passed',rejection_reason='none',records_validated=True,validation=validation)
    return result

def main(arguments):
    result={'schema':2,'status':'rejected','rejection_reason':'unclassified','records_validated':False,'receiver_evidence':receiver_evidence(None,[])}
    try:
        observing=len(arguments)==3 and arguments[0]=='observe-receiver'
        require(observing or len(arguments)==4,'arguments')
        counts,events,raw=load_receiver(arguments[-2],arguments[-1])
        result['receiver_evidence']=receiver_evidence(counts,events,raw)
        if observing:result.update(status='observed',rejection_reason='not_validated')
        else:result=validation_report(read(arguments[0],65536),read(arguments[1],65536),counts,events,raw)
    except (ValueError,OSError,TypeError,KeyError,OverflowError) as error:result['rejection_reason']=rejection_reason(error)
    print(json.dumps(result,separators=(',',':')))
    return 0 if result['status'] in ('passed','observed') else 1

if __name__=='__main__':sys.exit(main(sys.argv[1:]))
