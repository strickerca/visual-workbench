//! Nonallocating MEDIA structure admission before prost materializes repeated
//! tile vectors. Semantic geometry/codec/authentication checks remain required.
pub const MAX_MEDIA_BYTES:usize=8*1024*1024;
pub const MAX_CAPTURE_TILES:usize=128;
type Result<T>=std::result::Result<T,&'static str>;

#[derive(Clone,Copy)] enum Kind{Envelope,Tiles,Tile,Geometry,Rect,Uuid,Video}
struct Reader<'a>{bytes:&'a[u8],at:usize}
struct Field<'a>{number:u32,wire:u8,data:&'a[u8]}
impl<'a> Reader<'a>{
    fn varint(&mut self)->Result<u64>{
        let mut value=0u64;
        for index in 0..10{
            let byte=*self.bytes.get(self.at).ok_or("capture wire truncated")?;self.at+=1;
            if index==9&&byte>1{return Err("capture wire varint")}
            value|=u64::from(byte&127)<<(index*7);
            if byte&128==0{return Ok(value)}
        }
        Err("capture wire varint")
    }
    fn take(&mut self,n:usize)->Result<&'a[u8]>{
        let end=self.at.checked_add(n).filter(|v|*v<=self.bytes.len()).ok_or("capture wire length")?;
        let value=&self.bytes[self.at..end];self.at=end;Ok(value)
    }
    fn next(&mut self)->Result<Option<Field<'a>>>{
        if self.at==self.bytes.len(){return Ok(None)}
        let key=self.varint()?;let number=u32::try_from(key>>3).map_err(|_|"capture wire tag")?;
        if number==0||number>0x1fff_ffff{return Err("capture wire tag")}
        let wire=(key&7)as u8;
        let data=match wire{
            0=>{self.varint()?;&[]},1=>self.take(8)?,5=>self.take(4)?,
            2=>{let length=usize::try_from(self.varint()?).map_err(|_|"capture wire length")?;self.take(length)?},
            _=>return Err("capture wire group"),
        };
        Ok(Some(Field{number,wire,data}))
    }
}
fn bytes(field:&Field<'_>,maximum:usize)->Result<()>{if field.wire!=2||field.data.len()>maximum{Err("capture wire bytes")}else{Ok(())}}
fn scalar(field:&Field<'_>,wire:u8)->Result<()>{if field.wire==wire{Ok(())}else{Err("capture wire scalar")}}
fn nested(field:&Field<'_>,maximum:usize,kind:Kind)->Result<()>{bytes(field,maximum)?;walk(field.data,kind)}
fn walk(input:&[u8],kind:Kind)->Result<()>{
    let mut reader=Reader{bytes:input,at:0};let mut seen=0u64;let mut count=0usize;let mut tiles=0usize;let mut body=false;
    let maximum=match kind{Kind::Envelope=>4,Kind::Tiles=>11+MAX_CAPTURE_TILES,Kind::Tile=>3,Kind::Geometry=>7,Kind::Rect=>4,Kind::Uuid=>1,Kind::Video=>8};
    while let Some(field)=reader.next()?{
        count+=1;if count>maximum{return Err("capture wire fields")}
        let repeated=matches!(kind,Kind::Tiles)&&field.number==7;
        if !repeated{
            if field.number>=64{return Err("capture wire unknown")}
            let bit=1u64<<field.number;if seen&bit!=0{return Err("capture wire duplicate")}seen|=bit;
        }
        match kind{
            Kind::Envelope=>match field.number{
                1|2=>scalar(&field,0)?,3=>nested(&field,32,Kind::Uuid)?,
                51|52=>{if body{return Err("capture wire body")};body=true;
                    nested(&field,MAX_MEDIA_BYTES,if field.number==51{Kind::Tiles}else{Kind::Video})?;},
                _=>return Err("capture wire unknown"),
            },
            Kind::Tiles=>match field.number{
                1=>nested(&field,32,Kind::Uuid)?,2|4|5|6|8|9|10|11|12=>scalar(&field,0)?,3=>nested(&field,512,Kind::Geometry)?,
                7=>{tiles+=1;if tiles>MAX_CAPTURE_TILES{return Err("capture tile count")};nested(&field,MAX_MEDIA_BYTES,Kind::Tile)?;},
                _=>return Err("capture wire unknown"),
            },
            Kind::Tile=>match field.number{1=>nested(&field,64,Kind::Rect)?,2=>bytes(&field,16)?,3=>bytes(&field,MAX_MEDIA_BYTES)?,_=>return Err("capture wire unknown")},
            Kind::Geometry=>match field.number{1=>bytes(&field,32)?,2|6|7=>scalar(&field,0)?,3=>bytes(&field,128)?,4=>nested(&field,64,Kind::Rect)?,5=>scalar(&field,1)?,_=>return Err("capture wire unknown")},
            Kind::Rect=>match field.number{1..=4=>scalar(&field,0)?,_=>return Err("capture wire unknown")},
            Kind::Uuid=>match field.number{1=>bytes(&field,16)?,_=>return Err("capture wire unknown")},
            Kind::Video=>match field.number{1=>nested(&field,32,Kind::Uuid)?,2|5|6|8=>scalar(&field,0)?,3=>nested(&field,512,Kind::Geometry)?,4=>bytes(&field,16)?,7=>bytes(&field,MAX_MEDIA_BYTES)?,_=>return Err("capture wire unknown")},
        }
    }
    Ok(())
}
pub fn frame_tiles(input:&[u8])->Result<()>{
    if input.len()>MAX_MEDIA_BYTES{return Err("capture media quota")};walk(input,Kind::Tiles)
}
pub fn media_envelope(input:&[u8])->Result<()>{
    if input.len()>MAX_MEDIA_BYTES{return Err("capture media quota")};walk(input,Kind::Envelope)
}
