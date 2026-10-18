#![allow(dead_code,clippy::unwrap_used,clippy::expect_used,clippy::panic)]
use vw_model::Id;
use vw_proto::v1;
use vw_stream::*;
pub fn id(n:u8)->Id{Id::from_parts(1_700_000_000_000,[n;10]).unwrap()}
pub fn geometry(w:u32,h:u32,time:i64)->StreamGeometry{
    StreamGeometry::new(&v1::CaptureGeometry{source_kind:"monitor".into(),window_handle:0,monitor_id:"synthetic-display".into(),
        client_rect_host:Some(v1::RectI{x:-1920,y:-256,w:w as i32,h:h as i32}),dpi_scale:2.0,geometry_revision:1,timestamp_ns:time},w,h).unwrap()
}
pub fn identity(n:u64,w:u32,h:u32)->FrameIdentity{FrameIdentity{session:id(1),frame_id:n,geometry:geometry(w,h,n as i64*1000),timestamp_ns:n as i64*1000}}
pub fn ring(w:u32,h:u32)->FrameRing{FrameRing::default_for(id(1),geometry(w,h,1000)).unwrap()}
pub fn frame(ring:&mut FrameRing,n:u64,w:u32,h:u32,color:[u8;4])->OriginalFrame{
    let mut writer=ring.begin_frame(identity(n,w,h),&Cancellation::default()).unwrap();
    for p in writer.rgba_mut().chunks_exact_mut(4){p.copy_from_slice(&color)}
    let frame=writer.finish(&Cancellation::default()).unwrap();ring.publish(frame.clone()).unwrap();frame
}
pub fn display(frame:&OriginalFrame,previous:Option<&OriginalFrame>,keyframe:bool,old:Option<&DisplayFrame>)->DisplayFrame{
    let packet=encode_tiles(frame,previous,keyframe,85,128*1024*1024,&Cancellation::default()).unwrap();
    decode_tiles(&packet,old,256*1024*1024,&Cancellation::default()).unwrap()
}
