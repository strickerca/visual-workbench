use std::io::Read;
fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("cargo:rerun-if-env-changed=VW_MCP_APP_IMAGE_MANIFEST");
    let bytes = match std::env::var_os("VW_MCP_APP_IMAGE_MANIFEST") {
        Some(path) => {
            let path = std::path::PathBuf::from(path);
            println!("cargo:rerun-if-changed={}", path.display());
            let file = std::fs::File::open(path)?;
            let length = file.metadata()?.len();
            if length == 0 || length > 4 * 1024 * 1024 {
                return Err("app inventory bound".into());
            }
            let mut bytes = Vec::with_capacity(length as usize);
            file.take(length + 1).read_to_end(&mut bytes)?;
            if bytes.len() as u64 != length {
                return Err("app inventory changed".into());
            }
            bytes
        }
        None => Vec::new(), // Host/verifier build only; bridge startup refuses.
    };
    let out = std::path::PathBuf::from(std::env::var_os("OUT_DIR").ok_or("cargo OUT_DIR")?);
    std::fs::write(out.join("app-image.inventory"), bytes)?;
    Ok(())
}
