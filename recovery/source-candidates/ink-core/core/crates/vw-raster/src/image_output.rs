//! One sample-space output transform for preview, export and package pixels.
use crate::{DecodedImage,Pixels,RasterError,Region};
use serde::{Deserialize,Serialize};
use vw_geom::{ImageOutputFrame,QuarterTurn};

/// Embedded export/package mapping; D stays the oriented immutable source.
#[derive(Debug,Clone,Copy,PartialEq,Eq,Serialize,Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ImageOutputMetadata {
    pub version:u32,
    pub source_width:u32,
    pub source_height:u32,
    pub crop:Region,
    pub clockwise_degrees:u16,
    pub output_width:u32,
    pub output_height:u32,
    /// Fixed, explicit policy. This is interpolation of encoded source samples,
    /// not a claim of linear-light resampling or an automatic color conversion.
    pub resampling:ImageOutputResampling,
}
#[derive(Debug,Clone,Copy,PartialEq,Eq,Serialize,Deserialize)]
pub enum ImageOutputResampling { PremultipliedBilinearSourceSamplesV1 }
impl ImageOutputMetadata {
    pub fn new(frame:ImageOutputFrame)->Self {
        let [source_width,source_height]=frame.source_size();let [x,y,width,height]=frame.crop();
        let [output_width,output_height]=frame.output_size();
        Self{version:1,source_width,source_height,crop:Region{x,y,width,height},
            clockwise_degrees:match frame.rotation(){QuarterTurn::Zero=>0,QuarterTurn::Clockwise90=>90,QuarterTurn::Half=>180,QuarterTurn::Clockwise270=>270},
            output_width,output_height,resampling:ImageOutputResampling::PremultipliedBilinearSourceSamplesV1}
    }
    pub fn frame(self)->Result<ImageOutputFrame,RasterError> {
        if self.version!=1 {return Err(RasterError::Unsupported("image output version"));}
        let rotation=QuarterTurn::from_degrees(i32::from(self.clockwise_degrees)).map_err(|_|RasterError::Invalid("output rotation"))?;
        if self.clockwise_degrees>=360 {return Err(RasterError::Invalid("output rotation"));}
        ImageOutputFrame::new([self.source_width,self.source_height],[self.crop.x,self.crop.y,self.crop.width,self.crop.height],
            rotation,[self.output_width,self.output_height]).map_err(|_|RasterError::Invalid("image output grid"))
    }
}

/// Actual retained source allocation, including unused Vec capacity.
pub fn image_resident_bytes(source:&DecodedImage)->Result<u64,RasterError> {
    let pixels=match &source.pixels {Pixels::Rgba8(v)=>v.capacity() as u64,Pixels::Rgba16(v)=>(v.capacity() as u64).checked_mul(2).ok_or(RasterError::Allocation)?};
    pixels.checked_add(source.icc.as_ref().map_or(0,|v|v.capacity() as u64)).ok_or(RasterError::Allocation)
}
/// Nonallocating admission before ICC parsing or any output sample allocation.
/// Budget includes the complete caller-owned source allocation, one output,
/// copied metadata and admitted ICC parsing. At most 50 million output pixels.
pub fn image_output_estimate(source:&DecodedImage,frame:ImageOutputFrame,region:Region)->Result<u64,RasterError> {
    crate::export::image_shape(source)?;
    if !source.original_available {return Err(RasterError::OriginalRequired);}
    if frame.source_size()!=[source.width,source.height] {return Err(RasterError::Invalid("image output source extent"));}
    let [width,height]=frame.output_size();region.validate(width,height)?;
    if u64::from(width)*u64::from(height)>50_000_000 {return Err(RasterError::Unsupported("image output pixel limit"));}
    let output=u64::from(region.width)*u64::from(region.height)*u64::from(source.pixels.bit_depth())/2;
    let profile=crate::export::icc_workspace(source.icc.as_ref())?;
    image_resident_bytes(source)?.checked_add(output)
        .and_then(|v|v.checked_add(1024*1024)).and_then(|v|v.checked_add(profile))
        .ok_or(RasterError::Allocation)
}
fn axis(index:u32,source:u32,output:u32)->([u32;2],[u64;2],u64) {
    let denominator=2*u64::from(output);
    let numerator=(2*i64::from(index)+1)*i64::from(source)-i64::from(output);
    let whole=numerator.div_euclid(denominator as i64);
    let fraction=numerator.rem_euclid(denominator as i64) as u64;
    let left=whole.clamp(0,i64::from(source)-1) as u32;
    let right=(whole+1).clamp(0,i64::from(source)-1) as u32;
    ([left,right],[denominator-fraction,fraction],denominator)
}
/// Produce a full-resolution region of O; no thumbnail can serve as its input.
/// Exact crop/quarter-turn sampling preserves even hidden RGB under zero alpha.
/// Resizing uses integer rational weights and premultiplied alpha in the source
/// sample space, preserving 8/16-bit storage depth and the exact ICC bytes.
pub fn render_image_output_region(source:&DecodedImage,frame:ImageOutputFrame,region:Region,
    budget:u64,cancelled:&dyn Fn()->bool)->Result<DecodedImage,RasterError> {
    crate::source::check_cancel(cancelled)?;
    crate::source::check_memory(image_output_estimate(source,frame,region)?,budget)?;
    source.validate()?;
    let count=crate::checked_samples(region.width,region.height)?;
    let mut pixels=match &source.pixels {
        Pixels::Rgba8(_)=>{let mut v=crate::source::reserve::<u8>(count)?;v.resize(count,0);Pixels::Rgba8(v)},
        Pixels::Rgba16(_)=>{let mut v=crate::source::reserve::<u16>(count)?;v.resize(count,0);Pixels::Rgba16(v)},
    };
    let [rw,rh]=frame.rotated_size();let [ow,oh]=frame.output_size();
    for y in 0..region.height {
        crate::source::check_cancel(cancelled)?;
        let (ys,wy,dy)=axis(region.y+y,rh,oh);
        for x in 0..region.width {
            let (xs,wx,dx)=axis(region.x+x,rw,ow);
            let at=((u64::from(y)*u64::from(region.width)+u64::from(x))*4) as usize;
            let mut sums=[0u128;4];let mut exact=None;let mut contributions=0;
            for iy in 0..2 {for ix in 0..2 {
                let weight=u128::from(wx[ix])*u128::from(wy[iy]);if weight==0 {continue;}
                let [sx,sy]=frame.source_pixel(xs[ix],ys[iy]).map_err(|_|RasterError::Invalid("output sample"))?;
                let pos=((u64::from(sy)*u64::from(source.width)+u64::from(sx))*4) as usize;
                if exact.is_none() {exact=Some(pos);} else if exact!=Some(pos) {contributions=2;}
                let a=u128::from(source.pixels.sample(pos+3));sums[3]+=a*weight;
                for (channel,sum) in sums[..3].iter_mut().enumerate() {*sum+=u128::from(source.pixels.sample(pos+channel))*a*weight;}
            }}
            if contributions<2 {
                let pos=exact.ok_or(RasterError::Invalid("empty output sample"))?;
                for channel in 0..4 {pixels.set_sample(at+channel,source.pixels.sample(pos+channel));}
            } else {
                let area=u128::from(dx)*u128::from(dy);
                pixels.set_sample(at+3,((sums[3]+area/2)/area) as u16);
                for channel in 0..3 {
                    let sample=if sums[3]==0 {0}else{(sums[channel]+sums[3]/2)/sums[3]};
                    pixels.set_sample(at+channel,sample as u16);
                }
            }
        }
    }
    crate::source::check_cancel(cancelled)?;
    Ok(DecodedImage{width:region.width,height:region.height,pixels,icc:source.icc.clone(),source_asset:source.source_asset.clone(),
        original_available:source.original_available,orientation_applied:source.orientation_applied})
}
/// Full explicit output grid, subject to the same memory/work admission.
pub fn render_image_output(source:&DecodedImage,frame:ImageOutputFrame,budget:u64,
    cancelled:&dyn Fn()->bool)->Result<DecodedImage,RasterError> {
    let [width,height]=frame.output_size();
    render_image_output_region(source,frame,Region{x:0,y:0,width,height},budget,cancelled)
}
