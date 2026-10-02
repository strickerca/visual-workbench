use std::{
    io::{BufRead, Write},
    process::{Command, Stdio},
    sync::mpsc,
    thread,
    time::{Duration, Instant},
};
use vw_vdd_probe::{self as protocol, Mode, Target};
use windows::{
    Win32::{
        Devices::{DeviceAndDriverInstallation::*, Display::*},
        Foundation::{CloseHandle, GENERIC_READ, GENERIC_WRITE, HANDLE},
        Graphics::Gdi::DISPLAYCONFIG_PATH_ACTIVE,
        Storage::FileSystem::*,
        System::{Com::CoCreateGuid, IO::DeviceIoControl},
        UI::HiDpi::*,
    },
    core::{GUID, PCWSTR},
};

type Result<T> = std::result::Result<T, String>;
const INTERFACE: GUID = GUID::from_u128(0xe5bcc234_1e0c_418a_a0d4_ef8b7501414d);

fn api_error(name: &str, error: windows::core::Error) -> String {
    format!("{name} failed (HRESULT 0x{:08x})", error.code().0 as u32)
}

fn interfaces() -> Result<Vec<Vec<u16>>> {
    for _ in 0..3 {
        let mut length = 0;
        // SAFETY: Exact SudoVDA interface GUID and a writable count; no device ID filter.
        let status = unsafe {
            CM_Get_Device_Interface_List_SizeW(
                &mut length,
                &INTERFACE,
                PCWSTR::null(),
                CM_GET_DEVICE_INTERFACE_LIST_PRESENT,
            )
        };
        if status != CR_SUCCESS {
            return Err(format!("Interface size query failed ({})", status.0));
        }
        if length > 65536 {
            return Err("Interface list exceeds probe bound".into());
        }
        let mut buffer = vec![0_u16; length.max(2) as usize];
        // SAFETY: Initialized UTF-16 buffer is passed with its actual capacity.
        let status = unsafe {
            CM_Get_Device_Interface_ListW(
                &INTERFACE,
                PCWSTR::null(),
                &mut buffer,
                CM_GET_DEVICE_INTERFACE_LIST_PRESENT,
            )
        };
        if status == CR_BUFFER_SMALL {
            continue;
        }
        if status != CR_SUCCESS {
            return Err(format!("Interface query failed ({})", status.0));
        }
        if buffer.last() != Some(&0) {
            return Err("Unterminated interface list".into());
        }
        return Ok(buffer
            .split(|c| *c == 0)
            .take_while(|part| !part.is_empty())
            .map(|part| {
                let mut path = part.to_vec();
                path.push(0);
                path
            })
            .collect());
    }
    Err("Interface list changed during three bounded reads".into())
}

struct Device(HANDLE);
impl Drop for Device {
    fn drop(&mut self) {
        // SAFETY: This wrapper exclusively owns the successful CreateFile handle.
        let _ = unsafe { CloseHandle(self.0) };
    }
}
impl Device {
    fn open() -> Result<Self> {
        let devices = interfaces()?;
        if devices.len() != 1 {
            return Err("Exactly one dedicated SudoVDA interface is required".into());
        }
        // SAFETY: CM returned a NUL-terminated path. No arbitrary device path is accepted.
        let handle = unsafe {
            CreateFileW(
                PCWSTR(devices[0].as_ptr()),
                GENERIC_READ.0 | GENERIC_WRITE.0,
                FILE_SHARE_READ | FILE_SHARE_WRITE,
                None,
                OPEN_EXISTING,
                FILE_ATTRIBUTE_NORMAL,
                None,
            )
        }
        .map_err(|error| api_error("Open SudoVDA", error))?;
        let device = Self(handle);
        protocol::check_version(&device.ioctl(protocol::VERSION, &[], 4)?)
            .map_err(|e| e.to_string())?;
        protocol::check_watchdog(&device.ioctl(protocol::WATCHDOG, &[], 8)?)
            .map_err(|e| e.to_string())?;
        Ok(device)
    }
    fn ioctl(&self, code: u32, input: &[u8], output_size: usize) -> Result<Vec<u8>> {
        if input.len() > 56 || output_size > 12 {
            return Err("IOCTL buffer exceeds ABI bounds".into());
        }
        let mut output = vec![0_u8; output_size];
        let mut returned = 0;
        // SAFETY: Synchronous call; owned input/output slices remain live for its duration.
        // A driver hang is contained by the mandatory run.ps1 Windows process-tree timeout.
        unsafe {
            DeviceIoControl(
                self.0,
                code,
                if input.is_empty() {
                    None
                } else {
                    Some(input.as_ptr().cast())
                },
                input.len() as u32,
                if output.is_empty() {
                    None
                } else {
                    Some(output.as_mut_ptr().cast())
                },
                output.len() as u32,
                Some(&mut returned),
                None,
            )
        }
        .map_err(|error| api_error("SudoVDA IOCTL", error))?;
        if returned as usize != output_size {
            return Err("Unexpected IOCTL output length".into());
        }
        Ok(output)
    }
    fn ping(&self) -> Result<()> {
        self.ioctl(protocol::PING, &[], 0).map(|_| ())
    }
    fn add(&self, mode: Mode, guid: [u8; 16]) -> Result<Target> {
        let request = protocol::add_request(mode, guid).map_err(|e| e.to_string())?;
        let result = self
            .ioctl(protocol::ADD, &request, 12)
            .and_then(|bytes| protocol::add_response(&bytes).map_err(|e| e.to_string()));
        if result.is_err() {
            let _ = self.remove(&guid);
        }
        result
    }
    fn remove(&self, guid: &[u8; 16]) -> Result<()> {
        self.ioctl(protocol::REMOVE, guid, 0).map(|_| ())
    }
}

fn topology() -> Result<(Vec<DISPLAYCONFIG_PATH_INFO>, Vec<DISPLAYCONFIG_MODE_INFO>)> {
    for _ in 0..3 {
        let (mut paths, mut modes) = (0, 0);
        // SAFETY: Valid mutable output counts; queried arrays are allocated below.
        let status = unsafe { GetDisplayConfigBufferSizes(QDC_ALL_PATHS, &mut paths, &mut modes) };
        if status.0 != 0 {
            return Err(format!("Topology size query failed ({})", status.0));
        }
        if paths > 4096 || modes > 4096 {
            return Err("Topology exceeds probe bounds".into());
        }
        let mut path_array = vec![DISPLAYCONFIG_PATH_INFO::default(); paths as usize];
        let mut mode_array = vec![DISPLAYCONFIG_MODE_INFO::default(); modes as usize];
        // SAFETY: Arrays have initialized capacity corresponding to both input counts.
        let status = unsafe {
            QueryDisplayConfig(
                QDC_ALL_PATHS,
                &mut paths,
                path_array.as_mut_ptr(),
                &mut modes,
                mode_array.as_mut_ptr(),
                None,
            )
        };
        if status.0 == 122 {
            continue;
        }
        if status.0 != 0 {
            return Err(format!("Topology query failed ({})", status.0));
        }
        path_array.truncate(paths as usize);
        mode_array.truncate(modes as usize);
        return Ok((path_array, mode_array));
    }
    Err("Topology changed during three bounded reads".into())
}

fn present(target: Target, active_mode: Option<Mode>) -> Result<bool> {
    let (paths, modes) = topology()?;
    for path in paths {
        if path.targetInfo.adapterId.LowPart != target.adapter_low
            || path.targetInfo.adapterId.HighPart != target.adapter_high
            || path.targetInfo.id != target.target
            || !path.targetInfo.targetAvailable.as_bool()
        {
            continue;
        }
        let Some(mode) = active_mode else {
            return Ok(true);
        };
        if path.flags & DISPLAYCONFIG_PATH_ACTIVE == 0 {
            continue;
        }
        let refresh = path.targetInfo.refreshRate;
        if refresh.Denominator == 0
            || u64::from(refresh.Numerator) * 1000
                != u64::from(mode.refresh_millihertz) * u64::from(refresh.Denominator)
        {
            continue;
        }
        // SAFETY: QDC_ALL_PATHS (without virtual-mode awareness) selects modeInfoIdx.
        let index = unsafe { path.sourceInfo.Anonymous.modeInfoIdx } as usize;
        if let Some(source) = modes
            .get(index)
            .filter(|m| m.infoType == DISPLAYCONFIG_MODE_INFO_TYPE_SOURCE)
        {
            // SAFETY: infoType was checked against the sourceMode union discriminator.
            let source_mode = unsafe { source.Anonymous.sourceMode };
            if source_mode.width == mode.width && source_mode.height == mode.height {
                return Ok(true);
            }
        }
    }
    Ok(false)
}

fn wait_target(
    target: Target,
    mode: Option<Mode>,
    desired: bool,
    device: Option<&Device>,
) -> Result<f64> {
    let started = Instant::now();
    let mut last_ping = started;
    loop {
        if present(target, mode)? == desired {
            return Ok(started.elapsed().as_secs_f64() * 1000.0);
        }
        if started.elapsed() > Duration::from_secs(6) {
            return Err("DisplayConfig transition exceeded six seconds".into());
        }
        if last_ping.elapsed() >= Duration::from_secs(1) {
            if let Some(device) = device {
                device.ping()?;
            }
            last_ping = Instant::now();
        }
        thread::sleep(Duration::from_millis(100));
    }
}

fn new_guid() -> Result<[u8; 16]> {
    // SAFETY: CoCreateGuid has no caller-owned pointer and does not need COM initialization.
    let guid = unsafe { CoCreateGuid() }.map_err(|error| api_error("CoCreateGuid", error))?;
    let mut bytes = [0; 16];
    bytes[..4].copy_from_slice(&guid.data1.to_le_bytes());
    bytes[4..6].copy_from_slice(&guid.data2.to_le_bytes());
    bytes[6..8].copy_from_slice(&guid.data3.to_le_bytes());
    bytes[8..].copy_from_slice(&guid.data4);
    Ok(bytes)
}

fn parse_guid(value: &str) -> Result<[u8; 16]> {
    if value.len() != 32 || !value.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err("Invalid internal run identity".into());
    }
    let mut guid = [0; 16];
    for (index, byte) in guid.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&value[index * 2..index * 2 + 2], 16)
            .map_err(|_| "Invalid identity")?;
    }
    if guid == [0; 16] {
        return Err("Zero identity refused".into());
    }
    Ok(guid)
}

fn normal_test(device: &Device) -> Result<serde_json::Value> {
    let mut rows = Vec::new();
    for mode in protocol::MODES {
        let guid = new_guid()?;
        let started = Instant::now();
        let target = device.add(mode, guid)?;
        let check: Result<f64> = (|| {
            wait_target(target, Some(mode), true, Some(device))?;
            let arrival_ms = started.elapsed().as_secs_f64() * 1000.0;
            for _ in 0..3 {
                device.ping()?;
                thread::sleep(Duration::from_secs(1));
            }
            if !present(target, Some(mode))? {
                return Err("Display vanished while pinging".into());
            }
            Ok(arrival_ms)
        })();
        let removal_started = Instant::now();
        let cleanup = device
            .remove(&guid)
            .and_then(|()| wait_target(target, None, false, None).map(|_| ()));
        if cleanup.is_err() {
            return Err("Owned display cleanup failed; inspect the dedicated driver".into());
        }
        let arrival_ms = check?;
        rows.push(serde_json::json!({"mode": mode, "arrival_ms": arrival_ms,
            "remove_ms": removal_started.elapsed().as_secs_f64() * 1000.0, "ping_count": 3}));
    }
    Ok(serde_json::json!({"scenario":"normal", "modes":rows, "cleanup_confirmed":true}))
}

fn hold_child(guid: [u8; 16]) -> Result<()> {
    let device = Device::open()?;
    let mode = protocol::MODES[0];
    let target = device.add(mode, guid)?;
    let result = (|| {
        wait_target(target, Some(mode), true, Some(&device))?;
        device.ping()?;
        println!(
            "{}",
            serde_json::to_string(&target).map_err(|_| "Target serialization failed")?
        );
        std::io::stdout()
            .flush()
            .map_err(|_| "Ready receipt flush failed")?;
        for _ in 0..30 {
            thread::sleep(Duration::from_secs(1));
            device.ping()?;
        }
        Ok(())
    })();
    let cleanup = device.remove(&guid);
    result.and(cleanup)
}

fn watchdog_test(device: &Device) -> Result<serde_json::Value> {
    let guid = new_guid()?;
    let encoded = guid.iter().map(|b| format!("{b:02x}")).collect::<String>();
    let executable = std::env::current_exe().map_err(|_| "Cannot locate this probe")?;
    let mut child = Command::new(executable)
        .args(["hold-child", "--owner-ready", &encoded])
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|_| "Cannot start owned watchdog child")?;
    let Some(stdout) = child.stdout.take() else {
        let _ = child.kill();
        let _ = child.wait();
        return Err("Child stdout unavailable".into());
    };
    let (sender, receiver) = mpsc::channel();
    let reader = thread::spawn(move || {
        let mut line = String::new();
        let result = std::io::BufReader::new(stdout).read_line(&mut line);
        let _ = sender.send(
            result
                .ok()
                .filter(|length| *length <= 1024)
                .and_then(|_| serde_json::from_str::<Target>(&line).ok()),
        );
    });
    let ready = receiver
        .recv_timeout(Duration::from_secs(15))
        .ok()
        .flatten();
    let active_before_kill = ready
        .map(|target| present(target, Some(protocol::MODES[0])))
        .transpose();
    let started = Instant::now();
    // kill() targets only the child Process handle we just created, never a PID search.
    let killed = child.kill();
    let waited = child.wait();
    let _ = reader.join();
    let result = (|| {
        killed.map_err(|_| "Owned child termination failed")?;
        waited.map_err(|_| "Owned child reaping failed")?;
        let target = ready.ok_or("Watchdog child never confirmed an active target")?;
        if active_before_kill? != Some(true) {
            return Err("Child target was not active immediately before termination".into());
        }
        wait_target(target, None, false, None)?;
        let elapsed_ms = started.elapsed().as_secs_f64() * 1000.0;
        Ok(
            serde_json::json!({"scenario":"watchdog", "child_killed":true,
            "removal_after_kill_ms":elapsed_ms, "within_four_seconds":elapsed_ms <= 4000.0,
            "cleanup_confirmed":true, "window_return_inspected":false}),
        )
    })();
    if result.is_err() {
        let _ = device.remove(&guid);
    }
    result
}

pub fn run() -> Result<()> {
    // SAFETY: First platform operation in the process, before any windows or coordinate queries.
    unsafe { SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2) }
        .map_err(|e| api_error("PMv2", e))?;
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args == ["inventory"] {
        let count = interfaces()?.len();
        let (paths, _) = topology()?;
        println!(
            "{}",
            serde_json::json!({"schema":1, "sudovda_interfaces":count,
            "active_paths":paths.iter().filter(|p| p.flags & DISPLAYCONFIG_PATH_ACTIVE != 0).count(),
            "driver_ioctls_sent":0, "mutated":false})
        );
        return Ok(());
    }
    if args.len() == 3 && args[0] == "hold-child" && args[1] == "--owner-ready" {
        return hold_child(parse_guid(&args[2])?);
    }
    if args.len() != 2
        || args[1] != "--owner-ready"
        || !["normal", "watchdog"].contains(&args[0].as_str())
    {
        return Err(
            "Use run.ps1 -Scenario inventory, or owner-reserved normal/watchdog tests".into(),
        );
    }
    let device = Device::open()?;
    let result = if args[0] == "normal" {
        normal_test(&device)?
    } else {
        watchdog_test(&device)?
    };
    println!("{result}");
    Ok(())
}
