//! Exact forwarding boundary from the reviewed protected-provider adapter.
#[unsafe(no_mangle)]
pub extern "system" fn Java_com_visualworkbench_shared_AndroidProviderRuntime_initializeNative<
    'local,
>(
    env: jni::EnvUnowned<'local>,
    class: jni::objects::JClass<'local>,
    application: jni::objects::JObject<'local>,
    store: jni::objects::JObject<'local>,
) -> jni::sys::jint {
    vw_ai_platform::android::initialize_native(env, class, application, store)
}
