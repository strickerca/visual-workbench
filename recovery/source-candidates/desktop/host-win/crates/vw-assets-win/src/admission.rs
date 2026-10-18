use crate::{Error, Result};
use serde::{Deserialize, Serialize};
use std::io::{Read, Seek, SeekFrom};
use vw_assets::Source;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BuildRequest {
    pub original_hash: [u8;32], pub original_bytes:u64,
    pub width:u32, pub height:u32, pub orientation:u8,
    pub disk_budget:u64, pub timeout_ms:u64,
}
impl BuildRequest {
    pub fn source(&self)->Source { Source {original:self.original_hash,original_bytes:self.original_bytes,width:self.width,height:self.height,orientation:self.orientation,bit_depth:8,icc:None,display_conversion:false} }
    pub fn validate(&self)->Result<()> {
        self.source().validate().map_err(|_|Error::Limit)?;
        if !(1000..=600_000).contains(&self.timeout_ms) || self.disk_budget>16*1024*1024*1024 || estimated_disk_bytes(self)? > self.disk_budget {return Err(Error::Limit);}
        Ok(())
    }
}
/// Conservative original + decoded random-access scratch + lossless pyramid,
/// JPEG lower-level replacements and small metadata. Admission is before decode.
pub fn estimated_disk_bytes(r:&BuildRequest)->Result<u64> {
    u64::from(r.width).checked_mul(u64::from(r.height)).and_then(|n|n.checked_mul(12)).and_then(|n|n.checked_add(r.original_bytes)).and_then(|n|n.checked_add(64*1024*1024)).ok_or(Error::Limit)
}

/// Narrow PNG/JPEG header admission before libvips touches any codec. ICC/alpha
/// and orientation are checked again using actual libvips header metadata.
pub fn admit_input(file:&mut (impl Read+Seek),request:&BuildRequest)->Result<()> {
    request.validate()?;file.seek(SeekFrom::Start(0)).map_err(|_|Error::Storage)?;
    let mut first=[0;33];file.read_exact(&mut first).map_err(|_|Error::Invalid)?;
    let dimensions=if &first[..8]==b"\x89PNG\r\n\x1a\n" {
        if &first[8..16]!=b"\0\0\0\rIHDR" || first[24]!=8 || first[25]!=2 || first[26]!=0 || first[27]!=0 || first[28]>1 {return Err(Error::Unsupported);}
        (u32::from_be_bytes(first[16..20].try_into().map_err(|_|Error::Invalid)?),u32::from_be_bytes(first[20..24].try_into().map_err(|_|Error::Invalid)?))
    } else if first[..2]==[0xff,0xd8] {
        file.seek(SeekFrom::Start(2)).map_err(|_|Error::Storage)?;let mut walked=2u64;
        loop {
            if walked>16*1024*1024{return Err(Error::Limit);}
            let mut b=[0];file.read_exact(&mut b).map_err(|_|Error::Invalid)?;walked+=1;if b[0]!=0xff{return Err(Error::Invalid);}
            loop {file.read_exact(&mut b).map_err(|_|Error::Invalid)?;walked+=1;if walked>16*1024*1024{return Err(Error::Limit);}if b[0]!=0xff{break;}}
            let marker=b[0];if matches!(marker,0|0xd8|0xd9|0xda|0xd0..=0xd7){return Err(Error::Invalid);}
            let mut len=[0;2];file.read_exact(&mut len).map_err(|_|Error::Invalid)?;let len=u16::from_be_bytes(len)as usize;if len<2{return Err(Error::Invalid);}walked+=len as u64;
            if matches!(marker,0xc0|0xc1|0xc2) {
                let mut sof=[0;6];if len<8{return Err(Error::Invalid);}file.read_exact(&mut sof).map_err(|_|Error::Invalid)?;
                if sof[0]!=8 || sof[5]!=3 || len!=17{return Err(Error::Unsupported);}
                break (u32::from(u16::from_be_bytes([sof[3],sof[4]])),u32::from(u16::from_be_bytes([sof[1],sof[2]])));
            }
            if marker==0xe2 && len>=14 {let mut tag=[0;12];file.read_exact(&mut tag).map_err(|_|Error::Invalid)?;if &tag==b"ICC_PROFILE\0"{return Err(Error::Unsupported);}file.seek(SeekFrom::Current((len-14)as i64)).map_err(|_|Error::Storage)?;}
            else {file.seek(SeekFrom::Current((len-2)as i64)).map_err(|_|Error::Storage)?;}
        }
    } else {return Err(Error::Unsupported);};
    let oriented=if request.orientation>=5 {(dimensions.1,dimensions.0)}else{dimensions};
    if oriented!=(request.width,request.height){return Err(Error::Binding);}file.seek(SeekFrom::Start(0)).map_err(|_|Error::Storage)?;Ok(())
}
