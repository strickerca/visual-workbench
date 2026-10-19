//! No registered foreign callback/result directory. Native invokes one pinned
//! JVM class, whose private method builds the real private AndroidTileBuild.
//! Its nested JNI callback may consume output exactly once on that same thread.
use super::*;
use ::jni::objects::{JClass, JObject, JObjectArray, JString};
use ::jni::refs::Global;
use ::jni::sys::{jboolean, jint, jlong};
use ::jni::{Env, EnvUnowned, JValue, JavaVM, Outcome, jni_sig, jni_str};
use std::{
    cell::RefCell,
    sync::{
        Mutex, OnceLock,
        atomic::{AtomicU64, Ordering},
    },
    time::Duration,
};

struct Runtime {
    vm: JavaVM,
    class: Global<JClass<'static>>,
    output_class: Global<JClass<'static>>,
}
static RUNTIME: OnceLock<Runtime> = OnceLock::new();
static INITIALIZING: Mutex<()> = Mutex::new(());
static NEXT_ID: AtomicU64 = AtomicU64::new(1);
static FOREIGN_SOURCES: OnceLock<foreign_retirement::ForeignSources> = OnceLock::new();
fn foreign_sources() -> &'static foreign_retirement::ForeignSources {
    FOREIGN_SOURCES.get_or_init(foreign_retirement::ForeignSources::new)
}
struct Probe {
    id: u64,
    closed: Arc<AtomicBool>,
    cancel: Arc<Cancellation>,
    deadline: Instant,
}
static PROBES: OnceLock<Mutex<Vec<Probe>>> = OnceLock::new();
fn probes() -> &'static Mutex<Vec<Probe>> {
    PROBES.get_or_init(|| Mutex::new(Vec::with_capacity(2)))
}
struct Pending {
    id: u64,
    context: Context,
    consumed: bool,
    result: Option<Result<cache::ApprovedBuild>>,
}
thread_local! { static ACTIVE:RefCell<Option<Pending>>=const{RefCell::new(None)}; }
struct ActiveGuard(u64);
impl Drop for ActiveGuard {
    fn drop(&mut self) {
        ACTIVE.with(|cell| {
            cell.borrow_mut().take();
        });
        let mut entries = match probes().lock() {
            Ok(v) => v,
            Err(p) => p.into_inner(),
        };
        entries.retain(|probe| probe.id != self.0);
    }
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
        != "com.visualworkbench.shared.giant.AndroidGiantRuntime"
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
    let output_class = env
        .find_class(jni_str!(
            "com/visualworkbench/shared/giant/AndroidTileBuild"
        ))
        .map_err(|_| Error::Unsupported)?;
    let runtime = Runtime {
        vm: env.get_java_vm().map_err(|_| Error::Unsupported)?,
        class: env.new_global_ref(class).map_err(|_| Error::Unsupported)?,
        output_class: env
            .new_global_ref(output_class)
            .map_err(|_| Error::Unsupported)?,
    };
    RUNTIME.set(runtime).map_err(|_| Error::Backpressure)
}
#[unsafe(no_mangle)]
pub extern "system" fn Java_com_visualworkbench_shared_giant_AndroidGiantRuntime_initializeNative<
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
            Ok::<jint, ::jni::errors::Error>(status)
        })
        .into_outcome();
    match outcome {
        Outcome::Ok(v) => v,
        Outcome::Err(_) | Outcome::Panic(_) => {
            clear_exception(&mut owner);
            1
        }
    }
}
fn clear_exception(owner: &mut EnvUnowned<'_>) {
    let _ = owner
        .with_env(|env| {
            env.exception_clear();
            Ok::<(), ::jni::errors::Error>(())
        })
        .into_outcome();
}

pub(in crate::giant_images::producer) fn build(
    source: Arc<input::PreparedSource>,
    disk_cache_bytes: u64,
    timeout_ms: u64,
    closed: Arc<AtomicBool>,
    cancel: Arc<Cancellation>,
    budget: SessionBudget,
) -> Result<cache::ApprovedBuild> {
    let runtime = RUNTIME.get().ok_or(Error::Unsupported)?;
    // Kotlin reserves its 80 MiB owner through sessionResources(). Native
    // preparation/copy must be the exact same process pool, never a local cap.
    if !budget.same_pool(&crate::session_resources::shared_budget()?) {
        return Err(Error::Invalid);
    }
    let context = Context {
        source,
        closed,
        cancel,
        budget,
        disk_cache_bytes,
        deadline: Instant::now()
            .checked_add(Duration::from_millis(timeout_ms))
            .ok_or(Error::Limit)?,
    };
    context.check()?;
    if context.source.format != "jpeg" || context.source.info.encoded_bytes > 1024 * 1024 * 1024 {
        return Err(Error::Unsupported);
    }
    context
        .source
        .verify(&context.closed, &context.cancel, &context.budget)?;
    let work = context.source.staging.path().join("android-work");
    std::fs::create_dir(&work).map_err(io_error)?;
    let id = NEXT_ID
        .try_update(Ordering::AcqRel, Ordering::Acquire, |id| {
            (id < i64::MAX as u64).then_some(id + 1)
        })
        .map_err(|_| Error::Limit)?;
    if ACTIVE.with(|cell| cell.borrow().is_some()) {
        return Err(Error::Backpressure);
    }
    {
        let mut entries = probes().lock().map_err(|_| Error::Storage)?;
        if entries.len() >= 2 {
            return Err(Error::Backpressure);
        }
        entries.push(Probe {
            id,
            closed: context.closed.clone(),
            cancel: context.cancel.clone(),
            deadline: context.deadline,
        });
    }
    ACTIVE.with(|cell| {
        *cell.borrow_mut() = Some(Pending {
            id,
            context,
            consumed: false,
            result: None,
        })
    });
    let _guard = ActiveGuard(id);
    let foreign_retired = ACTIVE.with(|cell| {
        let active = cell.borrow();
        let pending = active.as_ref().ok_or(Error::Storage)?;
        foreign_sources().retain(id, pending.context.source.clone())
    })?;
    // This pin has no scope guard: JNI/attachment errors, panic, cancellation,
    // timeout and unsuccessful Java cleanup must all retain the exact source.
    // Only retiredNative may remove it, including after an explicit Java retry.
    // Synchronous scoped attachment remains on the admitted input worker. Java
    // runBlocking joins its retained codec worker and actual output close.
    let status = runtime
        .vm
        .attach_current_thread_for_scope(|env| {
            let result = invoke(env, runtime, id, &work);
            env.exception_clear();
            Ok::<Result<jint>, ::jni::errors::Error>(result)
        })
        .map_err(|_| Error::Unsupported)??;
    let mut pending = ACTIVE
        .with(|cell| cell.borrow_mut().take())
        .ok_or(Error::Storage)?;
    // A status code is not proof of retirement, including status zero.
    foreign_retired.require_confirmed()?;
    after_android_retirement(
        &pending.context,
        pending.consumed,
        status,
        pending.result.take(),
    )
}
fn invoke(env: &mut Env<'_>, runtime: &Runtime, id: u64, work: &std::path::Path) -> Result<jint> {
    let info = ACTIVE
        .with(|cell| {
            cell.borrow()
                .as_ref()
                .map(|p| p.context.source.info.clone())
        })
        .ok_or(Error::Storage)?;
    let disk = ACTIVE
        .with(|cell| cell.borrow().as_ref().map(|p| p.context.disk_cache_bytes))
        .ok_or(Error::Storage)?;
    let source = env
        .new_string(info.source_path.to_str().ok_or(Error::Invalid)?)
        .map_err(|_| Error::Storage)?;
    let work = env
        .new_string(work.to_str().ok_or(Error::Invalid)?)
        .map_err(|_| Error::Storage)?;
    let sha = env
        .new_string(tiles::hex(&info.sha256))
        .map_err(|_| Error::Storage)?;
    env.call_static_method(
        &runtime.class,
        jni_str!("buildPrepared"),
        jni_sig!("(JLjava/lang/String;Ljava/lang/String;Ljava/lang/String;IIIJJ)I"),
        &[
            JValue::Long(id as jlong),
            JValue::Object(source.as_ref()),
            JValue::Object(work.as_ref()),
            JValue::Object(sha.as_ref()),
            JValue::Int(i32::try_from(info.raw_width).map_err(|_| Error::Limit)?),
            JValue::Int(i32::try_from(info.raw_height).map_err(|_| Error::Limit)?),
            JValue::Int(i32::from(info.orientation)),
            JValue::Long(i64::try_from(info.encoded_bytes).map_err(|_| Error::Limit)?),
            JValue::Long(i64::try_from(disk).map_err(|_| Error::Limit)?),
        ],
    )
    .and_then(|v| v.i())
    .map_err(|_| Error::Storage)
}
/// The pinned runner calls this only after its exact operation has no live or
/// quarantined Java producer owner. It also runs after a later successful retry.
#[unsafe(no_mangle)]
pub extern "system" fn Java_com_visualworkbench_shared_giant_AndroidGiantRuntime_retiredNative<
    'local,
>(
    mut owner: EnvUnowned<'local>,
    class: JClass<'local>,
    id: jlong,
) -> jint {
    let outcome = owner
        .with_env(|env| {
            let valid = id > 0
                && RUNTIME
                    .get()
                    .is_some_and(|r| env.is_same_object(&class, &r.class).unwrap_or(false));
            let status = if valid && foreign_sources().confirm_retired(id as u64).is_ok() {
                0
            } else {
                1
            };
            env.exception_clear();
            Ok::<jint, ::jni::errors::Error>(status)
        })
        .into_outcome();
    match outcome {
        Outcome::Ok(v) => v,
        Outcome::Err(_) | Outcome::Panic(_) => {
            clear_exception(&mut owner);
            1
        }
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn Java_com_visualworkbench_shared_giant_AndroidGiantRuntime_cancelledNative<
    'local,
>(
    mut owner: EnvUnowned<'local>,
    class: JClass<'local>,
    id: jlong,
) -> jboolean {
    let outcome = owner
        .with_env(|env| {
            let valid = RUNTIME
                .get()
                .is_some_and(|r| env.is_same_object(&class, &r.class).unwrap_or(false));
            let cancelled = !valid
                || id <= 0
                || probes()
                    .lock()
                    .ok()
                    .and_then(|entries| {
                        entries.iter().find(|p| p.id == id as u64).map(|p| {
                            p.closed.load(Ordering::Acquire)
                                || p.cancel.is_cancelled()
                                || Instant::now() >= p.deadline
                        })
                    })
                    .unwrap_or(true);
            env.exception_clear();
            Ok::<jboolean, ::jni::errors::Error>(cancelled)
        })
        .into_outcome();
    match outcome {
        Outcome::Ok(v) => v,
        Outcome::Err(_) | Outcome::Panic(_) => {
            clear_exception(&mut owner);
            true
        }
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn Java_com_visualworkbench_shared_giant_AndroidGiantRuntime_consumeNative<
    'local,
>(
    mut owner: EnvUnowned<'local>,
    class: JClass<'local>,
    id: jlong,
    output: JObject<'local>,
) -> jint {
    let outcome = owner
        .with_env(|env| {
            let status = consume(env, &class, id, &output);
            env.exception_clear();
            Ok::<jint, ::jni::errors::Error>(if status.is_ok() { 0 } else { 1 })
        })
        .into_outcome();
    match outcome {
        Outcome::Ok(v) => v,
        Outcome::Err(_) | Outcome::Panic(_) => {
            clear_exception(&mut owner);
            1
        }
    }
}
fn consume(env: &mut Env<'_>, class: &JClass<'_>, id: jlong, output: &JObject<'_>) -> Result<()> {
    let runtime = RUNTIME.get().ok_or(Error::Unsupported)?;
    if id <= 0
        || !env
            .is_same_object(class, &runtime.class)
            .map_err(|_| Error::Unsupported)?
    {
        return Err(Error::Unsupported);
    }
    let output_class = env
        .get_object_class(output)
        .map_err(|_| Error::Unsupported)?;
    if !env
        .is_same_object(&output_class, &runtime.output_class)
        .map_err(|_| Error::Unsupported)?
    {
        return Err(Error::Unsupported);
    }
    ACTIVE.with(|cell| {
        let mut active = cell.borrow_mut();
        let pending = active.as_mut().ok_or(Error::Unsupported)?;
        if pending.id != id as u64 || pending.consumed {
            return Err(Error::Unsupported);
        }
        pending.consumed = true; // A failed copy cannot be retried with another directory.
        let result = (|| {
            pending.context.check()?;
            let description = describe(env, runtime, output)?;
            copy_output(description, &pending.context)
        })();
        let success = result.is_ok();
        pending.result = Some(result);
        if success { Ok(()) } else { Err(Error::Storage) }
    })
}
fn describe(env: &mut Env<'_>, runtime: &Runtime, output: &JObject<'_>) -> Result<CompletedOutput> {
    let value = env
        .call_static_method(
            &runtime.class,
            jni_str!("describeOutputForNative"),
            jni_sig!("(Lcom/visualworkbench/shared/giant/AndroidTileBuild;)[Ljava/lang/String;"),
            &[JValue::Object(output)],
        )
        .and_then(|v| v.l())
        .map_err(|_| Error::Invalid)?;
    let array = env
        .cast_local::<JObjectArray>(value)
        .map_err(|_| Error::Invalid)?;
    if array.len(env).map_err(|_| Error::Invalid)? != 10 {
        return Err(Error::Invalid);
    }
    let mut fields = Vec::with_capacity(10);
    for index in 0..10 {
        let field = array.get_element(env, index).map_err(|_| Error::Invalid)?;
        let field = env
            .cast_local::<JString>(field)
            .map_err(|_| Error::Invalid)?;
        let length = env
            .call_method(&field, jni_str!("length"), jni_sig!(() -> int), &[])
            .and_then(|v| v.i())
            .map_err(|_| Error::Invalid)?;
        if !(1..=8192).contains(&length) {
            return Err(Error::Limit);
        }
        fields.push(field.try_to_string(env).map_err(|_| Error::Invalid)?);
    }
    Ok(CompletedOutput {
        directory: PathBuf::from(&fields[0]),
        source_sha256: fields[1].clone(),
        encoded_bytes: fields[2].parse().map_err(|_| Error::Invalid)?,
        raw_width: fields[3].parse().map_err(|_| Error::Invalid)?,
        raw_height: fields[4].parse().map_err(|_| Error::Invalid)?,
        orientation: fields[5].parse().map_err(|_| Error::Invalid)?,
        width: fields[6].parse().map_err(|_| Error::Invalid)?,
        height: fields[7].parse().map_err(|_| Error::Invalid)?,
        preview_width: fields[8].parse().map_err(|_| Error::Invalid)?,
        preview_height: fields[9].parse().map_err(|_| Error::Invalid)?,
    })
}
