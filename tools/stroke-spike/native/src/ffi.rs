//! JNI ABI contains primitives only. Neither JNI environment nor receiver is read.
use std::ffi::c_void;

macro_rules! export {
    ($symbol:ident, $function:ident, ($($name:ident: $ty:ty),*) -> $result:ty) => {
        // SAFETY: The unique symbol is a JVM native-method ABI entry point. The
        // two opaque JVM pointers are never dereferenced or retained; every
        // remaining argument is a primitive and checked by the safe registry.
        #[unsafe(no_mangle)]
        pub extern "system" fn $symbol(_env: *mut c_void, _receiver: *mut c_void, $($name: $ty),*) -> $result {
            super::$function($($name),*)
        }
    };
}
export!(Java_com_visualworkbench_strokespike_NativeInk_begin, begin, (family:i32,width:f64,stabilization:f64)->i64);
export!(Java_com_visualworkbench_strokespike_NativeInk_append, append, (handle:i64,x:f64,y:f64,time:i64,pressure:f64)->i32);
export!(Java_com_visualworkbench_strokespike_NativeInk_preview, preview, (handle:i64,x:f64,y:f64,time:i64,pressure:f64)->i64);
export!(Java_com_visualworkbench_strokespike_NativeInk_polygons, polygons, (handle:i64)->i32);
export!(Java_com_visualworkbench_strokespike_NativeInk_vertices, vertices, (handle:i64,polygon:i32)->i32);
export!(Java_com_visualworkbench_strokespike_NativeInk_coordinate, coordinate, (handle:i64,polygon:i32,vertex:i32,axis:i32)->f64);
export!(Java_com_visualworkbench_strokespike_NativeInk_finish, finish, (handle:i64)->i32);
export!(Java_com_visualworkbench_strokespike_NativeInk_hashWord, hash_word, (handle:i64,word:i32)->i64);
export!(Java_com_visualworkbench_strokespike_NativeInk_release, release, (handle:i64)->i32);
export!(Java_com_visualworkbench_strokespike_NativeInk_liveHandles, live_handles, ()->i32);
