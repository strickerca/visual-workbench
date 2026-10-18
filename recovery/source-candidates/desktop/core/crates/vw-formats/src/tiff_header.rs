//! Borrowed first-IFD admission precedes tiff::Decoder's eager metadata parse.
use crate::{Error,Result,Header,Format,Limits,Notice};
use crate::admission::{range,profile};
use vw_model::AssetId;
#[derive(Clone,Copy)] struct Order(bool);
impl Order {
    fn number(self,b:&[u8])->Result<u64>{let mut a=[0_u8;8];if b.len()>8{return Err(Error::Invalid);}if self.0{a[..b.len()].copy_from_slice(b);Ok(u64::from_le_bytes(a))}else{a[8-b.len()..].copy_from_slice(b);Ok(u64::from_be_bytes(a))}}
    fn get(self,b:&[u8],at:u64,n:u64)->Result<u64>{self.number(range(b,at,n)?)}
}
#[derive(Clone,Copy)]struct Entry<'a>{tag:u16,kind:u16,count:u64,bytes:&'a[u8]}
impl Entry<'_>{fn uint(self,o:Order,i:u64)->Result<u64>{let size=match self.kind{1=>1,3=>2,4|13=>4,16|18=>8,_=>return Err(Error::Invalid)};if i>=self.count{return Err(Error::Invalid);}o.number(range(self.bytes,i*size,size)?)} }
fn get<'a>(v:&[Entry<'a>],tag:u16)->Option<Entry<'a>>{v.iter().find(|e|e.tag==tag).copied()}
fn scalar(v:&[Entry<'_>],tag:u16,o:Order,default:Option<u64>)->Result<u64>{match get(v,tag){Some(e)if e.count==1=>e.uint(o,0),Some(_)=>Err(Error::Invalid),None=>default.ok_or(Error::Invalid)}}

pub(crate) fn inspect(bytes:&[u8],limits:Limits)->Result<Header>{
    let o=match bytes.get(..2){Some(b"II")=>Order(true),Some(b"MM")=>Order(false),_=>return Err(Error::Invalid)};
    let (ifd,count_size,entry_size,inline)=match o.get(bytes,2,2)?{
        42=>(o.get(bytes,4,4)?,2,12,4),
        43=>{if o.get(bytes,4,2)?!=8 || o.get(bytes,6,2)?!=0{return Err(Error::Invalid);}(o.get(bytes,8,8)?,8,20,8)},
        _=>return Err(Error::Unsupported),
    };
    if ifd<if inline==8{16}else{8}{return Err(Error::Invalid);}
    let count=o.get(bytes,ifd,count_size)?;if count==0 || count>256{return Err(Error::Limit);}
    let table_end=ifd.checked_add(count_size).and_then(|n|n.checked_add(count.checked_mul(entry_size)?)).ok_or(Error::Invalid)?;
    range(bytes,ifd,table_end-ifd+inline)?;
    let next=o.get(bytes,table_end,inline)?;
    if next!=0{if next==ifd{return Err(Error::Invalid);}range(bytes,next,count_size)?;}
    let mut entries=Vec::new();entries.try_reserve_exact(count as usize).map_err(|_|Error::Allocation)?;
    let mut total=0_u64;
    for i in 0..count{
        let at=ifd+count_size+i*entry_size;let tag=o.get(bytes,at,2)? as u16;let kind=o.get(bytes,at+2,2)? as u16;
        if entries.iter().any(|e:&Entry<'_>|e.tag==tag){return Err(Error::Invalid);}
        let n=o.get(bytes,at+4,if inline==8{8}else{4})?;
        let size=match kind{1|2|6|7=>1,3|8=>2,4|9|11|13=>4,5|10|12|16|17|18=>8,_=>return Err(Error::Unsupported)};
        let length=n.checked_mul(size).ok_or(Error::Limit)?;
        if length>4*1024*1024 || (tag!=34675 && n>65536){return Err(Error::Limit);}
        total=total.checked_add(length).ok_or(Error::Limit)?;if total>16*1024*1024{return Err(Error::Limit);}
        let value_at=at+if inline==8{12}else{8};let loc=if length<=inline{value_at}else{o.get(bytes,value_at,inline)?};
        let value=range(bytes,loc,length)?;
        entries.push(Entry{tag,kind,count:n,bytes:value});
    }
    let width=u32::try_from(scalar(&entries,256,o,None)?).map_err(|_|Error::Limit)?;
    let height=u32::try_from(scalar(&entries,257,o,None)?).map_err(|_|Error::Limit)?;
    let photo=scalar(&entries,262,o,None)?;
    if !matches!(photo,0|1|2){return Err(Error::Unsupported);}
    let base=if photo==2{3}else{1};let samples=scalar(&entries,277,o,Some(1))?;
    if samples!=base && samples!=base+1{return Err(Error::Unsupported);}
    let bits=get(&entries,258).ok_or(Error::Depth)?;
    if bits.count!=samples{return Err(Error::Invalid);}let depth=bits.uint(o,0)?;
    if !matches!(depth,8|16){return Err(Error::Depth);}
    for i in 0..samples{if bits.uint(o,i)?!=depth{return Err(Error::Depth);}}
    if let Some(e)=get(&entries,339){if e.count!=samples{return Err(Error::Invalid);}for i in 0..samples{if e.uint(o,i)?!=1{return Err(Error::Depth);}}}
    let alpha=samples>base;
    // The pinned decoder's WhiteIsZero inversion does not distinguish a
    // multiband alpha sample. Refuse instead of inverting alpha implicitly.
    if photo==0 && alpha{return Err(Error::Unsupported);}
    if alpha {let e=get(&entries,338).ok_or(Error::Unsupported)?;if e.count!=1 || e.uint(o,0)?!=2{return Err(Error::Unsupported);}}
    else if get(&entries,338).is_some(){return Err(Error::Invalid);}
    let orientation=u8::try_from(scalar(&entries,274,o,Some(1))?).map_err(|_|Error::Invalid)?;if !(1..=8).contains(&orientation){return Err(Error::Invalid);}
    let planar=match scalar(&entries,284,o,Some(1))?{1=>false,2=>true,_=>return Err(Error::Unsupported)};
    let compression=scalar(&entries,259,o,Some(1))?;
    if !matches!(compression,1|5|7|8|32773|32946){return Err(Error::Unsupported);}
    let predictor=scalar(&entries,317,o,Some(1))?;
    if !matches!(predictor,1|2) || compression==7 && predictor!=1{return Err(Error::Unsupported);}
    if compression==7 && (depth!=8 || alpha || planar || photo==0){return Err(Error::Unsupported);}
    let is_tiled=get(&entries,322).is_some()||get(&entries,323).is_some();
    let (offset_tag,count_tag)=if is_tiled{
        let tw=scalar(&entries,322,o,None)?;let th=scalar(&entries,323,o,None)?;
        if tw==0||th==0||tw>65535||th>65535||tw.checked_mul(th).is_none_or(|n|n>limits.pixels){return Err(Error::Limit);}(324,325)
    }else{if scalar(&entries,278,o,Some(u64::from(height)))?==0{return Err(Error::Invalid);}(273,279)};
    let offsets=get(&entries,offset_tag).ok_or(Error::Invalid)?;let counts=get(&entries,count_tag).ok_or(Error::Invalid)?;
    if offsets.count==0||offsets.count!=counts.count||offsets.count>65536{return Err(Error::Limit);}
    let mut chunk_bytes=0_u64;
    for i in 0..offsets.count{let n=counts.uint(o,i)?;if n==0{return Err(Error::Invalid);}chunk_bytes=chunk_bytes.checked_add(n).ok_or(Error::Limit)?;if chunk_bytes>limits.encoded_bytes{return Err(Error::Limit);}range(bytes,offsets.uint(o,i)?,n)?;}
    if compression==7 {
        // tiff 0.11.3's JPEG branch does not forward its pixel limits to
        // zune-jpeg. Validate each exact baseline strip before any codec call.
        // Tiled, planar and color-converting JPEG variants remain explicit.
        if is_tiled{return Err(Error::Unsupported);}
        let rows=scalar(&entries,278,o,Some(u64::from(height)))?;
        if rows==0 || rows>u64::from(height) || offsets.count!=u64::from(height).div_ceil(rows){return Err(Error::Invalid);}
        if let Some(tables)=get(&entries,347){crate::jpeg::tables(tables.bytes)?;}
        for i in 0..offsets.count {
            let strip=range(bytes,offsets.uint(o,i)?,counts.uint(o,i)?)?;
            let actual_rows=rows.min(u64::from(height)-i*rows) as u32;
            crate::jpeg::image(strip,width,actual_rows,samples as u8)?;
        }
    }
    let icc=get(&entries,34675).map(|e|e.bytes);if let Some(p)=icc{profile(p)?;}
    let peak=limits.admit(width,height,bytes.len()as u64,icc.map_or(0,<[u8]>::len))?;
    let mut notices=Vec::new();if next!=0{notices.push(Notice::FirstTiffPageOnly);}if icc.is_none(){notices.push(Notice::UntaggedColor);}
    Ok(Header{format:Format::Tiff,source_asset:AssetId::hash(bytes),width,height,bit_depth:depth as u8,orientation,alpha,icc:icc.map(<[u8]>::to_vec),encoded_bytes:bytes.len()as u64,notices,estimated_peak_bytes:peak,samples:samples as u8,planar})
}
