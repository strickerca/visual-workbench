//! Private persistent helper. Never prints image bytes, source IDs or text to
//! a console/log; stdout is the bounded typed pipe protocol only.
use vw_capture::{Error,Result,stream_wire::{self,Command,Header}};
#[cfg(windows)] use vw_capture::stream::{StreamCancellation,stream_error};
use std::sync::{Arc,atomic::{AtomicU64,Ordering}};
use std::time::{Duration,Instant};
fn main(){if run().is_err(){std::process::exit(1)}}
fn run()->Result<()> {
    let started=Instant::now();let deadline=Arc::new(AtomicU64::new(5000));let watchdog=deadline.clone();
    // Covers a blocked GPU/driver, codec, pipe write, or abandoned private pipe.
    // This watchdog owns only this exact helper, never another application.
    std::thread::Builder::new().name("vw-stream-watchdog".into()).spawn(move||loop{
        if started.elapsed().as_millis()>u128::from(watchdog.load(Ordering::Acquire)){std::process::exit(2)}
        std::thread::sleep(Duration::from_millis(5));
    }).map_err(|_|Error::Platform)?;
    let mut input=std::io::stdin().lock();let mut output=std::io::stdout().lock();
    let first=stream_wire::read_command(&mut input)?;
    let Command::Open{request}=&first else{return Err(Error::Invalid)};request.validate()?;
    let request_hash=vw_model::AssetId::hash(&stream_wire::encode_header(&first)?).to_string();
    #[cfg(not(windows))]{let _=(request_hash,deadline);stream_wire::write_packet(&mut output,&Header::Refused{sequence:0,error:Error::Unsupported},&[])?;return Ok(())}
    #[cfg(windows)]{
        use vw_proto::Message;
        use vw_model::Id;
        let _job=vw_capture::stream_process::confine_stream_helper()?;
        struct Runtime;
        impl Drop for Runtime{fn drop(&mut self){/* SAFETY: successful owned MTA initialization on this thread. */unsafe{::windows::Win32::System::WinRT::RoUninitialize();}}}
        // SAFETY: this helper owns the current thread apartment and all WGC work.
        unsafe{::windows::Win32::System::WinRT::RoInitialize(::windows::Win32::System::WinRT::RO_INIT_MULTITHREADED).map_err(|_|Error::Platform)?;}
        let _runtime=Runtime;let cancel=StreamCancellation::default();
        let mut stream=match vw_capture::windows::stream_capture::WgcStream::open(request.clone(),&cancel){Ok(value)=>value,Err(error)=>{
            stream_wire::write_packet(&mut output,&Header::Refused{sequence:0,error},&[])?;return Ok(())
        }};
        stream_wire::write_packet(&mut output,&Header::Ready{request_hash,ring_frames:request.ring_frames,ring_limit:request.ring_bytes,
            estimated_peak_bytes:request.peak_bytes()?,border_visible:true},&[])?;
        let mut sequence=0;let mut pacer=vw_stream::StreamPacer::new(request.target_fps).map_err(stream_error)?;
        let mut previous:Option<vw_stream::OriginalFrame>=None;let mut offered:Option<(u64,Instant)>=None;
        let mut first_frame=false;
        loop{
            deadline.store(now_ms(started)?.checked_add(5000).ok_or(Error::Limit)?,Ordering::Release);
            let command=stream_wire::read_command(&mut input)?;
            let next=command.sequence().filter(|v|*v>sequence).ok_or(Error::Invalid)?;sequence=next;
            let unavailable_is_recoverable=matches!(&command,Command::Original{..});
            if matches!(&command,Command::Original{..}){deadline.store(now_ms(started)?.checked_add(10_000).ok_or(Error::Limit)?,Ordering::Release);}
            let outcome=(||->Result<bool>{match command{
                Command::Open{..}=>Err(Error::Invalid),
                Command::Stop{..}=>{stream_wire::write_packet(&mut output,&Header::Stopped{sequence},&[])?;Ok(false)},
                Command::Heartbeat{..}=>{idle(&mut output,sequence,&stream)?;Ok(true)},
                Command::Original{capture_session_id,frame_id,..}=>{
                    let session=Id::try_from(capture_session_id).map_err(|_|Error::Invalid)?;
                    let frame=stream.originals().pin(&session,frame_id)?;let identity=stream.metadata(&frame)?;
                    let original=vw_stream::encode_original(&frame,vw_stream::MAX_LOSSLESS_BYTES,request.codec_workspace()?,cancel.codec()).map_err(stream_error)?;
                    let header=Header::Original{sequence,identity,asset_id:original.asset_id.to_string(),payload_bytes:u32::try_from(original.bytes().len()).map_err(|_|Error::Limit)?};
                    stream_wire::write_packet(&mut output,&header,original.bytes())?;Ok(true)
                },
                Command::Poll{feedback,repair,..}=>{
                    match (offered.take(),feedback){
                        (Some((expected,began)),Some(receipt)) if expected==receipt.offer_sequence=>pacer.complete(u64::try_from(began.elapsed().as_millis()).map_err(|_|Error::Limit)?,receipt.admitted_to_media).map_err(stream_error)?,
                        (None,None)=>{},_=>return Err(Error::Invalid),
                    }
                    if repair{pacer.keyframe_required();}
                    let now=now_ms(started)?;
                    if !pacer.can_begin(now){idle(&mut output,sequence,&stream)?;return Ok(true)}
                    let captured=stream.poll_frame(if first_frame{0}else{100},&cancel)?;
                    let frame=match captured{
                        Some(frame)=>{first_frame=true;frame},
                        None if pacer.keyframe_pending()=>match stream.originals().latest(){Ok(frame)=>frame,Err(Error::Unavailable)=>{
                            if now>=3000{return Err(Error::Timeout)}idle(&mut output,sequence,&stream)?;return Ok(true)
                        },Err(error)=>return Err(error)},
                        None=>{idle(&mut output,sequence,&stream)?;return Ok(true)},
                    };
                    let Some(keyframe)=pacer.begin(now)else{return Err(Error::Busy)};
                    let began=Instant::now();
                    let mut packet=match vw_stream::encode_tiles(&frame,previous.as_ref(),keyframe,pacer.quality(),request.codec_workspace()?,cancel.codec()){
                        Ok(value)=>value,Err(error)=>{pacer.cancelled();return Err(stream_error(error))},
                    };
                    // Bind the actual retained capture wall time, including static-frame repairs.
                    packet.captured_at_ms=stream.metadata(&frame)?.captured_at_ms;
                    let encoded=packet.encode_to_vec();
                    let header=Header::Tiles{sequence,frame_id:frame.identity().frame_id,timestamp_ns:frame.identity().timestamp_ns,
                        payload_bytes:u32::try_from(encoded.len()).map_err(|_|Error::Limit)?};
                    stream_wire::write_packet(&mut output,&header,&encoded)?;
                    previous=Some(frame);offered=Some((sequence,began));Ok(true)
                },
            }})();
            match outcome{
                Ok(true)=>{},Ok(false)=>return Ok(()),Err(error)=>{
                    stream_wire::write_packet(&mut output,&Header::Refused{sequence,error},&[])?;
                    if error!=Error::Unavailable||!unavailable_is_recoverable{return Ok(())}
                },
            }
        }
    }
}
#[cfg(windows)]
fn now_ms(started:Instant)->Result<u64>{u64::try_from(started.elapsed().as_millis()).map_err(|_|Error::Limit)}
#[cfg(windows)]
fn idle(output:&mut impl std::io::Write,sequence:u64,stream:&vw_capture::windows::stream_capture::WgcStream)->Result<()>{
    stream_wire::write_packet(output,&Header::Idle{sequence,retained_frames:stream.originals().retained_frames(),retained_bytes:stream.originals().retained_bytes()},&[])
}
