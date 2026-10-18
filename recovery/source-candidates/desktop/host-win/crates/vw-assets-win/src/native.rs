//! The only unsafe codec boundary, in the disposable Windows helper process.
//! Signatures/enums are from the pinned libvips8.18.7 C headers.
use crate::{BuildRequest,Error,LockedRuntime,Result,wide};
use std::{ffi::{CStr,CString,c_char,c_int,c_void},path::Path,ptr};
use windows_sys::Win32::{Foundation::{HMODULE,FreeLibrary},System::LibraryLoader::{GetProcAddress,LoadLibraryExW,LOAD_LIBRARY_SEARCH_DLL_LOAD_DIR,LOAD_LIBRARY_SEARCH_SYSTEM32}};

type ImagePtr=*mut c_void;
type Open=unsafe extern "C" fn(*const c_char,...)->ImagePtr;
type Inspect=unsafe extern "C" fn(ImagePtr)->c_int;
type TypeOf=unsafe extern "C" fn(ImagePtr,*const c_char)->usize;
type Unary=unsafe extern "C" fn(ImagePtr,*mut ImagePtr,...)->c_int;
type Save=unsafe extern "C" fn(ImagePtr,*const c_char,...)->c_int;
type Unref=unsafe extern "C" fn(*mut c_void);
struct Module(HMODULE);
impl Module {
    fn load(path:&Path)->Result<Self>{let path=wide(path.as_os_str())?;
        // SAFETY: exact verified runtime path retained with no-write/delete pins;
        // dependencies search only this DLL's directory and Windows System32.
        let module=unsafe{LoadLibraryExW(path.as_ptr(),ptr::null_mut(),LOAD_LIBRARY_SEARCH_DLL_LOAD_DIR|LOAD_LIBRARY_SEARCH_SYSTEM32)};
        if module.is_null(){Err(Error::WorkerUnavailable)}else{Ok(Self(module))}
    }
    fn symbol(&self,name:&CStr)->Result<unsafe extern "system" fn()->isize>{
        // SAFETY: live library and static NUL-terminated exported name.
        unsafe{GetProcAddress(self.0,name.as_ptr().cast())}.ok_or(Error::WorkerUnavailable)
    }
}
impl Drop for Module{fn drop(&mut self){ // SAFETY: unique successful LoadLibrary ref.
    unsafe{FreeLibrary(self.0);}
}}
macro_rules! function { ($module:expr,$name:expr,$kind:ty)=>{{
    let symbol=$module.symbol($name)?;
    // SAFETY: named symbol has this exact C ABI in the pinned official headers.
    unsafe{std::mem::transmute::<unsafe extern "system" fn()->isize,$kind>(symbol)}
}}; }
pub(crate) struct Api {
    open:Open,width:Inspect,height:Inspect,bands:Inspect,format:Inspect,coding:Inspect,interpretation:Inspect,orientation:Inspect,alpha:Inspect,pages:Inspect,
    typeof_:TypeOf,autorot:Unary,dzsave:Save,jpegsave:Save,unref:Unref,
    shutdown:unsafe extern "C" fn(),initialized:bool,_vips:Module,_gobject:Module,
}
impl Api {
    pub(crate) fn new(runtime:&LockedRuntime)->Result<Self>{
        let vips=Module::load(&runtime.directory.join("libvips-42.dll"))?;
        let gobject=Module::load(&runtime.directory.join("libgobject-2.0-0.dll"))?;
        let init=function!(vips,c"vips_init",unsafe extern "C" fn(*const c_char)->c_int);
        let concurrency=function!(vips,c"vips_concurrency_set",unsafe extern "C" fn(c_int));
        let cache=function!(vips,c"vips_cache_set_max_mem",unsafe extern "C" fn(usize));
        let operations=function!(vips,c"vips_cache_set_max",unsafe extern "C" fn(c_int));
        let mut api=Self{
            open:function!(vips,c"vips_image_new_from_file",Open),width:function!(vips,c"vips_image_get_width",Inspect),height:function!(vips,c"vips_image_get_height",Inspect),
            bands:function!(vips,c"vips_image_get_bands",Inspect),format:function!(vips,c"vips_image_get_format",Inspect),coding:function!(vips,c"vips_image_get_coding",Inspect),
            interpretation:function!(vips,c"vips_image_get_interpretation",Inspect),orientation:function!(vips,c"vips_image_get_orientation",Inspect),alpha:function!(vips,c"vips_image_hasalpha",Inspect),pages:function!(vips,c"vips_image_get_n_pages",Inspect),
            typeof_:function!(vips,c"vips_image_get_typeof",TypeOf),autorot:function!(vips,c"vips_autorot",Unary),dzsave:function!(vips,c"vips_dzsave",Save),jpegsave:function!(vips,c"vips_jpegsave",Save),
            unref:function!(gobject,c"g_object_unref",Unref),shutdown:function!(vips,c"vips_shutdown",unsafe extern "C" fn()),initialized:false,_vips:vips,_gobject:gobject,
        };
        // SAFETY: helper is single-request and has not started other work. These
        // bounded settings apply only to its own dynamically loaded instance.
        unsafe{if init(c"vw-pyramid-helper".as_ptr())!=0{return Err(Error::Codec);}api.initialized=true;concurrency(2);cache(128*1024*1024);operations(100);}
        Ok(api)
    }
    fn open(&self,path:&Path)->Result<Image<'_>> {
        let name=filename(path)?;
        // SAFETY: live Api, terminated filename/options; fail_on warning refuses
        // corrupt/truncated files. Input paths are fixed private owned filenames.
        let raw=unsafe{(self.open)(name.as_ptr(),c"access".as_ptr(),0 as c_int,c"fail_on".as_ptr(),3 as c_int,ptr::null::<c_char>())};
        Image::new(self,raw)
    }
    pub(crate) fn pyramid(&self,input:&Path,directory:&Path,request:&BuildRequest)->Result<()> {
        let image=self.open(input)?;
        // SAFETY: all getters access a live, fully constructed VipsImage header.
        unsafe {
            if (self.format)(image.raw)!=0 || (self.bands)(image.raw)!=3 || (self.coding)(image.raw)!=0 || (self.interpretation)(image.raw)!=22 || (self.alpha)(image.raw)!=0 || (self.pages)(image.raw)!=1 || (self.typeof_)(image.raw,c"icc-profile-data".as_ptr())!=0 {return Err(Error::Unsupported);}
            if (self.orientation)(image.raw)!=c_int::from(request.orientation){return Err(Error::Binding);}
        }
        let mut rotated=ptr::null_mut();
        // SAFETY: writable out-pointer and valid optional-argument terminator.
        if unsafe{(self.autorot)(image.raw,&mut rotated,ptr::null::<c_char>())}!=0{return Err(Error::Codec);}
        let rotated=Image::new(self,rotated)?;
        // SAFETY: rotated is a live owned image.
        if unsafe{(self.width)(rotated.raw)}!=request.width as c_int || unsafe{(self.height)(rotated.raw)}!=request.height as c_int{return Err(Error::Binding);}
        let directory=filename(directory)?;
        // SAFETY: named options/types exactly match dzsave. Full lossless pyramid
        // first; lower tiles are then independently reencoded to JPEG Q90.
        if unsafe{(self.dzsave)(rotated.raw,directory.as_ptr(),c"tile_size".as_ptr(),256 as c_int,c"overlap".as_ptr(),0 as c_int,c"suffix".as_ptr(),c".webp[lossless,effort=0,keep=none]".as_ptr(),c"depth".as_ptr(),0 as c_int,c"skip_blanks".as_ptr(),-1 as c_int,ptr::null::<c_char>())}!=0 {return Err(Error::Codec);}
        Ok(())
    }
    pub(crate) fn jpeg(&self,input:&Path,output:&Path)->Result<()> {
        let image=self.open(input)?;let output=filename(output)?;
        // SAFETY: bounded tile image; fixed JPEG options, no profile/depth/alpha
        // change can reach this call after initial opaque8 sRGB admission.
        if unsafe{(self.jpegsave)(image.raw,output.as_ptr(),c"Q".as_ptr(),90 as c_int,c"keep".as_ptr(),0 as c_int,ptr::null::<c_char>())}!=0{return Err(Error::Codec);}Ok(())
    }
}
impl Drop for Api{fn drop(&mut self){if self.initialized{// SAFETY: all borrowed Images are gone; single-request helper owns init.
    unsafe{(self.shutdown)();}
}}}
struct Image<'a>{api:&'a Api,raw:ImagePtr}
impl<'a> Image<'a>{fn new(api:&'a Api,raw:ImagePtr)->Result<Self>{if raw.is_null(){Err(Error::Codec)}else{Ok(Self{api,raw})}}}
impl Drop for Image<'_>{fn drop(&mut self){// SAFETY: unique reference from libvips constructor, Api modules still live.
    unsafe{(self.api.unref)(self.raw);}
}}
fn filename(path:&Path)->Result<CString>{let text=path.to_str().ok_or(Error::Invalid)?;if text.len()>30_000||(text.contains('[')||text.contains(']')){return Err(Error::Invalid);}CString::new(text).map_err(|_|Error::Invalid)}
