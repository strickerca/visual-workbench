use super::{
    ConnectionRequest, Error, Handle, Result, helper_path, native_family, process, read_routes,
    wide,
};
use crate::connection_assist::routes::{self, Plan};
use serde::{Deserialize, Serialize};
use std::{
    ffi::OsStr,
    mem::size_of,
    ptr,
    sync::atomic::{AtomicBool, Ordering},
    time::{Duration, Instant},
};
use windows_sys::Win32::{
    Foundation::{ERROR_ALREADY_EXISTS, ERROR_CANCELLED, FILETIME, GetLastError, WAIT_OBJECT_0},
    NetworkManagement::{
        IpHelper::{GetIpInterfaceEntry, MIB_IPINTERFACE_ROW, SetIpInterfaceEntry},
        Ndis::NET_LUID_LH,
    },
    Security::Cryptography::{BCRYPT_USE_SYSTEM_PREFERRED_RNG, BCryptGenRandom},
    System::{
        Com::{COINIT_APARTMENTTHREADED, COINIT_DISABLE_OLE1DDE, CoInitializeEx, CoUninitialize},
        SystemInformation::GetTickCount64,
        Threading::{
            CreateEventW, ExitProcess, GetCurrentProcess, GetCurrentProcessId, GetExitCodeProcess,
            GetProcessTimes, OpenEventW, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
            SYNCHRONIZATION_SYNCHRONIZE, SetEvent, WaitForSingleObject,
        },
    },
    UI::{
        Shell::{
            SEE_MASK_FLAG_NO_UI, SEE_MASK_NOASYNC, SEE_MASK_NOCLOSEPROCESS, SHELLEXECUTEINFOW,
            ShellExecuteExW,
        },
        WindowsAndMessaging::SW_HIDE,
    },
};

const REQUEST_MS: u64 = 45_000;
const HELPER_MS: u64 = 5_000;
const MAX_ENVELOPE: usize = 4096;
const APPLIED: u32 = 0;
const STALE: u32 = 10;
const DECLINED: u32 = 11;
const CANCELLED: u32 = 12;
const FAILED: u32 = 13;
const UNKNOWN: u32 = 14;
static ELEVATING: AtomicBool = AtomicBool::new(false);
struct ElevationPermit;
impl Drop for ElevationPermit {
    fn drop(&mut self) {
        ELEVATING.store(false, Ordering::Release);
    }
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Envelope {
    version: u8,
    parent_pid: u32,
    parent_created: u64,
    expires_tick: u64,
    nonce: String,
    plan: Plan,
}
impl Envelope {
    fn validate(&self, now: u64) -> Result<()> {
        self.plan.validate()?;
        if self.version != 1
            || self.parent_pid == 0
            || self.parent_created == 0
            || self.nonce.len() != 48
            || !self
                .nonce
                .bytes()
                .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
            || self.expires_tick <= now
            || self.expires_tick.saturating_sub(now) > REQUEST_MS
        {
            return Err(Error::Cancelled);
        }
        Ok(())
    }
    fn event_name(&self) -> String {
        format!("Local\\VisualWorkbench-Metric-{}", self.nonce)
    }
}
struct CancelEvent(Handle);
impl Drop for CancelEvent {
    fn drop(&mut self) {
        // SAFETY: valid owned manual-reset event. Late elevated helpers either
        // observe this signal or cannot reopen the event after its last close.
        unsafe {
            SetEvent(self.0.0);
        }
    }
}
fn ticks() -> u64 {
    // SAFETY: pure monotonic boot-relative Windows tick query.
    unsafe { GetTickCount64() }
}
fn creation_time(process: windows_sys::Win32::Foundation::HANDLE) -> Result<u64> {
    let (mut created, mut exited, mut kernel, mut user) = (
        FILETIME::default(),
        FILETIME::default(),
        FILETIME::default(),
        FILETIME::default(),
    );
    // SAFETY: caller retains the process handle; all four output pointers live.
    if unsafe { GetProcessTimes(process, &mut created, &mut exited, &mut kernel, &mut user) } == 0 {
        return Err(Error::Cancelled);
    }
    Ok((u64::from(created.dwHighDateTime) << 32) | u64::from(created.dwLowDateTime))
}
fn hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8] = b"0123456789abcdef";
    let mut result = String::with_capacity(bytes.len() * 2);
    for &byte in bytes {
        result.push(DIGITS[(byte >> 4) as usize] as char);
        result.push(DIGITS[(byte & 15) as usize] as char);
    }
    result
}
fn encode(envelope: &Envelope) -> Result<String> {
    let bytes = serde_json::to_vec(envelope).map_err(|_| Error::InvalidSelection)?;
    if bytes.len() > MAX_ENVELOPE {
        return Err(Error::InvalidSelection);
    }
    Ok(hex(&bytes))
}
fn decode(text: &str) -> Result<Envelope> {
    if text.len() > MAX_ENVELOPE * 2
        || !text.len().is_multiple_of(2)
        || text.is_empty()
        || !text
            .bytes()
            .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
    {
        return Err(Error::InvalidSelection);
    }
    let nibble = |b: u8| {
        if b.is_ascii_digit() {
            b - b'0'
        } else {
            b - b'a' + 10
        }
    };
    let bytes: Vec<u8> = text
        .as_bytes()
        .as_chunks::<2>()
        .0
        .iter()
        .map(|p| (nibble(p[0]) << 4) | nibble(p[1]))
        .collect();
    serde_json::from_slice(&bytes).map_err(|_| Error::InvalidSelection)
}

pub(in crate::connection_assist) fn elevate_metric(
    plan: &Plan,
    request: &ConnectionRequest,
) -> Result<()> {
    ELEVATING
        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
        .map_err(|_| Error::Busy)?;
    let _permit = ElevationPermit;
    request.check()?;
    let helper = helper_path()?;
    // Keep the packaged helper from being replaced while launching it. Root
    // packaging must install DLL and helper in the same trusted app directory.
    use std::os::windows::fs::OpenOptionsExt;
    let _helper_lease = std::fs::OpenOptions::new()
        .read(true)
        .share_mode(windows_sys::Win32::Storage::FileSystem::FILE_SHARE_READ)
        .open(&helper)
        .map_err(|_| Error::HelperUnavailable)?;
    let mut nonce = [0u8; 24];
    // SAFETY: system-preferred CSPRNG fills exactly the writable nonce buffer.
    if unsafe {
        BCryptGenRandom(
            ptr::null_mut(),
            nonce.as_mut_ptr(),
            nonce.len() as u32,
            BCRYPT_USE_SYSTEM_PREFERRED_RNG,
        )
    } < 0
    {
        return Err(Error::WorkerUnavailable);
    }
    // SAFETY: pseudo handle and PID refer only to this current app process.
    let (parent_pid, parent_created) =
        unsafe { (GetCurrentProcessId(), creation_time(GetCurrentProcess())?) };
    let envelope = Envelope {
        version: 1,
        parent_pid,
        parent_created,
        expires_tick: ticks().checked_add(REQUEST_MS).ok_or(Error::Timeout)?,
        nonce: hex(&nonce),
        plan: plan.clone(),
    };
    let event_name = wide(OsStr::new(&envelope.event_name()))?;
    // SAFETY: default current-user DACL, manual-reset, initially nonsignaled;
    // random local-session name, never an inherited handle.
    let event = Handle::checked(unsafe { CreateEventW(ptr::null(), 1, 0, event_name.as_ptr()) })?;
    // SAFETY: immediately checks CreateEventW's documented collision status.
    if unsafe { GetLastError() } == ERROR_ALREADY_EXISTS {
        return Err(Error::WorkerUnavailable);
    }
    let _cancel_event = CancelEvent(event);
    request.check()?;
    let result = process::run(
        &helper,
        &["--elevate-metric".into(), encode(&envelope)?],
        request,
        Duration::from_millis(REQUEST_MS),
    );
    let code = match result {
        Ok(output) if output.output.is_empty() => output.code,
        Ok(_) => return Err(Error::MetricOutcomeUnknown),
        // A callback/timeout cannot certify that SetIpInterfaceEntry wasn't
        // already entered. Never present cancellation as a successful rollback.
        Err(Error::Cancelled | Error::Timeout) => return Err(Error::MetricOutcomeUnknown),
        Err(_) => return Err(Error::MetricOutcomeUnknown),
    };
    match code {
        APPLIED => Ok(()),
        STALE => Err(Error::StaleRoute),
        DECLINED => Err(Error::ElevationDeclined),
        CANCELLED => Err(Error::Cancelled),
        FAILED => Err(Error::MetricFailed),
        _ => Err(Error::MetricOutcomeUnknown),
    }
}

struct Authorization {
    event: Handle,
    parent: Handle,
}
impl Authorization {
    fn open(envelope: &Envelope) -> Result<Self> {
        envelope.validate(ticks())?;
        let name = wide(OsStr::new(&envelope.event_name()))?;
        // SAFETY: open existing app-owned synchronization object only. Never
        // create a missing authorization event from the elevated process.
        let event =
            Handle::checked(unsafe { OpenEventW(SYNCHRONIZATION_SYNCHRONIZE, 0, name.as_ptr()) })
                .map_err(|_| Error::Cancelled)?;
        // SAFETY: read-only process identity/liveness query for captured PID.
        let parent = Handle::checked(unsafe {
            OpenProcess(
                PROCESS_QUERY_LIMITED_INFORMATION | SYNCHRONIZATION_SYNCHRONIZE,
                0,
                envelope.parent_pid,
            )
        })
        .map_err(|_| Error::Cancelled)?;
        let value = Self { event, parent };
        value.check(envelope)?;
        Ok(value)
    }
    fn check(&self, envelope: &Envelope) -> Result<()> {
        envelope.validate(ticks())?;
        // SAFETY: both handles remain owned by this authorization object.
        if unsafe { WaitForSingleObject(self.event.0, 0) } == WAIT_OBJECT_0
            || unsafe { WaitForSingleObject(self.parent.0, 0) } == WAIT_OBJECT_0
            || creation_time(self.parent.0)? != envelope.parent_created
        {
            return Err(Error::Cancelled);
        }
        Ok(())
    }
}
fn watchdog(milliseconds: u64) -> Result<()> {
    std::thread::Builder::new()
        .name("vw-metric-deadline".into())
        .spawn(move || {
            std::thread::sleep(Duration::from_millis(milliseconds));
            // SAFETY: exits only this task-owned helper, never another process. If
            // an OS setter was in flight the caller receives an uncertain outcome.
            unsafe {
                ExitProcess(UNKNOWN);
            }
        })
        .map(|_| ())
        .map_err(|_| Error::WorkerUnavailable)
}
struct Com;
impl Com {
    fn enter() -> Result<Self> {
        // SAFETY: helper main thread initializes its own STA exactly once.
        if unsafe {
            CoInitializeEx(
                ptr::null(),
                (COINIT_APARTMENTTHREADED | COINIT_DISABLE_OLE1DDE) as u32,
            )
        } < 0
        {
            return Err(Error::WorkerUnavailable);
        }
        Ok(Self)
    }
}
impl Drop for Com {
    fn drop(&mut self) {
        /* SAFETY: paired successful initialization on this thread. */
        unsafe {
            CoUninitialize();
        }
    }
}

fn launcher(envelope: &Envelope, encoded: &str) -> Result<u32> {
    watchdog(REQUEST_MS)?;
    let authorization = Authorization::open(envelope)?;
    let _com = Com::enter()?;
    let helper = std::env::current_exe().map_err(|_| Error::HelperUnavailable)?;
    let file = wide(helper.as_os_str())?;
    let directory = wide(helper.parent().ok_or(Error::HelperUnavailable)?.as_os_str())?;
    let verb: Vec<u16> = "runas\0".encode_utf16().collect();
    // Encoded is validated lowercase hex only. No text, shell, route command,
    // environment expansion or owner-controlled executable enters this call.
    let parameters = wide(OsStr::new(&format!("--apply-metric {encoded}")))?;
    authorization.check(envelope)?;
    let mut info = SHELLEXECUTEINFOW {
        cbSize: size_of::<SHELLEXECUTEINFOW>() as u32,
        fMask: SEE_MASK_NOCLOSEPROCESS | SEE_MASK_NOASYNC | SEE_MASK_FLAG_NO_UI,
        lpVerb: verb.as_ptr(),
        lpFile: file.as_ptr(),
        lpParameters: parameters.as_ptr(),
        lpDirectory: directory.as_ptr(),
        nShow: SW_HIDE,
        ..Default::default()
    };
    // SAFETY: all pointers remain live; runas is reached only through the
    // explicit desktop Apply/Revert action, outside DLL load and UI threads.
    if unsafe { ShellExecuteExW(&mut info) } == 0 {
        // SAFETY: immediate error from the failed ShellExecuteExW call.
        return if unsafe { GetLastError() } == ERROR_CANCELLED {
            Ok(DECLINED)
        } else {
            Err(Error::MetricFailed)
        };
    }
    let child = Handle::checked(info.hProcess).map_err(|_| Error::MetricOutcomeUnknown)?;
    let deadline = Instant::now() + Duration::from_millis(HELPER_MS + 500);
    loop {
        if authorization.check(envelope).is_err() || Instant::now() >= deadline {
            return Ok(UNKNOWN);
        }
        // SAFETY: real process handle from SEE_MASK_NOCLOSEPROCESS.
        if unsafe { WaitForSingleObject(child.0, 10) } == WAIT_OBJECT_0 {
            let mut code = UNKNOWN;
            // SAFETY: handle remains live and output pointer is valid.
            if unsafe { GetExitCodeProcess(child.0, &mut code) } == 0 {
                return Ok(UNKNOWN);
            }
            return Ok(code);
        }
    }
}

fn apply(envelope: &Envelope) -> Result<()> {
    watchdog(HELPER_MS)?;
    let authorization = Authorization::open(envelope)?;
    let before = read_routes()?;
    routes::preflight(&envelope.plan, &before)?;
    let plan = &envelope.plan;
    let mut interface = MIB_IPINTERFACE_ROW {
        Family: native_family(plan.family),
        InterfaceLuid: NET_LUID_LH { Value: plan.luid },
        InterfaceIndex: plan.index,
        ..Default::default()
    };
    // SAFETY: initialized exact interface identity; fills the complete current
    // row, preserving every field other than automatic metric and metric.
    if unsafe { GetIpInterfaceEntry(&mut interface) } != 0 {
        return Err(Error::StaleRoute);
    }
    if interface.InterfaceIndex != plan.index
        || interface.Metric != plan.old_metric
        || interface.UseAutomaticMetric != plan.old_automatic
        || !interface.Connected
        || interface.DisableDefaultRoutes
    {
        return Err(Error::StaleRoute);
    }
    routes::preflight(plan, &read_routes()?)?;
    authorization.check(envelope)?;
    interface.UseAutomaticMetric = plan.new_automatic;
    interface.Metric = plan.new_metric;
    // Windows requires zero SitePrefixLength on IPv4 SetIpInterfaceEntry input.
    if plan.family == super::RouteFamily::Ipv4 {
        interface.SitePrefixLength = 0;
    }
    // SAFETY: complete freshly-read row with only requested metric fields
    // changed. Windows itself enforces administrator privilege for this API.
    if unsafe { SetIpInterfaceEntry(&mut interface) } != 0 {
        return Err(Error::MetricFailed);
    }
    let after = read_routes().map_err(|_| Error::MetricOutcomeUnknown)?;
    routes::verified_inverse(plan, &before, &after)?;
    Ok(())
}

pub(in crate::connection_assist) fn helper_main() -> i32 {
    // No OS calls happen for unknown/oversized input, and no payload is echoed.
    let args: Vec<_> = std::env::args_os().take(4).collect();
    if args.len() != 3 {
        return FAILED as i32;
    }
    let Some(mode) = args[1].to_str() else {
        return FAILED as i32;
    };
    let Some(encoded) = args[2].to_str() else {
        return FAILED as i32;
    };
    let Ok(envelope) = decode(encoded) else {
        return FAILED as i32;
    };
    if envelope.validate(ticks()).is_err() {
        return CANCELLED as i32;
    }
    let result = match mode {
        "--elevate-metric" => launcher(&envelope, encoded),
        "--apply-metric" => apply(&envelope).map(|()| APPLIED),
        _ => Err(Error::InvalidSelection),
    };
    match result {
        Ok(code) => code as i32,
        Err(Error::StaleRoute) => STALE as i32,
        Err(Error::Cancelled | Error::Timeout) => CANCELLED as i32,
        Err(Error::MetricOutcomeUnknown) => UNKNOWN as i32,
        Err(_) => FAILED as i32,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn envelope() -> Envelope {
        Envelope {
            version: 1,
            parent_pid: 123,
            parent_created: 42,
            expires_tick: 50_000,
            nonce: "0123456789abcdef".repeat(3),
            plan: Plan {
                kind: routes::MetricActionKind::Fix,
                family: routes::RouteFamily::Ipv4,
                index: 9,
                luid: 90,
                old_automatic: true,
                old_metric: 25,
                new_automatic: false,
                new_metric: 500,
                snapshot_hash: [3; 32],
            },
        }
    }
    #[test]
    fn helper_protocol_is_bounded_typed_and_contains_no_shell() -> Result<()> {
        let encoded = encode(&envelope())?;
        assert!(encoded.bytes().all(|b| b.is_ascii_hexdigit()));
        let decoded = decode(&encoded)?;
        assert_eq!(decoded.plan.index, 9);
        assert!(decode(&"a".repeat(MAX_ENVELOPE * 2 + 2)).is_err());
        assert!(decode("00;").is_err());
        assert!(decode("FF").is_err());
        Ok(())
    }
    #[test]
    fn late_and_far_future_uac_requests_are_rejected_before_route_access() {
        let value = envelope();
        assert!(value.validate(5_000).is_ok());
        assert_eq!(value.validate(50_000), Err(Error::Cancelled));
        assert_eq!(value.validate(4_999), Err(Error::Cancelled));
        let mut invalid = value;
        invalid.nonce.push('x');
        assert!(invalid.validate(10_000).is_err());
    }
}
