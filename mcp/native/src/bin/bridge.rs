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
    use std::{
        io::{self, BufReader, Read, Write},
        sync::mpsc,
        thread,
        time::{Duration, Instant},
    };
    use vw_mcp_native::{REQUEST_BYTES, WIRE_BYTES, framing, windows};
    let image = vw_mcp_native::app_image::AppImage::installed()?;
    let sid = windows::own_sid()?;
    let mut pipe = windows::client_pipe(&sid);
    if pipe.is_err() {
        // The compiled complete app inventory supplies the launcher. A private
        // Job owns this exact suspended child until readiness or bounded cleanup.
        let launch = vw_mcp_native::app_launch::AppLaunch::start(&image.executable)?;
        let start = Instant::now();
        while start.elapsed() < Duration::from_secs(15) {
            thread::sleep(Duration::from_millis(50));
            pipe = windows::client_pipe(&sid);
            if pipe.is_ok() {
                break;
            }
        }
        if pipe.is_ok() {
            launch.detach()?;
        } else {
            launch.abort()?;
            return Err("app_not_ready");
        }
    }
    let mut pipe = pipe?;
    let (send, receive) = mpsc::sync_channel::<Vec<u8>>(2);
    thread::spawn(move || {
        let mut input = BufReader::new(io::stdin());
        while let Ok(Some(mut bytes)) = framing::line(&mut input, REQUEST_BYTES) {
            if framing::parse(&bytes).is_err() {
                break;
            }
            bytes.push(b'\n');
            if send.send(bytes).is_err() {
                break;
            }
        }
    });
    let mut output = io::stdout();
    let mut bytes = Vec::new();
    let mut buffer = [0u8; 8192];
    let mut outgoing: Option<(Vec<u8>, usize, Instant)> = None;
    let output_deadline =
        vw_mcp_native::write_deadline::WriteDeadline::new(Duration::from_secs(15), || {
            // SAFETY: exit only this bridge. Windows closes its pipe handle, allowing
            // the host to retire this connection and revoke its capture grants.
            unsafe {
                windows_sys::Win32::System::Threading::ExitProcess(1);
            }
        })?;
    loop {
        if outgoing.is_none() {
            match receive.try_recv() {
                Ok(value) => outgoing = Some((value, 0, Instant::now())),
                Err(mpsc::TryRecvError::Disconnected) => break,
                Err(mpsc::TryRecvError::Empty) => {}
            }
        }
        if let Some((value, at, start)) = &mut outgoing {
            if start.elapsed() > Duration::from_secs(15) {
                return Err("write_timeout");
            }
            match pipe.write(&value[*at..]) {
                Ok(0) => {}
                Ok(n) => {
                    *at += n;
                    if *at == value.len() {
                        outgoing = None;
                    }
                }
                Err(e)
                    if e.raw_os_error() == Some(232) || e.kind() == io::ErrorKind::WouldBlock => {}
                Err(_) => break,
            }
        }
        match pipe.read(&mut buffer) {
            Ok(0) => break,
            Ok(n) => {
                for b in &buffer[..n] {
                    if *b == b'\n' {
                        framing::parse(&bytes)?;
                        output_deadline.frame(&mut output, &bytes)?;
                        bytes.clear();
                    } else {
                        if bytes.len() >= WIRE_BYTES {
                            return Err("frame_limit");
                        }
                        bytes.push(*b);
                    }
                }
            }
            Err(e) if e.raw_os_error() == Some(232) || e.kind() == io::ErrorKind::WouldBlock => {}
            Err(_) => break,
        }
        thread::sleep(Duration::from_millis(2));
    }
    Ok(())
}
