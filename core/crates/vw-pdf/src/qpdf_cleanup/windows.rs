use crate::pc_content::{MAX_CONTENT, admit_source_content};
use crate::pc_redaction::{
    CleanedPcCopy, PcRedactionError, PcRedactionPlan, PcRedactionResult, PcSourceAdmission,
    UnverifiedPcCopy,
};
use crate::{PdfCancellation, PdfError, PdfLimits};
use std::{
    ffi::{CStr, c_char, c_int, c_uint, c_void},
    marker::PhantomData,
    os::windows::ffi::OsStrExt,
    path::Path,
    ptr,
    rc::Rc,
};
use windows_sys::Win32::{
    Foundation::{FreeLibrary, HMODULE},
    System::LibraryLoader::{
        GetProcAddress, LOAD_LIBRARY_SEARCH_DLL_LOAD_DIR, LOAD_LIBRARY_SEARCH_SYSTEM32,
        LoadLibraryExW,
    },
};

type Data = *mut c_void;
type Oh = c_uint;
struct Library(HMODULE);
impl Drop for Library {
    fn drop(&mut self) {
        // SAFETY: all contexts/functions have dropped before the last library
        // field. This wrapper owns the exact successful LoadLibraryExW handle.
        unsafe {
            FreeLibrary(self.0);
        }
    }
}
struct Functions {
    version: unsafe extern "C" fn() -> *const c_char,
    init: unsafe extern "C" fn() -> Data,
    cleanup: unsafe extern "C" fn(*mut Data),
    silence: unsafe extern "C" fn(Data),
    suppress: unsafe extern "C" fn(Data, c_int),
    recovery: unsafe extern "C" fn(Data, c_int),
    error: unsafe extern "C" fn(Data) -> c_int,
    warnings: unsafe extern "C" fn(Data) -> c_int,
    read: unsafe extern "C" fn(Data, *const c_char, *const c_char, u64, *const c_char) -> c_int,
    check: unsafe extern "C" fn(Data) -> c_int,
    page_count: unsafe extern "C" fn(Data) -> c_int,
    page: unsafe extern "C" fn(Data, usize) -> Oh,
    root: unsafe extern "C" fn(Data) -> Oh,
    trailer: unsafe extern "C" fn(Data) -> Oh,
    get: unsafe extern "C" fn(Data, Oh, *const c_char) -> Oh,
    remove: unsafe extern "C" fn(Data, Oh, *const c_char),
    null: unsafe extern "C" fn(Data, Oh) -> c_int,
    dict: unsafe extern "C" fn(Data, Oh) -> c_int,
    is_stream: unsafe extern "C" fn(Data, Oh) -> c_int,
    stream_dict: unsafe extern "C" fn(Data, Oh) -> Oh,
    name_equal: unsafe extern "C" fn(Data, Oh, *const c_char) -> c_int,
    number: unsafe extern "C" fn(Data, Oh, *mut f64) -> c_int,
    array_count: unsafe extern "C" fn(Data, Oh) -> c_int,
    array_item: unsafe extern "C" fn(Data, Oh, c_int) -> Oh,
    object_id: unsafe extern "C" fn(Data, Oh) -> c_int,
    generation: unsafe extern "C" fn(Data, Oh) -> c_int,
    begin_keys: unsafe extern "C" fn(Data, Oh),
    more_keys: unsafe extern "C" fn(Data) -> c_int,
    next_key: unsafe extern "C" fn(Data) -> *const c_char,
    content: unsafe extern "C" fn(Data, Oh, *mut *mut u8, *mut usize) -> c_int,
    stream: unsafe extern "C" fn(Data, Oh, c_int, *mut c_int, *mut *mut u8, *mut usize) -> c_int,
    free: unsafe extern "C" fn(*mut *mut u8),
    release: unsafe extern "C" fn(Data),
    init_write: unsafe extern "C" fn(Data) -> c_int,
    object_streams: unsafe extern "C" fn(Data, c_int),
    stream_mode: unsafe extern "C" fn(Data, c_int),
    compress: unsafe extern "C" fn(Data, c_int),
    preserve_orphans: unsafe extern "C" fn(Data, c_int),
    preserve_encryption: unsafe extern "C" fn(Data, c_int),
    normalize: unsafe extern "C" fn(Data, c_int),
    deterministic_id: unsafe extern "C" fn(Data, c_int),
    write: unsafe extern "C" fn(Data) -> c_int,
    buffer_length: unsafe extern "C" fn(Data) -> usize,
    buffer: unsafe extern "C" fn(Data) -> *const u8,
}
pub struct QpdfRuntime {
    functions: Functions,
    limits: PdfLimits,
    _thread: PhantomData<Rc<()>>,
    _library: Library,
}
impl QpdfRuntime {
    /// # Safety
    /// Caller must hold exact file/ancestor pins for official qpdf12.4.2 x64
    /// qpdf30.dll and all non-system dependencies for the lifetime of this
    /// worker. The current process must already be zero-capability LPAC with
    /// hard memory/time/Job limits and constrained DLL search. No system qpdf
    /// fallback or executable helper is accepted. Native allocations/calls are
    /// capped/interrupted by that OS boundary, not by Rust buffer admission.
    pub unsafe fn from_verified_library(path: &Path, limits: PdfLimits) -> PcRedactionResult<Self> {
        let limits = limits.validate()?;
        if !path.is_absolute() || path.file_name().is_none_or(|name| name != "qpdf30.dll") {
            return Err(PcRedactionError::CleanupUnavailable);
        }
        let mut encoded: Vec<u16> = path.as_os_str().encode_wide().collect();
        if encoded.contains(&0) {
            return Err(PcRedactionError::CleanupUnavailable);
        }
        encoded.push(0);
        // SAFETY: caller retains exact source-bound pins. Only this flat pinned
        // directory and trusted System32 can resolve dependent DLLs.
        let library = Library(unsafe {
            LoadLibraryExW(
                encoded.as_ptr(),
                ptr::null_mut(),
                LOAD_LIBRARY_SEARCH_DLL_LOAD_DIR | LOAD_LIBRARY_SEARCH_SYSTEM32,
            )
        });
        if library.0.is_null() {
            return Err(PcRedactionError::CleanupUnavailable);
        }
        macro_rules! load {
            ($name:literal, $ty:ty) => {{
                // SAFETY: fixed NUL-terminated publisher symbol and retained
                // library; caller proved exact C ABI12.4.2 x64 artifact.
                let address = unsafe { GetProcAddress(library.0, concat!($name, "\0").as_ptr()) }
                    .ok_or(PcRedactionError::CleanupUnavailable)?;
                // SAFETY: concrete function-pointer signature is copied from
                // qpdf12.4.2 qpdf-c.h; both pointer types have one address.
                unsafe { std::mem::transmute::<unsafe extern "system" fn() -> isize, $ty>(address) }
            }};
        }
        let functions = Functions {
            version: load!(
                "qpdf_get_qpdf_version",
                unsafe extern "C" fn() -> *const c_char
            ),
            init: load!("qpdf_init", unsafe extern "C" fn() -> Data),
            cleanup: load!("qpdf_cleanup", unsafe extern "C" fn(*mut Data)),
            silence: load!("qpdf_silence_errors", unsafe extern "C" fn(Data)),
            suppress: load!(
                "qpdf_set_suppress_warnings",
                unsafe extern "C" fn(Data, c_int)
            ),
            recovery: load!(
                "qpdf_set_attempt_recovery",
                unsafe extern "C" fn(Data, c_int)
            ),
            error: load!("qpdf_has_error", unsafe extern "C" fn(Data) -> c_int),
            warnings: load!("qpdf_more_warnings", unsafe extern "C" fn(Data) -> c_int),
            read: load!(
                "qpdf_read_memory",
                unsafe extern "C" fn(
                    Data,
                    *const c_char,
                    *const c_char,
                    u64,
                    *const c_char,
                ) -> c_int
            ),
            check: load!("qpdf_check_pdf", unsafe extern "C" fn(Data) -> c_int),
            page_count: load!("qpdf_get_num_pages", unsafe extern "C" fn(Data) -> c_int),
            page: load!("qpdf_get_page_n", unsafe extern "C" fn(Data, usize) -> Oh),
            root: load!("qpdf_get_root", unsafe extern "C" fn(Data) -> Oh),
            trailer: load!("qpdf_get_trailer", unsafe extern "C" fn(Data) -> Oh),
            get: load!(
                "qpdf_oh_get_key",
                unsafe extern "C" fn(Data, Oh, *const c_char) -> Oh
            ),
            remove: load!(
                "qpdf_oh_remove_key",
                unsafe extern "C" fn(Data, Oh, *const c_char)
            ),
            null: load!("qpdf_oh_is_null", unsafe extern "C" fn(Data, Oh) -> c_int),
            dict: load!(
                "qpdf_oh_is_dictionary",
                unsafe extern "C" fn(Data, Oh) -> c_int
            ),
            is_stream: load!("qpdf_oh_is_stream", unsafe extern "C" fn(Data, Oh) -> c_int),
            stream_dict: load!("qpdf_oh_get_dict", unsafe extern "C" fn(Data, Oh) -> Oh),
            name_equal: load!(
                "qpdf_oh_is_name_and_equals",
                unsafe extern "C" fn(Data, Oh, *const c_char) -> c_int
            ),
            number: load!(
                "qpdf_oh_get_value_as_number",
                unsafe extern "C" fn(Data, Oh, *mut f64) -> c_int
            ),
            array_count: load!(
                "qpdf_oh_get_array_n_items",
                unsafe extern "C" fn(Data, Oh) -> c_int
            ),
            array_item: load!(
                "qpdf_oh_get_array_item",
                unsafe extern "C" fn(Data, Oh, c_int) -> Oh
            ),
            object_id: load!(
                "qpdf_oh_get_object_id",
                unsafe extern "C" fn(Data, Oh) -> c_int
            ),
            generation: load!(
                "qpdf_oh_get_generation",
                unsafe extern "C" fn(Data, Oh) -> c_int
            ),
            begin_keys: load!(
                "qpdf_oh_begin_dict_key_iter",
                unsafe extern "C" fn(Data, Oh)
            ),
            more_keys: load!(
                "qpdf_oh_dict_more_keys",
                unsafe extern "C" fn(Data) -> c_int
            ),
            next_key: load!(
                "qpdf_oh_dict_next_key",
                unsafe extern "C" fn(Data) -> *const c_char
            ),
            content: load!(
                "qpdf_oh_get_page_content_data",
                unsafe extern "C" fn(Data, Oh, *mut *mut u8, *mut usize) -> c_int
            ),
            stream: load!(
                "qpdf_oh_get_stream_data",
                unsafe extern "C" fn(
                    Data,
                    Oh,
                    c_int,
                    *mut c_int,
                    *mut *mut u8,
                    *mut usize,
                ) -> c_int
            ),
            free: load!("qpdf_oh_free_buffer", unsafe extern "C" fn(*mut *mut u8)),
            release: load!("qpdf_oh_release_all", unsafe extern "C" fn(Data)),
            init_write: load!(
                "qpdf_init_write_memory",
                unsafe extern "C" fn(Data) -> c_int
            ),
            object_streams: load!(
                "qpdf_set_object_stream_mode",
                unsafe extern "C" fn(Data, c_int)
            ),
            stream_mode: load!(
                "qpdf_set_stream_data_mode",
                unsafe extern "C" fn(Data, c_int)
            ),
            compress: load!(
                "qpdf_set_compress_streams",
                unsafe extern "C" fn(Data, c_int)
            ),
            preserve_orphans: load!(
                "qpdf_set_preserve_unreferenced_objects",
                unsafe extern "C" fn(Data, c_int)
            ),
            preserve_encryption: load!(
                "qpdf_set_preserve_encryption",
                unsafe extern "C" fn(Data, c_int)
            ),
            normalize: load!(
                "qpdf_set_content_normalization",
                unsafe extern "C" fn(Data, c_int)
            ),
            deterministic_id: load!(
                "qpdf_set_deterministic_ID",
                unsafe extern "C" fn(Data, c_int)
            ),
            write: load!("qpdf_write", unsafe extern "C" fn(Data) -> c_int),
            buffer_length: load!(
                "qpdf_get_buffer_length",
                unsafe extern "C" fn(Data) -> usize
            ),
            buffer: load!("qpdf_get_buffer", unsafe extern "C" fn(Data) -> *const u8),
        };
        // SAFETY: verified API returns a static NUL-terminated publisher version.
        let version = unsafe { (functions.version)() };
        // SAFETY: exact verified C ABI; this owner retains the live context
        // and its scalar/object outputs. Every call is followed by ensure().
        if version.is_null() || unsafe { CStr::from_ptr(version) }.to_bytes() != b"12.4.2" {
            return Err(PcRedactionError::CleanupUnavailable);
        }
        Ok(Self {
            functions,
            limits,
            _thread: PhantomData,
            _library: library,
        })
    }

    pub fn admit_source(
        &self,
        source: &[u8],
        password: Option<&str>,
        cancel: &PdfCancellation,
    ) -> PcRedactionResult<PcSourceAdmission> {
        if password.is_some_and(|value| value.len() > 1024 || value.as_bytes().contains(&0)) {
            return Err(PcRedactionError::Invalid);
        }
        self.limits.admit_input(source.len(), source.len())?;
        cancel.check()?;
        self.limits
            .admit_memory(source.len() as u64, MAX_CONTENT as u64)?;
        let context = Context::read(&self.functions, source, password)?;
        let count = context.pages()?;
        if count > self.limits.pages {
            return Err(PcRedactionError::Limit);
        }
        let mut decoded = 0usize;
        for index in 0..count {
            cancel.check()?;
            let page = context.page(index)?;
            if !context.is_null(context.get(page, c"/Group")?)? {
                return Err(PcRedactionError::UnsupportedContent);
            }
            let mut ancestor = page;
            let mut found = false;
            for _ in 0..self.limits.pages {
                let resources = context.get(ancestor, c"/Resources")?;
                if !context.is_null(resources)? {
                    context.deny_type3_fonts(resources)?;
                    let colors = context.get(resources, c"/ColorSpace")?;
                    if !context.is_null(colors)? {
                        for name in [c"/DefaultRGB", c"/DefaultGray", c"/DefaultCMYK"] {
                            if !context.is_null(context.get(colors, name)?)? {
                                return Err(PcRedactionError::UnsupportedContent);
                            }
                        }
                    }
                    found = true;
                    break;
                }
                ancestor = context.get(ancestor, c"/Parent")?;
                if context.is_null(ancestor)? {
                    found = true;
                    break;
                }
            }
            if !found {
                return Err(PcRedactionError::Limit);
            }
            let content = context.content(page, MAX_CONTENT)?;
            decoded = decoded
                .checked_add(content.bytes().len())
                .ok_or(PcRedactionError::Limit)?;
            if decoded > MAX_CONTENT {
                return Err(PcRedactionError::Limit);
            }
            admit_source_content(content.bytes())?;
            drop(content);
            context.release_all()?;
        }
        cancel.check()?;
        Ok(PcSourceAdmission {
            source_blake3: blake3::hash(source).to_hex().to_string(),
            pages: count,
        })
    }

    /// Consumes only an exact private fresh reconstruction. All native object
    /// ownership is joined before returning either bytes or an error.
    pub fn rewrite_and_audit(
        &self,
        plan: &PcRedactionPlan,
        intermediate: UnverifiedPcCopy,
        generated: &[u8],
        cancel: &PdfCancellation,
    ) -> PcRedactionResult<(Vec<u8>, CleanedPcCopy)> {
        cancel.check()?;
        if intermediate.source_blake3 != plan.source_blake3
            || intermediate.plan_blake3 != plan.plan_blake3
            || intermediate.bytes != generated.len() as u64
            || intermediate.output_blake3 != blake3::hash(generated).to_hex().as_str()
        {
            return Err(PcRedactionError::Verification);
        }
        self.audit(plan, &intermediate, generated, cancel)?;
        let context = Context::read(&self.functions, generated, None)?;
        // SAFETY: live context. The generated dictionaries contain no original
        // metadata/actions; remove defensive keys before a fresh full rewrite.
        let root = unsafe { (self.functions.root)(context.raw) };
        context.ensure()?;
        // SAFETY: exact verified C ABI; this owner retains the live context
        // and its scalar/object outputs. Every call is followed by ensure().
        let trailer = unsafe { (self.functions.trailer)(context.raw) };
        context.ensure()?;
        for name in [
            c"/Metadata",
            c"/Names",
            c"/OpenAction",
            c"/AA",
            c"/AcroForm",
            c"/Outlines",
            c"/StructTreeRoot",
            c"/PieceInfo",
        ] {
            // SAFETY: live root dictionary and constant canonical key.
            unsafe { (self.functions.remove)(context.raw, root, name.as_ptr()) };
            context.ensure()?;
        }
        for name in [c"/Info", c"/Encrypt", c"/Prev", c"/ID"] {
            // SAFETY: live trailer dictionary and constant canonical key.
            unsafe { (self.functions.remove)(context.raw, trailer, name.as_ptr()) };
            context.ensure()?;
        }
        // SAFETY: live context and memory-only writer; no filesystem output path.
        if unsafe { (self.functions.init_write)(context.raw) } != 0 {
            return Err(PcRedactionError::Cleanup);
        }
        context.ensure()?;
        for (function, value) in [
            (self.functions.object_streams, 0),
            (self.functions.stream_mode, 0),
            (self.functions.compress, 0),
            (self.functions.preserve_orphans, 0),
            (self.functions.preserve_encryption, 0),
            (self.functions.normalize, 0),
            (self.functions.deterministic_id, 1),
        ] {
            // SAFETY: only documented writer parameters after init_write; 0
            // disables object streams/compression/orphans/incremental history.
            unsafe { function(context.raw, value) };
            context.ensure()?;
        }
        cancel.check()?;
        // SAFETY: synchronous writer; native buffers are context-owned until
        // cleanup. The external hard Job bound covers the native allocation.
        if unsafe { (self.functions.write)(context.raw) } != 0 {
            return Err(PcRedactionError::Cleanup);
        }
        context.ensure()?;
        // SAFETY: exact verified C ABI; this owner retains the live context
        // and its scalar/object outputs. Every call is followed by ensure().
        let length = unsafe { (self.functions.buffer_length)(context.raw) };
        context.ensure()?;
        if length < 8 || length as u64 > self.limits.output_bytes.min(self.limits.input_bytes) {
            return Err(PcRedactionError::Limit);
        }
        self.limits.admit_memory(
            generated.len() as u64,
            (length as u64)
                .checked_mul(2)
                .and_then(|n| n.checked_add(32 * 1024 * 1024))
                .ok_or(PcRedactionError::Limit)?,
        )?;
        // SAFETY: exact verified C ABI; this owner retains the live context
        // and its scalar/object outputs. Every call is followed by ensure().
        let pointer = unsafe { (self.functions.buffer)(context.raw) };
        context.ensure()?;
        if pointer.is_null() {
            return Err(PcRedactionError::Cleanup);
        }
        let mut output = Vec::new();
        output
            .try_reserve_exact(length)
            .map_err(|_| PdfError::Allocation)?;
        // SAFETY: qpdf's documented buffer is length-bounded and remains alive
        // through the copy. It is freed with context before independent reopen.
        output.extend_from_slice(unsafe { std::slice::from_raw_parts(pointer, length) });
        drop(context);
        cancel.check()?;
        self.audit(plan, &intermediate, &output, cancel)?;
        let receipt = CleanedPcCopy {
            source_blake3: plan.source_blake3.clone(),
            plan_blake3: plan.plan_blake3.clone(),
            output_blake3: blake3::hash(&output).to_hex().to_string(),
            bytes: output.len() as u64,
        };
        Ok((output, receipt))
    }

    fn audit(
        &self,
        plan: &PcRedactionPlan,
        intermediate: &UnverifiedPcCopy,
        bytes: &[u8],
        cancel: &PdfCancellation,
    ) -> PcRedactionResult<()> {
        self.limits
            .admit_memory(bytes.len() as u64, 64 * 1024 * 1024)?;
        let context = Context::read(&self.functions, bytes, None)?;
        if context.pages()? as usize != plan.pages.len() {
            return Err(PcRedactionError::Verification);
        }
        // SAFETY: live reparsed context; handles are opaque context-owned tokens.
        let root = unsafe { (self.functions.root)(context.raw) };
        context.ensure()?;
        context.keys(root, &[b"/Type", b"/Pages"])?;
        context.name(context.get(root, c"/Type")?, c"/Catalog")?;
        let tree = context.get(root, c"/Pages")?;
        context.keys(tree, &[b"/Type", b"/Count", b"/Kids"])?;
        context.name(context.get(tree, c"/Type")?, c"/Pages")?;
        context.equal_number(context.get(tree, c"/Count")?, plan.pages.len() as f64)?;
        let kids = context.get(tree, c"/Kids")?;
        // SAFETY: exact verified C ABI; this owner retains the live context
        // and its scalar/object outputs. Every call is followed by ensure().
        let count = unsafe { (self.functions.array_count)(context.raw, kids) };
        context.ensure()?;
        if count != plan.pages.len() as i32 {
            return Err(PcRedactionError::Verification);
        }
        // SAFETY: exact verified C ABI; this owner retains the live context
        // and its scalar/object outputs. Every call is followed by ensure().
        let trailer = unsafe { (self.functions.trailer)(context.raw) };
        context.ensure()?;
        context.keys_subset(trailer, &[b"/Size", b"/Root", b"/ID"])?;
        context.same_object(context.get(trailer, c"/Root")?, root)?;
        // qpdf may assign an indirect length object to each stream. Audit its
        // actual bounded ID census rather than assuming a particular numbering.
        let maximum_objects = 2 + plan.pages.len() * 3 + plan.work().tiles as usize * 2;
        let size = context.number(context.get(trailer, c"/Size")?)?;
        if size.fract() != 0.0 || size < 3.0 || size > (maximum_objects + 1) as f64 {
            return Err(PcRedactionError::Verification);
        }
        let mut seen = Vec::new();
        seen.try_reserve_exact(size as usize)
            .map_err(|_| PdfError::Allocation)?;
        seen.resize(size as usize, false);
        seen[0] = true;
        context.record_object(root, &mut seen, true)?;
        context.record_object(tree, &mut seen, true)?;
        let mut image_index = 0usize;
        for (index, expected) in plan.pages.iter().enumerate() {
            cancel.check()?;
            let page = context.page(index as u32)?;
            // SAFETY: exact verified C ABI; this owner retains the live context
            // and its scalar/object outputs. Every call is followed by ensure().
            let kid = unsafe { (self.functions.array_item)(context.raw, kids, index as i32) };
            context.ensure()?;
            context.same_object(kid, page)?;
            context.record_object(page, &mut seen, true)?;
            context.keys(
                page,
                &[
                    b"/Type",
                    b"/Parent",
                    b"/MediaBox",
                    b"/Resources",
                    b"/Contents",
                ],
            )?;
            context.name(context.get(page, c"/Type")?, c"/Page")?;
            context.same_object(context.get(page, c"/Parent")?, tree)?;
            let box_array = context.get(page, c"/MediaBox")?;
            // SAFETY: exact verified C ABI; this owner retains the live context
            // and its scalar/object outputs. Every call is followed by ensure().
            let box_count = unsafe { (self.functions.array_count)(context.raw, box_array) };
            context.ensure()?;
            if box_count != 4 {
                return Err(PcRedactionError::Verification);
            }
            for (ordinal, number) in [
                0.0,
                0.0,
                expected.geometry.width(),
                expected.geometry.height(),
            ]
            .into_iter()
            .enumerate()
            {
                // SAFETY: exact verified C ABI; this owner retains the live context
                // and its scalar/object outputs. Every call is followed by ensure().
                let token =
                    unsafe { (self.functions.array_item)(context.raw, box_array, ordinal as i32) };
                context.ensure()?;
                context.equal_number(token, number)?;
            }
            let resources = context.get(page, c"/Resources")?;
            context.keys(resources, &[b"/XObject"])?;
            let images = context.get(resources, c"/XObject")?;
            let names: Vec<_> = (0..expected.tiles.len())
                .map(|n| format!("/T{n}").into_bytes())
                .collect();
            let name_views: Vec<_> = names.iter().map(Vec::as_slice).collect();
            context.keys(images, &name_views)?;
            let contents = context.get(page, c"/Contents")?;
            context.record_object(contents, &mut seen, true)?;
            let dictionary = context.stream_keys(contents, &[b"/Length"])?;
            let content = context.content(page, 32 * 1024 * 1024)?;
            if content.bytes() != plan.page_content(expected)? {
                return Err(PcRedactionError::Verification);
            }
            let length = context.get(dictionary, c"/Length")?;
            context.equal_number(length, content.bytes().len() as f64)?;
            context.record_object(length, &mut seen, false)?;
            drop(content);
            for (ordinal, tile) in expected.tiles.iter().enumerate() {
                cancel.check()?;
                let name = std::ffi::CString::new(format!("/T{ordinal}"))
                    .map_err(|_| PcRedactionError::Invalid)?;
                let image = context.get(images, &name)?;
                context.record_object(image, &mut seen, true)?;
                let dictionary = context.stream_keys(
                    image,
                    &[
                        b"/Type",
                        b"/Subtype",
                        b"/Width",
                        b"/Height",
                        b"/ColorSpace",
                        b"/BitsPerComponent",
                        b"/Length",
                    ],
                )?;
                context.name(context.get(dictionary, c"/Type")?, c"/XObject")?;
                context.name(context.get(dictionary, c"/Subtype")?, c"/Image")?;
                context.name(context.get(dictionary, c"/ColorSpace")?, c"/DeviceRGB")?;
                context.equal_number(context.get(dictionary, c"/Width")?, f64::from(tile.width))?;
                context
                    .equal_number(context.get(dictionary, c"/Height")?, f64::from(tile.height))?;
                context.equal_number(context.get(dictionary, c"/BitsPerComponent")?, 8.0)?;
                let length = tile.width as usize * tile.height as usize * 3;
                let length_token = context.get(dictionary, c"/Length")?;
                context.equal_number(length_token, length as f64)?;
                context.record_object(length_token, &mut seen, false)?;
                let image = context.stream(image, length)?;
                if image.bytes().len() != length
                    || intermediate.tile_hashes.get(image_index)
                        != Some(blake3::hash(image.bytes()).as_bytes())
                {
                    return Err(PcRedactionError::Verification);
                }
                image_index += 1;
            }
        }
        if image_index != intermediate.tile_hashes.len() || seen.iter().any(|value| !value) {
            return Err(PcRedactionError::Verification);
        }
        cancel.check()?;
        Ok(())
    }
}

struct Context<'a> {
    api: &'a Functions,
    raw: Data,
    _source: &'a [u8],
}
impl<'a> Context<'a> {
    fn read(
        api: &'a Functions,
        source: &'a [u8],
        password: Option<&str>,
    ) -> PcRedactionResult<Self> {
        // SAFETY: exact verified ABI; qpdf_init creates sole context ownership.
        let raw = unsafe { (api.init)() };
        if raw.is_null() {
            return Err(PcRedactionError::Cleanup);
        }
        let context = Self {
            api,
            raw,
            _source: source,
        };
        // SAFETY: live newly-created context; prevent all source-derived logs.
        unsafe {
            (api.silence)(raw);
            (api.suppress)(raw, 1);
            (api.recovery)(raw, 0);
        }
        context.ensure()?;
        let password = password
            .map(std::ffi::CString::new)
            .transpose()
            .map_err(|_| PcRedactionError::Invalid)?;
        // SAFETY: the context retains the exact source borrow until cleanup,
        // including deferred stream reads. No assumption about a native input
        // copy is needed. Password stays live through read_memory, whose API
        // parses the password synchronously; the description is constant.
        if unsafe {
            (api.read)(
                raw,
                c"private-generated-document".as_ptr(),
                source.as_ptr().cast(),
                source.len() as u64,
                password.as_ref().map_or(ptr::null(), |p| p.as_ptr()),
            )
        } != 0
        {
            return Err(PcRedactionError::Cleanup);
        }
        context.ensure()?;
        // SAFETY: exact verified C ABI; this owner retains the live context
        // and its scalar/object outputs. Every call is followed by ensure().
        if unsafe { (api.check)(raw) } != 0 {
            return Err(PcRedactionError::Cleanup);
        }
        context.ensure()?;
        Ok(context)
    }
    fn ensure(&self) -> PcRedactionResult<()> {
        // SAFETY: context is live, on its one owner thread; never retrieve or
        // print native error strings. Any warning/refused recovery is an error.
        if unsafe { (self.api.error)(self.raw) } != 0
            || unsafe { (self.api.warnings)(self.raw) } != 0
        {
            Err(PcRedactionError::Cleanup)
        } else {
            Ok(())
        }
    }
    fn pages(&self) -> PcRedactionResult<u32> {
        // SAFETY: exact verified C ABI; this owner retains the live context
        // and its scalar/object outputs. Every call is followed by ensure().
        let value = unsafe { (self.api.page_count)(self.raw) };
        self.ensure()?;
        if !(1..=1000).contains(&value) {
            return Err(PcRedactionError::Limit);
        }
        Ok(value as u32)
    }
    // SAFETY: exact verified C ABI; this owner retains the live context
    // and its scalar/object outputs. Every call is followed by ensure().
    fn page(&self, index: u32) -> PcRedactionResult<Oh> {
        let value = unsafe { (self.api.page)(self.raw, index as usize) };
        self.ensure()?;
        if value == 0 {
            return Err(PcRedactionError::Cleanup);
        }
        Ok(value)
    }
    // SAFETY: exact verified C ABI; this owner retains the live context
    // and its scalar/object outputs. Every call is followed by ensure().
    fn get(&self, parent: Oh, key: &CStr) -> PcRedactionResult<Oh> {
        let value = unsafe { (self.api.get)(self.raw, parent, key.as_ptr()) };
        self.ensure()?;
        if value == 0 {
            return Err(PcRedactionError::Cleanup);
        }
        Ok(value)
    }
    // SAFETY: exact verified C ABI; this owner retains the live context
    // and its scalar/object outputs. Every call is followed by ensure().
    fn is_null(&self, value: Oh) -> PcRedactionResult<bool> {
        let null = unsafe { (self.api.null)(self.raw, value) } != 0;
        self.ensure()?;
        Ok(null)
    }
    // SAFETY: exact verified C ABI; this owner retains the live context
    // and its scalar/object outputs. Every call is followed by ensure().
    fn name(&self, value: Oh, name: &CStr) -> PcRedactionResult<()> {
        let equal = unsafe { (self.api.name_equal)(self.raw, value, name.as_ptr()) } != 0;
        self.ensure()?;
        if equal {
            Ok(())
        } else {
            Err(PcRedactionError::Verification)
        }
    }
    fn equal_number(&self, value: Oh, expected: f64) -> PcRedactionResult<()> {
        let number = self.number(value)?;
        if (number - expected).abs() <= 0.00000001 {
            Ok(())
        } else {
            Err(PcRedactionError::Verification)
        }
    }
    fn number(&self, value: Oh) -> PcRedactionResult<f64> {
        // SAFETY: exact verified C ABI; this owner retains the live context
        // and its scalar/object outputs. Every call is followed by ensure().
        let mut number = 0.0;
        let ok = unsafe { (self.api.number)(self.raw, value, &mut number) };
        self.ensure()?;
        if ok != 0 && number.is_finite() {
            Ok(number)
        } else {
            Err(PcRedactionError::Verification)
        }
    }
    fn record_object(&self, value: Oh, seen: &mut [bool], required: bool) -> PcRedactionResult<()> {
        // SAFETY: live context-owned handle; only scalar IDs leave the call.
        let id = unsafe { (self.api.object_id)(self.raw, value) };
        self.ensure()?;
        if id == 0 && !required {
            return Ok(());
        }
        // SAFETY: same live object handle; generated objects use generation0.
        let generation = unsafe { (self.api.generation)(self.raw, value) };
        self.ensure()?;
        if id <= 0 || generation != 0 {
            return Err(PcRedactionError::Verification);
        }
        let slot = seen
            .get_mut(id as usize)
            .ok_or(PcRedactionError::Verification)?;
        if *slot {
            return Err(PcRedactionError::Verification);
        }
        *slot = true;
        Ok(())
    }
    fn deny_type3_fonts(&self, resources: Oh) -> PcRedactionResult<()> {
        let fonts = self.get(resources, c"/Font")?;
        if self.is_null(fonts)? {
            return Ok(());
        }
        // SAFETY: live private resources; one dictionary iteration at a time.
        if unsafe { (self.api.dict)(self.raw, fonts) } == 0 {
            return Err(PcRedactionError::UnsupportedContent);
        }
        self.ensure()?;
        // SAFETY: live dictionary; all returned keys are copied before any next
        // iteration. Never start another iterator until this loop completes.
        unsafe { (self.api.begin_keys)(self.raw, fonts) };
        self.ensure()?;
        let mut count = 0usize;
        loop {
            // SAFETY: live single iterator; scalar result only.
            let more = unsafe { (self.api.more_keys)(self.raw) } != 0;
            self.ensure()?;
            if !more {
                break;
            }
            count += 1;
            if count > 4096 {
                return Err(PcRedactionError::Limit);
            }
            // SAFETY: canonical key lifetime lasts until next next_key call.
            let key = unsafe { (self.api.next_key)(self.raw) };
            self.ensure()?;
            if key.is_null() {
                return Err(PcRedactionError::UnsupportedContent);
            }
            // SAFETY: publisher ABI guarantees a NUL-terminated canonical name.
            let key = unsafe { CStr::from_ptr(key) };
            if key.to_bytes().len() > 127 {
                return Err(PcRedactionError::UnsupportedContent);
            }
            let font = self.get(fonts, key)?;
            let subtype = self.get(font, c"/Subtype")?;
            // SAFETY: immutable admitted handle and constant canonical name.
            let type3 =
                unsafe { (self.api.name_equal)(self.raw, subtype, c"/Type3".as_ptr()) } != 0;
            self.ensure()?;
            if type3 {
                return Err(PcRedactionError::UnsupportedContent);
            }
        }
        Ok(())
    }
    fn same_object(&self, left: Oh, right: Oh) -> PcRedactionResult<()> {
        // SAFETY: exact verified C ABI; this owner retains the live context
        // and its scalar/object outputs. Every call is followed by ensure().
        let a = unsafe { (self.api.object_id)(self.raw, left) };
        self.ensure()?;
        // SAFETY: exact verified C ABI; this owner retains the live context
        // and its scalar/object outputs. Every call is followed by ensure().
        let b = unsafe { (self.api.object_id)(self.raw, right) };
        self.ensure()?;
        if a > 0 && a == b {
            Ok(())
        } else {
            Err(PcRedactionError::Verification)
        }
    }
    fn keys(&self, dictionary: Oh, allowed: &[&[u8]]) -> PcRedactionResult<()> {
        let count = self.keys_subset(dictionary, allowed)?;
        if count == allowed.len() {
            Ok(())
        } else {
            Err(PcRedactionError::Verification)
        }
    }
    fn keys_subset(&self, dictionary: Oh, allowed: &[&[u8]]) -> PcRedactionResult<usize> {
        // SAFETY: exact verified C ABI; this owner retains the live context
        // and its scalar/object outputs. Every call is followed by ensure().
        if unsafe { (self.api.dict)(self.raw, dictionary) } == 0 {
            return Err(PcRedactionError::Verification);
        }
        self.ensure()?;
        // SAFETY: exact verified C ABI; this owner retains the live context
        // and its scalar/object outputs. Every call is followed by ensure().
        unsafe { (self.api.begin_keys)(self.raw, dictionary) };
        self.ensure()?;
        let mut count = 0usize;
        loop {
            // SAFETY: exact verified C ABI; this owner retains the live context
            // and its scalar/object outputs. Every call is followed by ensure().
            let more = unsafe { (self.api.more_keys)(self.raw) } != 0;
            self.ensure()?;
            if !more {
                break;
            }
            // SAFETY: exact verified C ABI; this owner retains the live context
            // and its scalar/object outputs. Every call is followed by ensure().
            let key = unsafe { (self.api.next_key)(self.raw) };
            self.ensure()?;
            if key.is_null() {
                return Err(PcRedactionError::Verification);
            }
            // SAFETY: qpdf guarantees a canonical NUL-terminated key until the
            // next iterator call. Exact allowed-key comparison retains nothing.
            let key = unsafe { CStr::from_ptr(key) }.to_bytes();
            if !allowed.contains(&key) || count >= allowed.len() {
                return Err(PcRedactionError::Verification);
            }
            count += 1;
        }
        Ok(count)
    }
    fn stream_keys(&self, stream: Oh, keys: &[&[u8]]) -> PcRedactionResult<Oh> {
        // SAFETY: exact verified C ABI; this owner retains the live context
        // and its scalar/object outputs. Every call is followed by ensure().
        if unsafe { (self.api.is_stream)(self.raw, stream) } == 0 {
            return Err(PcRedactionError::Verification);
        }
        self.ensure()?;
        // SAFETY: exact verified C ABI; this owner retains the live context
        // and its scalar/object outputs. Every call is followed by ensure().
        let dictionary = unsafe { (self.api.stream_dict)(self.raw, stream) };
        self.ensure()?;
        self.keys(dictionary, keys)?;
        Ok(dictionary)
    }
    fn content(&self, page: Oh, limit: usize) -> PcRedactionResult<Buffer<'a>> {
        let mut buffer = Buffer {
            api: self.api,
            pointer: ptr::null_mut(),
            length: 0,
        };
        // SAFETY: live page token; qpdf allocates/free_buffer owns output, even
        // on a failed call. The external process cap bounds pre-admission work.
        let code =
            unsafe { (self.api.content)(self.raw, page, &mut buffer.pointer, &mut buffer.length) };
        self.ensure()?;
        if code != 0 || buffer.length > limit || (buffer.length != 0 && buffer.pointer.is_null()) {
            return Err(PcRedactionError::Limit);
        }
        Ok(buffer)
    }
    fn stream(&self, stream: Oh, limit: usize) -> PcRedactionResult<Buffer<'a>> {
        let mut buffer = Buffer {
            api: self.api,
            pointer: ptr::null_mut(),
            length: 0,
        };
        let mut filtered = 0;
        // SAFETY: exact verified C ABI; this owner retains the live context
        // and its scalar/object outputs. Every call is followed by ensure().
        let code = unsafe {
            (self.api.stream)(
                self.raw,
                stream,
                0,
                &mut filtered,
                &mut buffer.pointer,
                &mut buffer.length,
            )
        };
        self.ensure()?;
        if code != 0 || buffer.length > limit || (buffer.length != 0 && buffer.pointer.is_null()) {
            return Err(PcRedactionError::Limit);
        }
        Ok(buffer)
    }
    // SAFETY: exact verified C ABI; this owner retains the live context
    // and its scalar/object outputs. Every call is followed by ensure().
    fn release_all(&self) -> PcRedactionResult<()> {
        unsafe { (self.api.release)(self.raw) };
        self.ensure()
    }
}
impl Drop for Context<'_> {
    fn drop(&mut self) {
        // SAFETY: sole live qpdf_data owner; synchronous calls have returned and
        // lexical Buffer guards have been dropped before this context.
        unsafe {
            (self.api.cleanup)(&mut self.raw);
        }
    }
}
struct Buffer<'a> {
    api: &'a Functions,
    pointer: *mut u8,
    length: usize,
}
impl Buffer<'_> {
    fn bytes(&self) -> &[u8] {
        if self.length == 0 {
            &[]
        } else {
            // SAFETY: constructors admitted non-null pointer and exact returned
            // length; qpdf malloc buffer lives independently until free_buffer.
            unsafe { std::slice::from_raw_parts(self.pointer, self.length) }
        }
    }
}
impl Drop for Buffer<'_> {
    fn drop(&mut self) {
        // SAFETY: sole malloc-buffer owner; qpdf's own allocator frees/clears it.
        if !self.pointer.is_null() {
            unsafe {
                (self.api.free)(&mut self.pointer);
            }
        }
    }
}
