use crate::{Error,Result,admission::alloc,check};
use vw_raster::Pixels;
pub(crate) fn apply(w:u32,h:u32,o:u8,p:Pixels,cancel:&dyn Fn()->bool)->Result<(u32,u32,Pixels)>{
    if !(1..=8).contains(&o){return Err(Error::Invalid);}if o==1{return Ok((w,h,p));}
    let (dw,dh)=if o>=5{(h,w)}else{(w,h)};
    fn run<T:Default+Clone+Copy>(w:u32,h:u32,dw:u32,o:u8,p:&[T],cancel:&dyn Fn()->bool)->Result<Vec<T>>{
        let mut q=alloc(p.len())?;
        for y in 0..h{check(cancel)?;for x in 0..w{
            let (dx,dy)=match o{2=>(w-1-x,y),3=>(w-1-x,h-1-y),4=>(x,h-1-y),5=>(y,x),6=>(h-1-y,x),7=>(h-1-y,w-1-x),8=>(y,w-1-x),_=>(x,y)};
            let a=((u64::from(y)*u64::from(w)+u64::from(x))*4)as usize;
            let b=((u64::from(dy)*u64::from(dw)+u64::from(dx))*4)as usize;
            q.get_mut(b..b+4).ok_or(Error::Invalid)?.copy_from_slice(p.get(a..a+4).ok_or(Error::Invalid)?);
        }}Ok(q)
    }
    let p=match p {Pixels::Rgba8(p)=>Pixels::Rgba8(run(w,h,dw,o,&p,cancel)?),Pixels::Rgba16(p)=>Pixels::Rgba16(run(w,h,dw,o,&p,cancel)?)};
    Ok((dw,dh,p))
}
