//! Header admission never allocates proportional to compressed source or pixels.
use crate::{Cancellation, CoreError, Result};
use std::{fs::File,io::{Read,Seek,SeekFrom,Write},path::Path};
use ring::digest::{Context,SHA256};

pub(super) const MAX_ORIGINAL:u64=4*1024*1024*1024;
const MAX_HEADER:u64=16*1024*1024;
#[derive(Clone,Debug,PartialEq,Eq)]
pub(super) struct Metadata { pub width:u32,pub height:u32,pub orientation:u8,pub format:&'static str }
impl Metadata {
    pub fn dimensions(&self)->(u32,u32){if self.orientation>=5{(self.height,self.width)}else{(self.width,self.height)}}
    fn admit(self)->Result<Self>{let(w,h)=self.dimensions();vw_assets::Pyramid::new(w,h).map_err(|_|CoreError::Invalid)?;Ok(self)}
}
pub(super) fn regular(path:&Path)->Result<File>{
    let before=std::fs::symlink_metadata(path)?;
    if !before.is_file() || before.file_type().is_symlink(){return Err(CoreError::Invalid)}
    #[cfg(windows)] {use std::os::windows::fs::{MetadataExt,OpenOptionsExt};if before.file_attributes()&0x400!=0{return Err(CoreError::Invalid)}let file=std::fs::OpenOptions::new().read(true).share_mode(1).custom_flags(0x0020_0000).open(path)?;if file.metadata()?.file_attributes()&0x400!=0{return Err(CoreError::Invalid)}Ok(file)}
    #[cfg(any(target_os="linux",target_os="android"))] {let fd=rustix::fs::open(path,rustix::fs::OFlags::RDONLY|rustix::fs::OFlags::NOFOLLOW|rustix::fs::OFlags::CLOEXEC,rustix::fs::Mode::empty()).map_err(std::io::Error::from)?;let file=File::from(fd);if !file.metadata()?.is_file(){return Err(CoreError::Invalid)}Ok(file)}
    #[cfg(not(any(windows,target_os="linux",target_os="android")))] {Err(CoreError::Unsupported)}
}
pub(super) fn directory(path:&str)->Result<std::path::PathBuf>{
    if path.is_empty()||path.len()>8192||path.contains('\0'){return Err(CoreError::Invalid)}
    let path=Path::new(path);if !path.is_absolute(){return Err(CoreError::Invalid)}
    let meta=std::fs::symlink_metadata(path)?;if !meta.is_dir()||meta.file_type().is_symlink(){return Err(CoreError::Invalid)}
    #[cfg(windows)] {use std::os::windows::fs::MetadataExt;if meta.file_attributes()&0x400!=0{return Err(CoreError::Invalid)}}
    Ok(path.canonicalize()?)
}
pub(super) fn hash_copy(input:&mut File,mut output:Option<&mut File>,limit:u64,cancel:&Cancellation)->Result<([u8;32],String,u64)>{
    input.seek(SeekFrom::Start(0))?;let mut blake=blake3::Hasher::new();let mut sha=Context::new(&SHA256);let mut bytes=0u64;let mut buffer=[0u8;65536];
    loop {cancel.check()?;let count=input.read(&mut buffer)?;if count==0{break}bytes=bytes.checked_add(count as u64).ok_or(CoreError::Invalid)?;if bytes>limit{return Err(CoreError::Backpressure)}blake.update(&buffer[..count]);sha.update(&buffer[..count]);if let Some(out)=output.as_mut(){out.write_all(&buffer[..count])?}}
    if bytes==0{return Err(CoreError::Invalid)}
    let sha=sha.finish();let hex=sha.as_ref().iter().map(|value|format!("{value:02x}")).collect();
    Ok((*blake.finalize().as_bytes(),hex,bytes))
}
pub(super) fn inspect(input:&mut File,cancel:&Cancellation)->Result<Metadata>{
    input.seek(SeekFrom::Start(0))?;let total=input.metadata()?.len();if !(33..=MAX_ORIGINAL).contains(&total){return Err(CoreError::Invalid)}
    let mut magic=[0;8];input.read_exact(&mut magic)?;input.seek(SeekFrom::Start(0))?;
    if magic==*b"\x89PNG\r\n\x1a\n"{png(input,total,cancel)}else if magic[..2]==[0xff,0xd8]{jpeg(input,total,cancel)}else{Err(CoreError::Unsupported)}
}
fn png(input:&mut File,total:u64,cancel:&Cancellation)->Result<Metadata>{
    input.seek(SeekFrom::Start(8))?;let mut result=None;let(mut header,mut has_data)=(0u64,false);
    loop {cancel.check()?;let mut chunk=[0;8];input.read_exact(&mut chunk)?;let len=u32::from_be_bytes(chunk[..4].try_into().map_err(|_|CoreError::Invalid)?) as u64;let kind=&chunk[4..];let start=input.stream_position()?;let end=start.checked_add(len).and_then(|v|v.checked_add(4)).ok_or(CoreError::Invalid)?;if end>total{return Err(CoreError::Invalid)}
        if result.is_none(){if kind!=b"IHDR"||len!=13{return Err(CoreError::Invalid)}let mut data=[0;13];input.read_exact(&mut data)?;if data[8..]!=[8,2,0,0,0]&&data[8..]!=[8,2,0,0,1]{return Err(CoreError::Unsupported)}result=Some(Metadata{width:u32::from_be_bytes(data[..4].try_into().map_err(|_|CoreError::Invalid)?),height:u32::from_be_bytes(data[4..8].try_into().map_err(|_|CoreError::Invalid)?),orientation:1,format:"png"});}
        else if kind==b"IHDR"{return Err(CoreError::Invalid)}
        else if [b"iCCP",b"sRGB",b"eXIf",b"gAMA",b"cHRM",b"cICP",b"tRNS",b"acTL"].iter().any(|value|kind==*value){return Err(CoreError::Unsupported)}
        else if kind==b"IDAT"{has_data=true;}
        else if kind==b"IEND"{if len!=0||!has_data||end!=total{return Err(CoreError::Invalid)}return result.ok_or(CoreError::Invalid)?.admit()}
        else if kind[0]&32==0&&kind!=b"PLTE"{return Err(CoreError::Unsupported)}
        if kind!=b"IDAT"{header=header.checked_add(len+12).ok_or(CoreError::Invalid)?;if header>MAX_HEADER{return Err(CoreError::Backpressure)}}
        input.seek(SeekFrom::Start(end))?;
    }
}
fn jpeg(input:&mut File,total:u64,cancel:&Cancellation)->Result<Metadata>{
    input.seek(SeekFrom::Start(2))?;let(mut result,mut orientation,mut exif)=(None,1,false);
    loop {cancel.check()?;if input.stream_position()?>MAX_HEADER{return Err(CoreError::Backpressure)}let mut marker=[0;2];input.read_exact(&mut marker)?;if marker[0]!=0xff{return Err(CoreError::Invalid)}while marker[1]==0xff{input.read_exact(&mut marker[1..])?;if input.stream_position()?>MAX_HEADER{return Err(CoreError::Backpressure)}}
        if marker[1]==0xda{return result.map(|mut value:Metadata|{value.orientation=orientation;value}).ok_or(CoreError::Invalid)?.admit()}
        if matches!(marker[1],0|0xd8|0xd9|0x01|0xd0..=0xd7){return Err(CoreError::Invalid)}
        let mut length=[0;2];input.read_exact(&mut length)?;let len=usize::from(u16::from_be_bytes(length));if len<2{return Err(CoreError::Invalid)}let end=input.stream_position()?.checked_add((len-2)as u64).ok_or(CoreError::Invalid)?;if end>total||end>MAX_HEADER{return Err(CoreError::Backpressure)}
        let mut payload=vec![0;len-2];input.read_exact(&mut payload)?;
        if marker[1]==0xe2&&payload.starts_with(b"ICC_PROFILE\0"){return Err(CoreError::Unsupported)}
        if marker[1]==0xe1&&payload.starts_with(b"Exif\0\0"){if exif{return Err(CoreError::Invalid)}exif=true;orientation=exif_orientation(&payload[6..])?;}
        if marker[1]==0xee&&payload.starts_with(b"Adobe"){return Err(CoreError::Unsupported)}
        if matches!(marker[1],0xc0..=0xcf)&&!matches!(marker[1],0xc4|0xc8|0xcc){
            if !matches!(marker[1],0xc0|0xc1|0xc2)||payload.len()!=15||payload[0]!=8||payload[5]!=3{return Err(CoreError::Unsupported)}
            if result.is_some(){return Err(CoreError::Invalid)}result=Some(Metadata{height:u32::from(u16::from_be_bytes([payload[1],payload[2]])),width:u32::from(u16::from_be_bytes([payload[3],payload[4]])),orientation:1,format:"jpeg"});
        }
    }
}
fn exif_orientation(bytes:&[u8])->Result<u8>{
    let little=match bytes.get(..2){Some(b"II")=>true,Some(b"MM")=>false,_=>return Err(CoreError::Invalid)};
    let word=|at:usize|->Result<u16>{let raw: [u8;2]=bytes.get(at..at+2).ok_or(CoreError::Invalid)?.try_into().map_err(|_|CoreError::Invalid)?;Ok(if little{u16::from_le_bytes(raw)}else{u16::from_be_bytes(raw)})};
    let long=|at:usize|->Result<u32>{let raw:[u8;4]=bytes.get(at..at+4).ok_or(CoreError::Invalid)?.try_into().map_err(|_|CoreError::Invalid)?;Ok(if little{u32::from_le_bytes(raw)}else{u32::from_be_bytes(raw)})};
    if word(2)?!=42{return Err(CoreError::Invalid)}let at=long(4)?as usize;let count=word(at)?as usize;if count>1024||at.checked_add(2+count*12+4).filter(|end|*end<=bytes.len()).is_none(){return Err(CoreError::Invalid)}
    let mut orientation=None;for index in 0..count{let p=at+2+index*12;if word(p)?==0x112{if orientation.is_some()||word(p+2)?!=3||long(p+4)?!=1{return Err(CoreError::Invalid)}let value=word(p+8)?;if !(1..=8).contains(&value){return Err(CoreError::Invalid)}orientation=Some(value as u8)}}Ok(orientation.unwrap_or(1))
}
