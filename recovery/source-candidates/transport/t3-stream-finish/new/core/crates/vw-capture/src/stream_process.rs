//! One pinned persistent helper with one bounded pipe exchange at a time.
//! Cancellation retains admission, executable lease and pipe ownership until
//! the exact child is affirmatively reaped and its I/O thread has finished.
use crate::{Error,Result,process::LockedHelper,stream::{StreamCancellation,StreamRequest},stream_wire::{self,Command,Header,Packet}};
use std::{io::Write,process::{Child,Command as ProcessCommand,Stdio},sync::{Arc,atomic::{AtomicBool,Ordering},mpsc},thread,time::{Duration,Instant}};
use vw_proto::Message;
static ACTIVE:AtomicBool=AtomicBool::new(false);
struct Permit;
impl Drop for Permit{fn drop(&mut self){ACTIVE.store(false,Ordering::Release);}}
pub struct StreamProcess {
    child:Child,send:Option<mpsc::SyncSender<Vec<u8>>>,receive:mpsc::Receiver<Result<Packet>>,
    io:Option<thread::JoinHandle<()>>,sequence:u64,closed:bool,
    request:StreamRequest,
    _helper:Arc<LockedHelper>,_permit:Permit,
}
impl StreamProcess {
    pub fn open(helper:Arc<LockedHelper>,request:StreamRequest,cancel:&StreamCancellation)->Result<Self>{
        request.validate()?;cancel.check()?;
        let command=Command::Open{request:request.clone()};let encoded=stream_wire::encode_header(&command)?;
        let expected=vw_model::AssetId::hash(&encoded).to_string();
        ACTIVE.compare_exchange(false,true,Ordering::AcqRel,Ordering::Acquire).map_err(|_|Error::Busy)?;let permit=Permit;
        let mut launch=ProcessCommand::new(helper.stream_path());launch.stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::null()).env_clear();
        #[cfg(windows)]{use std::os::windows::process::CommandExt;launch.creation_flags(0x08000000);}
        let mut child=launch.spawn().map_err(|_|Error::Platform)?;
        let (send,commands)=mpsc::sync_channel::<Vec<u8>>(1);let(results,receive)=mpsc::sync_channel(1);
        let io=(||{
            let mut input=child.stdin.take().ok_or(Error::Platform)?;let mut output=child.stdout.take().ok_or(Error::Platform)?;
            thread::Builder::new().name("vw-stream-pipe".into()).spawn(move||{
                while let Ok(bytes)=commands.recv(){
                    let result=input.write_all(&(bytes.len()as u32).to_le_bytes()).and_then(|()|input.write_all(&bytes)).and_then(|()|input.flush())
                        .map_err(|_|Error::Platform).and_then(|()|stream_wire::read_packet(&mut output));
                    let failed=result.is_err();if results.send(result).is_err()||failed{break}
                }
            }).map_err(|_|Error::Platform)
        })();
        let io=match io{Ok(value)=>value,Err(error)=>{reap(&mut child);return Err(error)}};
        let mut owner=Self{child,send:Some(send),receive,io:Some(io),sequence:0,closed:false,request:request.clone(),_helper:helper,_permit:permit};
        let ready=owner.exchange_bytes(encoded,cancel,5000)?;
        match ready.header{
            Header::Ready{request_hash,ring_frames,ring_limit,estimated_peak_bytes,border_visible}
                if request_hash==expected&&ring_frames==request.ring_frames&&ring_limit==request.ring_bytes
                    &&estimated_peak_bytes==request.peak_bytes()?&&border_visible&&ready.payload.is_empty()=>Ok(owner),
            Header::Refused{error,..}=>Err(error),_=>Err(Error::Invalid),
        }
    }
    pub fn exchange(&mut self,command:Command,cancel:&StreamCancellation)->Result<Packet>{
        cancel.check()?;if self.closed{return Err(Error::Unavailable)}
        let sequence=command.sequence().ok_or(Error::Invalid)?;
        if sequence==0||sequence<=self.sequence{return Err(Error::Invalid)}
        let timeout=if matches!(&command,Command::Original{..}){10_000}else{5_000};
        let bytes=stream_wire::encode_header(&command)?;self.sequence=sequence;
        let packet=self.exchange_bytes(bytes,cancel,timeout)?;
        let received=match &packet.header{Header::Tiles{sequence,..}|Header::Original{sequence,..}|Header::Idle{sequence,..}
            |Header::Refused{sequence,..}|Header::Stopped{sequence}=>*sequence,Header::Ready{..}=>0};
        if received!=sequence{self.terminate();return Err(Error::Invalid)}
        if let Err(error)=validate_reply(&self.request,&command,&packet){self.terminate();return Err(error)}
        if let Header::Refused{error,..}=&packet.header
            && (!matches!(&command,Command::Original{..})||*error!=Error::Unavailable){self.terminate();}
        if matches!(packet.header,Header::Stopped{..}){
            self.finish_process()?;
        }
        Ok(packet)
    }
    fn exchange_bytes(&mut self,bytes:Vec<u8>,cancel:&StreamCancellation,timeout_ms:u64)->Result<Packet>{
        if self.closed{return Err(Error::Unavailable)}
        if cancel.check().is_err(){self.terminate();return Err(Error::Cancelled)}
        if self.send.as_ref().ok_or(Error::Unavailable)?.try_send(bytes).is_err(){self.terminate();return Err(Error::Busy)}
        let started=Instant::now();
        loop{
            if cancel.check().is_err(){self.terminate();return Err(Error::Cancelled)}
            if started.elapsed()>=Duration::from_millis(timeout_ms){self.terminate();return Err(Error::Timeout)}
            match self.receive.recv_timeout(Duration::from_millis(5)){
                Ok(Ok(packet))=>return Ok(packet),
                Ok(Err(error))=>{self.terminate();return Err(error)},
                Err(mpsc::RecvTimeoutError::Disconnected)=>{self.terminate();return Err(Error::Platform)},
                Err(mpsc::RecvTimeoutError::Timeout)=>{},
            }
            match self.child.try_wait(){Ok(Some(_))=>{
                // The I/O worker may be transferring the final buffered packet.
                // Keep the bounded receive deadline rather than discarding it.
            },Ok(None)=>{},Err(_)=>{self.terminate();return Err(Error::Platform)}}
        }
    }
    fn finish_process(&mut self)->Result<()>{
        self.send.take();let started=Instant::now();let success=loop{
            match self.child.try_wait(){Ok(Some(status))=>break status.success(),Ok(None)=>{},Err(_)=>break false}
            if started.elapsed()>=Duration::from_secs(2){break false}thread::sleep(Duration::from_millis(5));
        };
        if !success{self.terminate();return Err(Error::Platform)}
        if let Some(io)=self.io.take(){io.join().map_err(|_|Error::Platform)?;}
        self.closed=true;Ok(())
    }
    pub fn terminate(&mut self){
        if self.closed{return}self.send.take();reap(&mut self.child);
        if let Some(io)=self.io.take(){let _=io.join();}self.closed=true;
    }
}
impl Drop for StreamProcess{fn drop(&mut self){self.terminate();}}
fn reap(child:&mut Child){loop{match child.try_wait(){Ok(Some(_))=>return,Ok(None)|Err(_)=>{let _=child.kill();thread::sleep(Duration::from_millis(5));}}}}

/// Metadata/content admission only. A caller still uses vw_stream's bounded
/// JPEG/PNG decoders before showing or saving a purported lossless original.
pub fn validate_reply(request:&StreamRequest,command:&Command,packet:&Packet)->Result<()> {
    let expected_sequence=command.sequence().ok_or(Error::Invalid)?;
    let actual_sequence=match &packet.header{Header::Tiles{sequence,..}|Header::Original{sequence,..}|Header::Idle{sequence,..}|Header::Refused{sequence,..}|Header::Stopped{sequence}=>*sequence,Header::Ready{..}=>0};
    if actual_sequence!=expected_sequence || expected_sequence==0 {return Err(Error::Invalid)}
    if !matches!(packet.header,Header::Tiles{..}|Header::Original{..})&&!packet.payload.is_empty(){return Err(Error::Invalid)}
    match (&packet.header,command){
        (Header::Refused{..},_)=>Ok(()),
        (Header::Stopped{..},Command::Stop{..})=>Ok(()),
        (Header::Idle{retained_frames,retained_bytes,..},Command::Heartbeat{..}|Command::Poll{..})=>{
            if *retained_frames>request.ring_frames||*retained_bytes>request.ring_bytes{return Err(Error::Invalid)}Ok(())
        },
        (Header::Tiles{frame_id,timestamp_ns,payload_bytes,..},Command::Poll{..})=>{
            if packet.payload.len()!=*payload_bytes as usize||u64::try_from(*timestamp_ns).map_err(|_|Error::Invalid)?<request.source.observed_ns(){return Err(Error::Invalid)}
            vw_proto::capture_admission::frame_tiles(&packet.payload).map_err(|_|Error::Limit)?;
            let tiles=vw_proto::v1::FrameTiles::decode(packet.payload.as_slice()).map_err(|_|Error::Invalid)?;
            let expected=request.geometry(u64::try_from(*timestamp_ns).map_err(|_|Error::Invalid)?)?;
            let actual=vw_stream::StreamGeometry::new(tiles.geometry.as_ref().ok_or(Error::Invalid)?,tiles.width,tiles.height).map_err(super::stream::stream_error)?;
            if !(1..=253_402_300_799_999).contains(&tiles.captured_at_ms)||tiles.frame_id!=*frame_id||tiles.timestamp_ns!=*timestamp_ns||actual!=expected
                || vw_model::Id::from_proto(tiles.capture_session_id.as_ref()).map_err(|_|Error::Invalid)?.as_str()!=request.capture_session_id
                || tiles.last_input_seq_applied!=0||tiles.tiles.iter().any(|t|t.format!="jpeg"){return Err(Error::Invalid)}
            Ok(())
        },
        (Header::Original{identity,asset_id,payload_bytes,..},Command::Original{capture_session_id,frame_id,..})=>{
            if packet.payload.len()!=*payload_bytes as usize||packet.payload.len()>vw_stream::MAX_LOSSLESS_BYTES{return Err(Error::Invalid)}
            identity.validate()?;let rect=request.source.rect();let expected=request.geometry(identity.monotonic_timestamp_ns)?;let g=expected.wire();
            if capture_session_id!=&request.capture_session_id||identity.capture_session_id!=*capture_session_id||identity.frame_id!=*frame_id
                ||identity.geometry_revision!=request.geometry_revision||identity.client_rect!=rect||identity.dpi_scale!=g.dpi_scale
                ||identity.source_kind!=g.source_kind||identity.window_handle!=g.window_handle||identity.monitor_id!=g.monitor_id
                ||identity.platform!="windows"||identity.monotonic_timestamp_ns<request.source.observed_ns()
                ||vw_model::AssetId::hash(&packet.payload).as_str()!=asset_id.as_str(){return Err(Error::Invalid)}Ok(())
        },
        _=>Err(Error::Invalid),
    }
}

/// Separate from the reviewed one-shot helper's 384MiB policy. The ring keeps
/// its independent <=300MB admission. This Job forbids child processes.
#[cfg(windows)]
pub fn confine_stream_helper()->Result<impl Drop>{
    use ::windows::Win32::{Foundation::*,System::{JobObjects::*,Threading::*}};
    struct Job(HANDLE);impl Drop for Job{fn drop(&mut self){/* SAFETY: uniquely owned unnamed Job handle. */unsafe{let _=CloseHandle(self.0);}}}
    // SAFETY: initialized structure and current helper process only. No
    // KILL_ON_CLOSE on this self-owned handle; parent owns termination.
    unsafe{let job=Job(CreateJobObjectW(None,None).map_err(|_|Error::Platform)?);let mut limits=JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
        limits.BasicLimitInformation.LimitFlags=JOB_OBJECT_LIMIT_ACTIVE_PROCESS|JOB_OBJECT_LIMIT_PROCESS_MEMORY;
        limits.BasicLimitInformation.ActiveProcessLimit=1;limits.ProcessMemoryLimit=super::stream::PROCESS_MEMORY_BYTES as usize;
        SetInformationJobObject(job.0,JobObjectExtendedLimitInformation,(&limits as *const JOBOBJECT_EXTENDED_LIMIT_INFORMATION).cast(),std::mem::size_of_val(&limits)as u32).map_err(|_|Error::Platform)?;
        AssignProcessToJobObject(job.0,GetCurrentProcess()).map_err(|_|Error::Platform)?;Ok(job)}
}
