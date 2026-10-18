//! Fixed Android URI reader. Native holds a strong JVM owner before opening the
//! resolver and keeps that owner plus the SAME input admission on failed close.
use super::{
    Error, GiantInputLimits, PickedOriginal, Result,
    uri_staging::{self, Reader, StagedInput},
};
use crate::Cancellation;
use jni::objects::{JByteArray, JClass, JObject, JString};
use jni::refs::Global;
use jni::sys::jint;
use jni::{Env, EnvUnowned, JValue, JavaVM, Outcome, jni_sig, jni_str};
use std::{
    path::Path,
    sync::{Arc, Mutex, OnceLock},
};
use vw_stream::SessionBudget;

struct Runtime {
    vm: JavaVM,
    class: Global<JClass<'static>>,
}
static RUNTIME: OnceLock<Runtime> = OnceLock::new();
static INITIALIZING: Mutex<()> = Mutex::new(());
struct Pending {
    reader: AndroidReader,
    input: StagedInput,
}
static QUARANTINE: OnceLock<Mutex<Vec<Pending>>> = OnceLock::new();
fn quarantine() -> &'static Mutex<Vec<Pending>> {
    // Each retained entry still holds one of the existing two input permits.
    QUARANTINE.get_or_init(|| Mutex::new(Vec::with_capacity(2)))
}
struct AndroidReader {
    runtime: &'static Runtime,
    owner: Global<JObject<'static>>,
    bytes: Global<JByteArray<'static>>,
}
impl AndroidReader {
    fn create(runtime: &'static Runtime, uri: &str) -> Result<Self> {
        runtime.vm.attach_current_thread_for_scope(|env| {
            let result = (|| {
                let text = env.new_string(uri).map_err(|_| Error::Storage)?;
                // This only allocates a holder. It does NOT open the URI.
                let owner = env.call_static_method(
                    &runtime.class,
                    jni_str!("readerForNative"),
                    jni_sig!("(Ljava/lang/String;)Lcom/visualworkbench/shared/giant/AndroidGiantUriReader;"),
                    &[JValue::Object(text.as_ref())],
                ).and_then(|v| v.l()).map_err(|_| Error::Storage)?;
                if owner.as_raw().is_null() { return Err(Error::Storage); }
                let owner = env.new_global_ref(owner).map_err(|_| Error::Storage)?;
                let bytes = JByteArray::new(env, uri_staging::CHUNK).map_err(|_| Error::Storage)?;
                let bytes = env.new_global_ref(bytes).map_err(|_| Error::Storage)?;
                Ok(Self { runtime, owner, bytes })
            })();
            env.exception_clear();
            Ok::<Result<Self>, jni::errors::Error>(result)
        }).map_err(|_| Error::Storage)?
    }
    fn invoke_void(&self, name: &jni::strings::JNIStr) -> Result<()> {
        self.runtime
            .vm
            .attach_current_thread_for_scope(|env| {
                let result = env
                    .call_method(&self.owner, name, jni_sig!(() -> void), &[])
                    .map(|_| ())
                    .map_err(|_| Error::Storage);
                env.exception_clear();
                Ok::<Result<()>, jni::errors::Error>(result)
            })
            .map_err(|_| Error::Storage)?
    }
}
impl Reader for AndroidReader {
    fn open(&mut self) -> Result<()> {
        self.invoke_void(jni_str!("openForNative"))
    }
    fn read(&mut self, bytes: &mut [u8]) -> Result<usize> {
        if bytes.len() != uri_staging::CHUNK {
            return Err(Error::Invalid);
        }
        self.runtime
            .vm
            .attach_current_thread_for_scope(|env| {
                let result = (|| {
                    let count = env
                        .call_method(
                            &self.owner,
                            jni_str!("readForNative"),
                            jni_sig!("([B)I"),
                            &[JValue::Object(self.bytes.as_obj())],
                        )
                        .and_then(|v| v.i())
                        .map_err(|_| Error::Storage)?;
                    if count == -1 {
                        return Ok(0);
                    }
                    // A positive-sized InputStream read must progress. Do not spin
                    // indefinitely or reinterpret an unknown count as end-of-file.
                    if count <= 0 || count as usize > bytes.len() {
                        return Err(Error::Invalid);
                    }
                    let count = count as usize;
                    let mut signed = [0_i8; uri_staging::CHUNK];
                    self.bytes
                        .get_region(env, 0, &mut signed[..count])
                        .map_err(|_| Error::Storage)?;
                    for (to, from) in bytes[..count].iter_mut().zip(&signed[..count]) {
                        *to = *from as u8;
                    }
                    Ok(count)
                })();
                env.exception_clear();
                Ok::<Result<usize>, jni::errors::Error>(result)
            })
            .map_err(|_| Error::Storage)?
    }
    fn close(&mut self) -> Result<()> {
        self.invoke_void(jni_str!("closeForNative"))
    }
    fn quarantine(self, input: StagedInput) {
        let mut entries = match quarantine().lock() {
            Ok(entries) => entries,
            Err(poisoned) => poisoned.into_inner(),
        };
        entries.push(Pending {
            reader: self,
            input,
        });
    }
}

pub(super) fn pick(
    uri: &str,
    work_parent: &Path,
    limits: GiantInputLimits,
    timeout_ms: u64,
    budget: SessionBudget,
    cancel: &Cancellation,
) -> Result<Arc<PickedOriginal>> {
    let runtime = RUNTIME.get().ok_or(Error::Unsupported)?;
    let input = StagedInput::create(work_parent, &limits, budget, cancel)?;
    // Admission is already installed before any Java allocation/source open.
    let reader = AndroidReader::create(runtime, uri)?;
    uri_staging::copy_reader(input, reader, limits, timeout_ms, cancel)
}

/// Explicit retry of actual close; no timeout, boolean or JNI result directory
/// can credit a failed reader. Failed directory cleanup keeps its existing
/// process-life quarantine even after the Java reader has retired.
pub(super) fn retry_readers(cancel: &Cancellation) -> Result<()> {
    cancel.check().map_err(Error::from)?;
    let pending = {
        let mut entries = match quarantine().lock() {
            Ok(entries) => entries,
            Err(poisoned) => poisoned.into_inner(),
        };
        std::mem::replace(&mut *entries, Vec::with_capacity(2))
    };
    let mut failed = false;
    for Pending { mut reader, input } in pending {
        if cancel.is_cancelled() || reader.close().is_err() {
            reader.quarantine(input);
            failed = true;
        } else if input.release().is_err() {
            failed = true;
        }
    }
    cancel.check().map_err(Error::from)?;
    if failed { Err(Error::Storage) } else { Ok(()) }
}

fn initialize(env: &mut Env<'_>, class: &JClass<'_>) -> Result<()> {
    let _guard = INITIALIZING.try_lock().map_err(|_| Error::Backpressure)?;
    let name = env
        .call_method(
            class,
            jni_str!("getName"),
            jni_sig!(() -> java.lang.String),
            &[],
        )
        .and_then(|v| v.l())
        .map_err(|_| Error::Unsupported)?;
    let name = env
        .cast_local::<JString>(name)
        .map_err(|_| Error::Unsupported)?;
    if name.try_to_string(env).map_err(|_| Error::Unsupported)?
        != "com.visualworkbench.shared.giant.AndroidGiantUriRuntime"
    {
        return Err(Error::Unsupported);
    }
    if let Some(runtime) = RUNTIME.get() {
        return if env
            .is_same_object(class, &runtime.class)
            .map_err(|_| Error::Unsupported)?
        {
            Ok(())
        } else {
            Err(Error::Unsupported)
        };
    }
    let runtime = Runtime {
        vm: env.get_java_vm().map_err(|_| Error::Unsupported)?,
        class: env.new_global_ref(class).map_err(|_| Error::Unsupported)?,
    };
    RUNTIME.set(runtime).map_err(|_| Error::Backpressure)
}

// SAFETY: unique fixed JNI entry. EnvUnowned contains the foreign invocation;
// with_env/Outcome catches unwinds and every error clears without logging Java
// exception text, which may include a private URI or provider identifier.
#[unsafe(no_mangle)]
pub extern "system" fn Java_com_visualworkbench_shared_giant_AndroidGiantUriRuntime_initializeNative<
    'local,
>(
    mut owner: EnvUnowned<'local>,
    class: JClass<'local>,
) -> jint {
    let outcome = owner
        .with_env(|env| {
            let status = if initialize(env, &class).is_ok() {
                0
            } else {
                1
            };
            env.exception_clear();
            Ok::<jint, jni::errors::Error>(status)
        })
        .into_outcome();
    match outcome {
        Outcome::Ok(status) => status,
        Outcome::Err(_) | Outcome::Panic(_) => {
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
