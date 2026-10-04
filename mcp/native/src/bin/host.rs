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
    use fs2::FileExt;
    use serde_json::{Value, json};
    use std::{
        fs::{self, OpenOptions},
        io::{self, BufReader, Read, Write},
        os::windows::process::CommandExt,
        path::PathBuf,
        process::{Command, Stdio},
        sync::{
            Arc, Mutex,
            atomic::{AtomicBool, Ordering},
            mpsc,
        },
        thread,
        time::{Duration, Instant},
    };
    use vw_mcp_native::{
        REQUEST_BYTES, Result, WIRE_BYTES, framing, package_reader::no_redirect, windows,
    };
    #[derive(serde::Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Start {
        kind: String,
        state_directory: String,
    }
    let mut parent = BufReader::new(io::stdin());
    let start: Start = serde_json::from_slice(&framing::line(&mut parent, 8192)?.ok_or("start")?)
        .map_err(|_| "start")?;
    if start.kind != "start" {
        return Err("start");
    }
    let state = PathBuf::from(start.state_directory);
    no_redirect(&state)?;
    if !state.is_dir() {
        return Err("state");
    }
    // Existing state files may contain only nonsecret port/lock data.
    let lock_path = state.join("mcp-owner.lock");
    if lock_path.exists() {
        no_redirect(&lock_path)?;
    }
    let lock = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(lock_path)
        .map_err(|_| "lock")?;
    lock.try_lock_exclusive().map_err(|_| "already_running")?;
    let port_path = state.join("mcp-port-v1");
    let port = if port_path.exists() {
        no_redirect(&port_path)?;
        let mut bytes = Vec::new();
        fs::File::open(&port_path)
            .map_err(|_| "port")?
            .take(6)
            .read_to_end(&mut bytes)
            .map_err(|_| "port")?;
        if bytes.len() > 5 {
            return Err("port");
        }
        let port = std::str::from_utf8(&bytes)
            .map_err(|_| "port")?
            .parse::<u16>()
            .map_err(|_| "port")?;
        if port == 0 {
            return Err("port");
        }
        port
    } else {
        0
    };
    let directory = std::env::current_exe()
        .map_err(|_| "executable")?
        .parent()
        .ok_or("executable")?
        .to_owned();
    let node = directory.join("node.exe");
    let script = directory.join("mcp/src/main.mjs");
    let verifier = directory.join("vw-mcp-package.exe");
    for path in [&node, &script, &verifier] {
        no_redirect(path)?;
        if !path.is_file() {
            return Err("runtime");
        }
    }
    let sid = windows::own_sid()?;
    let first_pipe = windows::server_pipe(&sid, true)?;
    let _job = windows::own_job()?;
    let token = windows::token()?;
    let mut command = Command::new(node);
    windows::node_environment(&mut command)?;
    let mut child = command
        .arg("--max-old-space-size=512")
        .arg(script)
        .creation_flags(0x08000000)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|_| "sdk")?;
    let mut sdk_input = child.stdin.take().ok_or("sdk")?;
    let sdk_output = child.stdout.take().ok_or("sdk")?;
    let (events, receive) = mpsc::sync_channel::<Value>(16);
    let stopped = Arc::new(AtomicBool::new(false));
    // A blocked inherited-pipe write has one fixed watchdog, not one thread per
    // request. Exiting this Job owner closes all exact child processes.
    let writing = Arc::new(Mutex::new(None::<Instant>));
    let watch = writing.clone();
    let stop = stopped.clone();
    thread::spawn(move || {
        while !stop.load(Ordering::Acquire) {
            thread::sleep(Duration::from_millis(100));
            let overdue = watch
                .lock()
                .map(|v| v.is_some_and(|t| t.elapsed() > Duration::from_secs(30)))
                .unwrap_or(true);
            if overdue {
                /* SAFETY: terminate only this owned supervisor; Job cleanup owns its children. */
                unsafe {
                    windows_sys::Win32::System::Threading::ExitProcess(1);
                }
            }
        }
    });
    let write = |out: &mut dyn Write, value: &Value| -> Result<()> {
        *writing.lock().map_err(|_| "lock")? = Some(Instant::now());
        let bytes = serde_json::to_vec(value).map_err(|_| "json")?;
        let result = if bytes.len() > WIRE_BYTES {
            Err("frame_limit")
        } else {
            out.write_all(&bytes)
                .and_then(|()| out.write_all(b"\n"))
                .and_then(|()| out.flush())
                .map_err(|_| "write")
        };
        *writing.lock().map_err(|_| "lock")? = None;
        result
    };
    *writing.lock().map_err(|_| "lock")? = Some(Instant::now());
    let initialized = framing::write_init(&mut sdk_input, &token, port, &verifier);
    *writing.lock().map_err(|_| "lock")? = None;
    initialized?;
    drop(token);
    for (mut input, origin) in [
        (Box::new(parent) as Box<dyn io::BufRead + Send>, "owner"),
        (Box::new(BufReader::new(sdk_output)), "sdk"),
    ] {
        let sender = events.clone();
        thread::spawn(move || {
            loop {
                let value = framing::line(&mut input, WIRE_BYTES)
                    .and_then(|bytes| bytes.map(|b| framing::parse(&b)).transpose());
                match value {
                    Ok(Some(message)) => {
                        if sender
                            .try_send(json!({"origin":origin,"message":message}))
                            .is_err()
                        {
                            let _ = sender.send(json!({"origin":"end"}));
                            break;
                        }
                    }
                    _ => {
                        let _ = sender.send(json!({"origin":"end"}));
                        break;
                    }
                }
            }
        });
    }
    let (accepted, connections) =
        mpsc::sync_channel::<(String, u32, mpsc::SyncSender<Value>, Arc<AtomicBool>)>(4);
    let events_pipe = events.clone();
    let stop = stopped.clone();
    thread::spawn(move || {
        let mut listener = first_pipe;
        let active = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        while !stop.load(Ordering::Acquire) {
            match windows::connect_server(&listener, &sid) {
                Ok(false) => {
                    thread::sleep(Duration::from_millis(20));
                    continue;
                }
                Err(_) => match windows::server_pipe(&sid, false) {
                    Ok(next) => {
                        listener = next;
                        continue;
                    }
                    Err(_) => {
                        let _ = events_pipe.try_send(json!({"origin":"end"}));
                        return;
                    }
                },
                Ok(true) => {}
            }
            let next = match windows::server_pipe(&sid, false) {
                Ok(v) => v,
                Err(_) => {
                    let _ = events_pipe.try_send(json!({"origin":"end"}));
                    return;
                }
            };
            let mut pipe = std::mem::replace(&mut listener, next);
            if active.load(Ordering::Acquire) >= 4 {
                drop(pipe);
                continue;
            }
            let id = match vw_mcp_native::nonce() {
                Ok(v) => v,
                Err(_) => return,
            };
            let (out, output) = mpsc::sync_channel::<Value>(2);
            let closed = Arc::new(AtomicBool::new(false));
            let pid = match windows::client_pid(&pipe) {
                Ok(pid) => pid,
                Err(_) => continue,
            };
            if accepted
                .try_send((id.clone(), pid, out, closed.clone()))
                .is_err()
            {
                continue;
            }
            active.fetch_add(1, Ordering::AcqRel);
            let active = active.clone();
            let send = events_pipe.clone();
            let stop = stop.clone();
            thread::spawn(move || {
                let mut input = Vec::new();
                let mut buffer = [0u8; 8192];
                let mut outgoing: Option<(Vec<u8>, usize, Instant)> = None;
                while !closed.load(Ordering::Acquire) && !stop.load(Ordering::Acquire) {
                    if outgoing.is_none() {
                        match output.try_recv() {
                            Ok(value) => match serde_json::to_vec(&value) {
                                Ok(mut bytes) if bytes.len() <= WIRE_BYTES => {
                                    bytes.push(b'\n');
                                    outgoing = Some((bytes, 0, Instant::now()));
                                }
                                _ => break,
                            },
                            Err(mpsc::TryRecvError::Disconnected) => break,
                            Err(mpsc::TryRecvError::Empty) => {}
                        }
                    }
                    if let Some((bytes, at, started)) = &mut outgoing {
                        if started.elapsed() > Duration::from_secs(15) {
                            break;
                        }
                        match pipe.write(&bytes[*at..]) {
                            Ok(0) => {}
                            Ok(n) => {
                                *at += n;
                                if *at == bytes.len() {
                                    outgoing = None;
                                }
                            }
                            Err(e)
                                if e.raw_os_error() == Some(232)
                                    || e.kind() == io::ErrorKind::WouldBlock => {}
                            Err(_) => break,
                        }
                    }
                    match pipe.read(&mut buffer) {
                        Ok(0) => break,
                        Ok(n) => {
                            let mut refused = false;
                            for b in &buffer[..n] {
                                if *b == b'\n' {
                                    let parsed = framing::parse(&input);
                                    input.clear();
                                    match parsed{Ok(message)=>{if send.try_send(json!({"origin":"pipe","connection":id,"message":message})).is_err(){refused=true;break;}},Err(_)=>{refused=true;break;}}
                                } else {
                                    if input.len() >= REQUEST_BYTES {
                                        refused = true;
                                        break;
                                    }
                                    input.push(*b);
                                }
                            }
                            if refused {
                                break;
                            }
                        }
                        Err(e)
                            if e.raw_os_error() == Some(232)
                                || e.kind() == io::ErrorKind::WouldBlock => {}
                        Err(_) => break,
                    }
                    thread::sleep(Duration::from_millis(2));
                }
                closed.store(true, Ordering::Release);
                active.fetch_sub(1, Ordering::AcqRel);
                let _ = send.try_send(json!({"origin":"closed","connection":id}));
            });
        }
    });
    let mut pipes =
        std::collections::BTreeMap::<String, (mpsc::SyncSender<Value>, Arc<AtomicBool>)>::new();
    let mut ready = false;
    let launched = Instant::now();
    let mut output = io::stdout();
    let reap = |pipes: &mut std::collections::BTreeMap<
        String,
        (mpsc::SyncSender<Value>, Arc<AtomicBool>),
    >,
                sdk: &mut dyn Write,
                out: &mut dyn Write|
     -> Result<()> {
        let retired: Vec<_> = pipes
            .iter()
            .filter(|(_, (_, closed))| closed.load(Ordering::Acquire))
            .map(|(id, _)| id.clone())
            .collect();
        for id in retired {
            pipes.remove(&id);
            write(sdk, &json!({"kind":"pipe/closed","connection":id}))?;
            write(out, &json!({"kind":"agent/closed","connection":id}))?;
        }
        Ok(())
    };
    loop {
        reap(&mut pipes, &mut sdk_input, &mut output)?;
        while let Ok((id, pid, sender, closed)) = connections.try_recv() {
            reap(&mut pipes, &mut sdk_input, &mut output)?;
            if closed.load(Ordering::Acquire) {
                continue;
            }
            if pipes.len() >= 4 {
                closed.store(true, Ordering::Release);
                continue;
            }
            write(&mut sdk_input, &json!({"kind":"pipe/open","connection":id}))?;
            write(
                &mut output,
                &json!({"kind":"agent/open","connection":id,"bridge_pid":pid}),
            )?;
            pipes.insert(id, (sender, closed));
        }
        let event = match receive.recv_timeout(Duration::from_millis(20)) {
            Ok(v) => v,
            Err(mpsc::RecvTimeoutError::Timeout) => {
                if !ready && launched.elapsed() > Duration::from_secs(30) {
                    break;
                }
                if child.try_wait().map_err(|_| "sdk")?.is_some() {
                    break;
                }
                continue;
            }
            Err(_) => break,
        };
        // Accept notices are enqueued before a pipe worker can enqueue bytes.
        // Drain again after recv: the first initialize must never overtake open.
        reap(&mut pipes, &mut sdk_input, &mut output)?;
        while let Ok((id, pid, sender, closed)) = connections.try_recv() {
            reap(&mut pipes, &mut sdk_input, &mut output)?;
            if closed.load(Ordering::Acquire) {
                continue;
            }
            if pipes.len() >= 4 {
                closed.store(true, Ordering::Release);
                continue;
            }
            write(&mut sdk_input, &json!({"kind":"pipe/open","connection":id}))?;
            write(
                &mut output,
                &json!({"kind":"agent/open","connection":id,"bridge_pid":pid}),
            )?;
            pipes.insert(id, (sender, closed));
        }
        match event["origin"].as_str() {
            Some("owner") => {
                let m = &event["message"];
                if m["kind"] == "shutdown" {
                    break;
                }
                if !matches!(
                    m["kind"].as_str(),
                    Some(
                        "publish"
                            | "unpublish"
                            | "preview/claude"
                            | "grant"
                            | "revoke"
                            | "push/claude"
                            | "owner/reply"
                    )
                ) {
                    break;
                }
                write(&mut sdk_input, m)?;
            }
            Some("pipe") => {
                let Some(id) = event["connection"].as_str() else {
                    break;
                };
                if pipes.contains_key(id) {
                    write(
                        &mut sdk_input,
                        &json!({"kind":"pipe/message","connection":id,"message":event["message"]}),
                    )?;
                }
            }
            Some("closed") => {
                let Some(id) = event["connection"].as_str() else {
                    break;
                };
                if pipes.remove(id).is_some() {
                    write(
                        &mut sdk_input,
                        &json!({"kind":"pipe/closed","connection":id}),
                    )?;
                    write(&mut output, &json!({"kind":"agent/closed","connection":id}))?;
                }
            }
            Some("sdk") => {
                let m = &event["message"];
                match m["kind"].as_str() {
                    Some("ready") => {
                        if ready {
                            break;
                        }
                        let bound = m["port"]
                            .as_u64()
                            .filter(|v| *v > 0 && *v <= 65535)
                            .ok_or("port")?;
                        if port != 0 && bound != u64::from(port) {
                            return Err("port_changed");
                        }
                        if port == 0 {
                            let mut file = OpenOptions::new()
                                .create_new(true)
                                .write(true)
                                .open(&port_path)
                                .map_err(|_| "port_publish")?;
                            file.write_all(bound.to_string().as_bytes())
                                .and_then(|()| file.sync_all())
                                .map_err(|_| "port_publish")?;
                        }
                        ready = true;
                        write(&mut output, m)?;
                    }
                    Some("pipe/send") => {
                        let Some(id) = m["connection"].as_str() else {
                            break;
                        };
                        if let Some((send, closed)) = pipes.get(id)
                            && send.try_send(m["message"].clone()).is_err()
                        {
                            closed.store(true, Ordering::Release);
                        }
                    }
                    Some("pipe/close") => {
                        if let Some(id) = m["connection"].as_str()
                            && let Some((_, closed)) = pipes.get(id)
                        {
                            closed.store(true, Ordering::Release);
                        }
                    }
                    Some("owner/request" | "owner/cancel" | "control/reply") => {
                        write(&mut output, m)?
                    }
                    _ => break,
                }
            }
            _ => break,
        }
    }
    stopped.store(true, Ordering::Release);
    for (_, (_, closed)) in pipes {
        closed.store(true, Ordering::Release);
    }
    let _ = child.kill();
    let _ = child.wait();
    Ok(())
}
