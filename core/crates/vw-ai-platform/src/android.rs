//! Application-owned JNI bridge. The final vw_core library exports the small
//! forwarding symbol documented in README; this crate never replaces JNI_OnLoad.
use crate::{Result, check, validate};
use jni::objects::{JByteArray, JClass, JClassLoader, JObject};
use jni::refs::Global;
use jni::sys::{jint, jlong};
use jni::{Env, EnvUnowned, JValue, JavaVM, Outcome, jni_sig, jni_str};
use std::{
    sync::{
        Mutex, OnceLock,
        atomic::{AtomicBool, Ordering},
    },
    time::Instant,
};
use vw_ai_provider::{AndroidRuntime, CredentialFailure, Secret, SecretProvider};
use zeroize::Zeroizing;

const MAX_KEY: usize = 4096;
static RUNTIME: OnceLock<Runtime> = OnceLock::new();
static INITIALIZING: Mutex<()> = Mutex::new(());
static READY: AtomicBool = AtomicBool::new(false);
struct Runtime {
    vm: JavaVM,
    application: Global<JObject<'static>>,
    loader: Global<JClassLoader<'static>>,
    store: Global<JObject<'static>>,
}
impl AndroidRuntime for Runtime {
    fn java_vm(&self) -> &JavaVM {
        &self.vm
    }
    fn context(&self) -> &Global<JObject<'static>> {
        &self.application
    }
    fn class_loader(&self) -> &Global<JClassLoader<'static>> {
        &self.loader
    }
}

/// Native method body; zero means initialized, one is a redacted unavailable
/// result. Only the same Application/store pair can initialize again. Global
/// references live exactly for this process, never an Activity or a paid call.
pub fn initialize_native<'local>(
    mut owner: EnvUnowned<'local>,
    _class: JClass<'local>,
    application: JObject<'local>,
    store: JObject<'local>,
) -> jint {
    let result = owner
        .with_env(|env| {
            let status = if initialize(env, &application, &store).is_ok() {
                0
            } else {
                1
            };
            env.exception_clear();
            Ok::<jint, jni::errors::Error>(status)
        })
        .into_outcome();
    match result {
        Outcome::Ok(status) => status,
        Outcome::Err(_) | Outcome::Panic(_) => {
            // Never describe/log Java exceptions or stringify panic payloads.
            let _ = owner
                .with_env(|env| {
                    env.exception_clear();
                    Ok::<(), jni::errors::Error>(())
                })
                .into_outcome();
            1
        }
    }
}
fn initialize(env: &mut Env<'_>, application: &JObject<'_>, store: &JObject<'_>) -> Result<()> {
    let _guard = INITIALIZING
        .try_lock()
        .map_err(|_| CredentialFailure::Unavailable)?;
    if application.as_raw().is_null()
        || store.as_raw().is_null()
        || !env
            .is_instance_of(application, jni_str!("android/app/Application"))
            .map_err(unavailable)?
        || !env
            .is_instance_of(
                store,
                jni_str!("com/visualworkbench/shared/AndroidProviderKeyStore"),
            )
            .map_err(unavailable)?
    {
        return Err(CredentialFailure::Unavailable);
    }
    let production = env
        .call_method(
            store,
            jni_str!("isProductionForNative"),
            jni_sig!((android.app.Application) -> boolean),
            &[JValue::Object(application)],
        )
        .and_then(|v| v.z())
        .map_err(unavailable)?;
    if !production {
        return Err(CredentialFailure::Unavailable);
    }
    if let Some(runtime) = RUNTIME.get() {
        if !env
            .is_same_object(application, &runtime.application)
            .map_err(unavailable)?
            || !env
                .is_same_object(store, &runtime.store)
                .map_err(unavailable)?
        {
            return Err(CredentialFailure::Unavailable);
        }
    } else {
        let context = env
            .call_method(
                application,
                jni_str!("getApplicationContext"),
                jni_sig!(() -> android.content.Context),
                &[],
            )
            .and_then(|v| v.l())
            .map_err(unavailable)?;
        if !env
            .is_same_object(application, &context)
            .map_err(unavailable)?
        {
            return Err(CredentialFailure::Unavailable);
        }
        let loader = env
            .call_method(
                application,
                jni_str!("getClassLoader"),
                jni_sig!(() -> JClassLoader),
                &[],
            )
            .and_then(|v| v.l())
            .map_err(unavailable)?;
        let loader = env
            .cast_local::<JClassLoader>(loader)
            .map_err(unavailable)?;
        let runtime = Runtime {
            vm: env.get_java_vm().map_err(unavailable)?,
            application: env.new_global_ref(application).map_err(unavailable)?,
            loader: env.new_global_ref(loader).map_err(unavailable)?,
            store: env.new_global_ref(store).map_err(unavailable)?,
        };
        RUNTIME
            .set(runtime)
            .map_err(|_| CredentialFailure::Unavailable)?;
    }
    let runtime = RUNTIME.get().ok_or(CredentialFailure::Unavailable)?;
    if !READY.load(Ordering::Acquire) {
        vw_ai_provider::initialize_android(runtime).map_err(|_| CredentialFailure::Unavailable)?;
        READY.store(true, Ordering::Release);
    }
    Ok(())
}
fn unavailable(_: jni::errors::Error) -> CredentialFailure {
    CredentialFailure::Unavailable
}

/// Cannot be created until real Android verifier classes were loaded. Construct
/// the adapter and validate readiness before reserve/begin_attempt, not after it.
pub struct AndroidCredentials;
impl AndroidCredentials {
    pub fn ready() -> Result<Self> {
        if !READY.load(Ordering::Acquire) {
            return Err(CredentialFailure::Unavailable);
        }
        Ok(Self)
    }
    pub fn is_configured(&self, stop: &dyn vw_ai::Cancellation, deadline: Instant) -> Result<bool> {
        match self.load(stop, deadline) {
            Ok(secret) => {
                drop(secret);
                Ok(true)
            }
            Err(CredentialFailure::Missing) => Ok(false),
            Err(other) => Err(other),
        }
    }
}
impl SecretProvider for AndroidCredentials {
    fn load(&self, stop: &dyn vw_ai::Cancellation, deadline: Instant) -> Result<Secret> {
        check(stop, deadline)?;
        Self::ready()?;
        let runtime = RUNTIME.get().ok_or(CredentialFailure::Unavailable)?;
        // Exactly the provider's bounded worker owns this blocking OS/JNI call.
        // Detaching the caller does not release that worker's capacity early.
        let result = runtime
            .vm
            .attach_current_thread_for_scope(|env| {
                let result = read(env, runtime, stop, deadline);
                env.exception_clear();
                Ok::<Result<Secret>, jni::errors::Error>(result)
            })
            .map_err(unavailable)?;
        result
    }
}
fn read(
    env: &mut Env<'_>,
    runtime: &Runtime,
    stop: &dyn vw_ai::Cancellation,
    deadline: Instant,
) -> Result<Secret> {
    check(stop, deadline)?;
    let remaining = deadline
        .saturating_duration_since(Instant::now())
        .as_millis()
        .clamp(1, 5000) as jlong;
    let object = env
        .call_method(
            &runtime.store,
            jni_str!("loadForNative"),
            jni_sig!("(J)[B"),
            &[JValue::Long(remaining)],
        )
        .and_then(|v| v.l())
        .map_err(unavailable)?;
    if object.as_raw().is_null() {
        return Err(CredentialFailure::Missing);
    }
    let array = env.cast_local::<JByteArray>(object).map_err(unavailable)?;
    let result = (|| {
        let size = array.len(env).map_err(unavailable)?;
        if !(1..=MAX_KEY).contains(&size) {
            return Err(CredentialFailure::Invalid);
        }
        let mut signed = Zeroizing::new(vec![0_i8; size]);
        array.get_region(env, 0, &mut signed).map_err(unavailable)?;
        let bytes = Zeroizing::new(signed.iter().map(|&v| v as u8).collect::<Vec<_>>());
        validate(&bytes, MAX_KEY)?;
        check(stop, deadline)?;
        Secret::from_protected_bytes(bytes).map_err(|_| CredentialFailure::Invalid)
    })();
    // The final Kotlin implementation returns <=4096 bytes. Clear the exact local
    // Java array, with no process-wide transient buffer or cross-reader race.
    env.exception_clear();
    let cleared = env.call_method(
        &runtime.store,
        jni_str!("clearForNative"),
        jni_sig!("([B)V"),
        &[JValue::Object(array.as_ref())],
    );
    if cleared.is_err() {
        env.exception_clear();
        return Err(CredentialFailure::Unavailable);
    }
    result
}
