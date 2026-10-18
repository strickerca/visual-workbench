//! Explicit opt-in hostile fixture. Never package this binary as production.
//! The central runner binds its exact build hash and supplies only owned paths.
#[cfg(windows)]
fn main() {
    std::panic::set_hook(Box::new(|_| {}));
    std::process::exit(run().unwrap_or(91));
}
#[cfg(not(windows))]
fn main() { std::process::exit(91); }

#[cfg(windows)]
fn run() -> Result<i32, Box<dyn std::error::Error>> {
    use std::{io::Write, net::{IpAddr, Ipv4Addr, SocketAddr, TcpStream}, time::Duration};
    use vw_document_protocol::{Operation, ReplyWriter};
    use vw_document_win::{DocumentFormat, ResultMetadata};
    #[derive(serde::Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Probe { mode: String, read_path: String, write_path: String, loopback_port: u16 }
    let _memory = vw_document_win::verify_worker_token()?;
    let job = vw_document_protocol::read_job(&mut std::io::stdin().lock())?;
    if job.source.len() > 4096 { return Ok(91); }
    let probe: Probe = serde_json::from_slice(&job.source)?;
    if probe.read_path.len() > 1024 || probe.write_path.len() > 1024 { return Ok(91); }
    if probe.mode == "timeout" { std::thread::sleep(Duration::from_secs(60)); return Ok(91); }
    if probe.mode == "stderr_flood" {
        let mut stderr = std::io::stderr().lock();
        for _ in 0..32 { stderr.write_all(&[42; 4096])?; }
        return Ok(91);
    }
    if probe.mode != "isolation" { return Ok(91); }
    let read = std::fs::File::open(&probe.read_path).is_ok();
    let write = std::fs::OpenOptions::new().create_new(true).write(true).open(&probe.write_path).is_ok();
    let network = TcpStream::connect_timeout(&SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), probe.loopback_port), Duration::from_millis(300)).is_ok();
    // Use only a fixed Windows-owned no-op child, never a supplied command.
    let mut system = [0_u16; 32_768];
    // SAFETY: fixed bounded buffer, read-only trusted Windows directory query.
    let length = unsafe { windows_sys::Win32::System::SystemInformation::GetSystemDirectoryW(system.as_mut_ptr(), system.len() as u32) };
    if length == 0 || length as usize >= system.len() { return Ok(91); }
    let system = std::path::PathBuf::from(String::from_utf16(&system[..length as usize])?);
    let child = std::process::Command::new(system.join("cmd.exe")).args(["/d", "/c", "exit", "0"]).spawn();
    let spawned = child.is_ok(); if let Ok(mut child) = child { let _ = child.wait(); }
    let Operation::SvgTile { tile } = job.request.operation else { return Ok(91); };
    if tile.width != 8 || tile.height != 1 { return Ok(91); }
    let mut data = [0_u8; 32]; for pixel in data.chunks_exact_mut(4) { pixel[3] = 255; }
    for (index, allowed) in [read, write, network, spawned, true].into_iter().enumerate() { data[index * 4] = u8::from(allowed) * 255; }
    let mut stdout = std::io::stdout().lock(); let mut reply = ReplyWriter::new(&mut stdout, job.binding, job.request.limits.output_bytes)?;
    reply.metadata(&ResultMetadata::RgbaTile { format: DocumentFormat::Svg, page: None, origin: [tile.x, tile.y], requested_scale: tile.scale,
        native_scale: f64::from(tile.scale as f32), width: 8, height: 1, opaque_paper: false })?;
    reply.data(&data)?; reply.finish()?;
    Ok(0)
}
