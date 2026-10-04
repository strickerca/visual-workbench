//! Owner-only App Server supervisor. This binary is never a callable MCP tool.
#[cfg(not(windows))]
fn main() {
    std::process::exit(1);
}
#[cfg(windows)]
fn main() {
    if run().is_err() {
        std::process::exit(1);
    }
}
#[cfg(windows)]
fn run() -> vw_mcp_native::Result<()> {
    use serde_json::{Value, json};
    use sha2::{Digest, Sha256};
    use std::{
        fs::{self, File, OpenOptions},
        io::{self, BufReader, Read},
        os::windows::{fs::OpenOptionsExt, process::CommandExt},
        path::{Path, PathBuf},
        process::{Command, Stdio},
        sync::mpsc,
        thread,
        time::{Duration, Instant},
    };
    use vw_mcp_native::{
        Result, framing, package_reader::no_redirect, write_deadline::WriteDeadline,
    };
    #[derive(serde::Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Start {
        kind: String,
        executable: String,
        binary_sha256: String,
        work: String,
        profiles: Vec<Value>,
    }
    fn file(path: &Path) -> Result<File> {
        no_redirect(path)?;
        let result = OpenOptions::new()
            .read(true)
            .share_mode(1)
            .open(path)
            .map_err(|_| "file")?;
        if !result.metadata().map_err(|_| "file")?.is_file() {
            return Err("file");
        }
        Ok(result)
    }
    fn directories(path: &Path, pins: &mut Vec<File>) -> Result<()> {
        for parent in path.ancestors().collect::<Vec<_>>().into_iter().rev() {
            no_redirect(parent)?;
            // FILE_LIST_DIRECTORY | FILE_READ_ATTRIBUTES (0x81) is required:
            // READ_ATTRIBUTES alone does not pin directory rename on Windows.
            let pin = OpenOptions::new()
                .access_mode(0x81)
                .share_mode(1)
                .custom_flags(0x0200_0000)
                .open(parent)
                .map_err(|_| "directory_pin")?;
            if !pin.metadata().map_err(|_| "directory")?.is_dir() {
                return Err("directory");
            }
            pins.push(pin);
        }
        Ok(())
    }
    fn hash(file: &mut File) -> Result<String> {
        let length = file.metadata().map_err(|_| "file")?.len();
        if length == 0 || length > 256 * 1024 * 1024 {
            return Err("file_size");
        }
        let mut sha = Sha256::new();
        let mut buffer = [0u8; 65536];
        let mut count = 0u64;
        loop {
            let n = file.read(&mut buffer).map_err(|_| "file")?;
            if n == 0 {
                break;
            }
            count += n as u64;
            if count > length {
                return Err("file_changed");
            }
            sha.update(&buffer[..n]);
        }
        if count != length {
            return Err("file_changed");
        }
        Ok(sha
            .finalize()
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect())
    }
    fn pin_stage(work: &Path, stage: &Path, pins: &mut Vec<File>) -> Result<()> {
        if stage.parent() != Some(work)
            || !stage.file_name().and_then(|v| v.to_str()).is_some_and(|v| {
                v.starts_with("stage-")
                    && v.len() == 42
                    && v[6..].bytes().all(|b| b.is_ascii_hexdigit() || b == b'-')
            })
        {
            return Err("stage");
        }
        directories(stage, pins)?;
        let mut count = 0;
        let mut total = 0;
        let mut todo = vec![stage.to_owned()];
        while let Some(folder) = todo.pop() {
            for entry in fs::read_dir(folder).map_err(|_| "stage")? {
                let entry = entry.map_err(|_| "stage")?;
                let path = entry.path();
                no_redirect(&path)?;
                let metadata = fs::symlink_metadata(&path).map_err(|_| "stage")?;
                if metadata.is_dir() {
                    if path != stage.join("images") {
                        return Err("stage_directory");
                    }
                    directories(&path, pins)?;
                    todo.push(path);
                } else {
                    count += 1;
                    if count > 69 {
                        return Err("stage_limit");
                    }
                    let pin = file(&path)?;
                    total += pin.metadata().map_err(|_| "stage")?.len();
                    if total > 32 * 1024 * 1024 {
                        return Err("stage_limit");
                    }
                    pins.push(pin);
                }
            }
        }
        Ok(())
    }
    enum Event {
        Owner(Value),
        Node(Value),
        End,
    }
    let mut lifetime = PinnedJob {
        job: Some(codex_job()?),
        pins: Vec::new(),
    };
    let (send, receive) = mpsc::sync_channel::<Event>(8);
    let parent = send.clone();
    thread::spawn(move || {
        let mut input = BufReader::new(io::stdin());
        while let Ok(Some(bytes)) = framing::line(&mut input, 2 * 1024 * 1024) {
            match framing::parse(&bytes) {
                Ok(value) => {
                    if parent.try_send(Event::Owner(value)).is_err() {
                        std::process::exit(1);
                    }
                }
                Err(_) => break,
            }
        }
        let _ = parent.try_send(Event::End);
    });
    let first = match receive
        .recv_timeout(Duration::from_secs(10))
        .map_err(|_| "start")?
    {
        Event::Owner(value) => value,
        _ => return Err("start"),
    };
    let start: Start = serde_json::from_value(first).map_err(|_| "start")?;
    if start.kind != "start"
        || start.profiles.len() > 16
        || start.binary_sha256.len() != 64
        || !start.binary_sha256.bytes().all(|b| b.is_ascii_hexdigit())
    {
        return Err("start");
    }
    let executable = PathBuf::from(start.executable);
    let work = PathBuf::from(start.work);
    no_redirect(&work)?;
    if !work.is_dir() || fs::read_dir(&work).map_err(|_| "work")?.next().is_some() {
        return Err("work");
    }
    directories(executable.parent().ok_or("executable")?, &mut lifetime.pins)?;
    directories(&work, &mut lifetime.pins)?;
    let mut executable_pin = file(&executable)?;
    if hash(&mut executable_pin)? != start.binary_sha256 {
        return Err("runtime_changed");
    }
    lifetime.pins.push(executable_pin);
    let root = std::env::current_exe()
        .map_err(|_| "runtime")?
        .parent()
        .ok_or("runtime")?
        .to_owned();
    let node = root.join("node.exe");
    let script = root.join("mcp/codex/main.mjs");
    let verifier = root.join("vw-mcp-package.exe");
    for path in [&node, &script, &verifier] {
        no_redirect(path)?;
        if !path.is_file() {
            return Err("runtime");
        }
    }
    // The desktop retains the complete packaged runtime inventory while this
    // helper is alive. Pass only Windows/user-location variables; never API keys
    // or arbitrary NODE_OPTIONS/CODEX_HOME overrides. Codex owns its normal OS
    // credential lookup. Workbench neither reads nor exports that credential.
    let mut command = Command::new(node);
    command
        .arg("--max-old-space-size=512")
        .arg(script)
        .creation_flags(0x0800_0000)
        .env_clear();
    for key in [
        "SystemRoot",
        "WINDIR",
        "USERPROFILE",
        "APPDATA",
        "LOCALAPPDATA",
        "TEMP",
        "TMP",
        "HOMEDRIVE",
        "HOMEPATH",
        "PATH",
    ] {
        if let Some(value) = std::env::var_os(key) {
            command.env(key, value);
        }
    }
    let mut child = command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|_| "runtime")?;
    let mut input = child.stdin.take().ok_or("runtime")?;
    let output = child.stdout.take().ok_or("runtime")?;
    let node_send = send.clone();
    thread::spawn(move || {
        let mut output = BufReader::new(output);
        while let Ok(Some(bytes)) = framing::line(&mut output, 4 * 1024 * 1024) {
            match framing::parse(&bytes) {
                Ok(value) => {
                    if node_send.try_send(Event::Node(value)).is_err() {
                        std::process::exit(1);
                    }
                }
                Err(_) => break,
            }
        }
        let _ = node_send.try_send(Event::End);
    });
    let deadline = WriteDeadline::new(Duration::from_secs(15), || std::process::exit(1))?;
    let mut stdout = io::stdout();
    let write = |mut out: &mut dyn io::Write, value: &Value| -> Result<()> {
        let bytes = serde_json::to_vec(value).map_err(|_| "frame")?;
        if bytes.len() > 4 * 1024 * 1024 {
            return Err("frame_limit");
        }
        deadline.frame(&mut out, &bytes)
    };
    write(
        &mut input,
        &json!({"kind":"start","executable":executable,"binarySha256":start.binary_sha256,"work":work,"verifier":verifier,"profiles":start.profiles}),
    )?;
    let mut heartbeat = Instant::now();
    let mut stages = 0;
    loop {
        match receive.recv_timeout(Duration::from_millis(100)) {
            Ok(Event::Owner(value)) => {
                if value["kind"] == "shutdown" {
                    break;
                }
                if !matches!(
                    value["kind"].as_str(),
                    Some("threads" | "select" | "preview" | "send" | "interrupt")
                ) {
                    return Err("command");
                }
                write(&mut input, &value)?;
            }
            Ok(Event::Node(value)) => {
                heartbeat = Instant::now();
                match value["kind"].as_str() {
                    Some("heartbeat") => {}
                    Some("stage/pin") => {
                        if stages >= 2 {
                            return Err("stage_limit");
                        }
                        let stage = PathBuf::from(value["directory"].as_str().ok_or("stage")?);
                        pin_stage(&work, &stage, &mut lifetime.pins)?;
                        stages += 1;
                        write(
                            &mut input,
                            &json!({"kind":"stage/pinned","directory":stage}),
                        )?;
                    }
                    Some(
                        "ready" | "reply" | "refused" | "runtime/refused" | "turn/completed"
                        | "approval/refused",
                    ) => write(&mut stdout, &value)?,
                    _ => return Err("runtime_protocol"),
                }
            }
            Ok(Event::End) => break,
            Err(mpsc::RecvTimeoutError::Timeout) => {
                if heartbeat.elapsed() > Duration::from_secs(45)
                    || child.try_wait().map_err(|_| "runtime")?.is_some()
                {
                    break;
                }
            }
            Err(_) => break,
        }
    }
    // Kill before closing potentially writer-locked pipes; Job owns every
    // descendant. Directory/file pins outlive the entire supervisor process.
    let _ = child.kill();
    let expires = Instant::now() + Duration::from_secs(5);
    while child.try_wait().map_err(|_| "cleanup")?.is_none() {
        if Instant::now() >= expires {
            return Err("cleanup_uncertain");
        }
        thread::sleep(Duration::from_millis(20));
    }
    drop(lifetime);
    Ok(())
}

#[cfg(windows)]
fn codex_job() -> vw_mcp_native::Result<vw_mcp_native::windows::Handle> {
    use vw_mcp_native::windows::Handle;
    use windows_sys::Win32::System::{JobObjects::*, Threading::GetCurrentProcess};
    // SAFETY: unnamed noninherited Job; live initialized exact-size limits.
    let job = Handle::new(unsafe { CreateJobObjectW(std::ptr::null(), std::ptr::null()) })?;
    let mut limits = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
    limits.BasicLimitInformation.LimitFlags =
        JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE | JOB_OBJECT_LIMIT_ACTIVE_PROCESS;
    // Supervisor + Node + App Server + bounded agent tool descendants. This is
    // containment, not an additional approval grant or sandbox-policy override.
    limits.BasicLimitInformation.ActiveProcessLimit = 16;
    if unsafe {
        SetInformationJobObject(
            job.raw(),
            JobObjectExtendedLimitInformation,
            (&limits as *const JOBOBJECT_EXTENDED_LIMIT_INFORMATION).cast(),
            std::mem::size_of_val(&limits) as u32,
        )
    } == 0
        || unsafe { AssignProcessToJobObject(job.raw(), GetCurrentProcess()) } == 0
    {
        return Err("job");
    }
    Ok(job)
}

#[cfg(windows)]
struct PinnedJob {
    job: Option<vw_mcp_native::windows::Handle>,
    pins: Vec<std::fs::File>,
}
#[cfg(windows)]
impl Drop for PinnedJob {
    fn drop(&mut self) {
        // Closing the sole Job handle terminates this supervisor and every
        // descendant before OS teardown releases the retained file pins.
        // This order also applies to every early error path after spawning.
        drop(self.job.take());
    }
}
