use crate::{Error, Result, check};
use std::io::{BufRead,Read,Seek,SeekFrom,Write};
pub trait WriteSeek: Write+Seek {}
impl<T:Write+Seek> WriteSeek for T {}
pub(crate) struct Input<'a> { inner:std::io::Cursor<&'a[u8]>, cancel:&'a dyn Fn()->bool }
impl<'a> Input<'a> { pub fn new(bytes:&'a[u8],cancel:&'a dyn Fn()->bool)->Self {Self{inner:std::io::Cursor::new(bytes),cancel}} }
impl Read for Input<'_> { fn read(&mut self,b:&mut[u8])->std::io::Result<usize>{if (self.cancel)(){return Err(std::io::ErrorKind::Other.into());}self.inner.read(b)} }
impl BufRead for Input<'_> { fn fill_buf(&mut self)->std::io::Result<&[u8]>{if (self.cancel)(){return Err(std::io::ErrorKind::Other.into());}self.inner.fill_buf()} fn consume(&mut self,n:usize){self.inner.consume(n);} }
impl Seek for Input<'_> { fn seek(&mut self,p:SeekFrom)->std::io::Result<u64>{if (self.cancel)(){return Err(std::io::ErrorKind::Other.into());}self.inner.seek(p)} }
pub(crate) struct Output<'a> { inner:&'a mut dyn WriteSeek, cap:u64, position:u64, high:u64, cancel:&'a dyn Fn()->bool }
impl<'a> Output<'a> {
    pub fn new(inner:&'a mut dyn WriteSeek,cap:u64,cancel:&'a dyn Fn()->bool)->Result<Self>{
        if inner.stream_position().map_err(|e|Error::Io(e.kind()))? !=0 {return Err(Error::Invalid);}
        if inner.seek(SeekFrom::End(0)).map_err(|e|Error::Io(e.kind()))? !=0 {return Err(Error::Invalid);}
        inner.seek(SeekFrom::Start(0)).map_err(|e|Error::Io(e.kind()))?;
        Ok(Self{inner,cap,position:0,high:0,cancel})
    }
    pub fn bytes(&mut self,b:&[u8])->Result<()> {check(self.cancel)?;let end=self.position.checked_add(b.len()as u64).ok_or(Error::Limit)?;if end>self.cap{return Err(Error::Limit);}self.inner.write_all(b).map_err(|e|Error::Io(e.kind()))?;self.position=end;self.high=self.high.max(end);Ok(())}
    pub fn finish(self)->Result<u64>{check(self.cancel)?;self.inner.flush().map_err(|e|Error::Io(e.kind()))?;Ok(self.high)}
}
