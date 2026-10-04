use std::io::{self, Read, Write};
fn run() -> vw_mcp_native::Result<()> {
    let mut bytes = Vec::new();
    io::stdin()
        .take(8193)
        .read_to_end(&mut bytes)
        .map_err(|_| "read")?;
    if bytes.len() > 8192 {
        return Err("limit");
    }
    #[derive(serde::Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Input {
        directory: String,
    }
    let input: Input = serde_json::from_slice(&bytes).map_err(|_| "json")?;
    let result = vw_mcp_native::package_reader::verify(std::path::Path::new(&input.directory))?;
    let bytes = serde_json::to_vec(&result).map_err(|_| "json")?;
    if bytes.len() > 16 * 1024 * 1024 {
        return Err("limit");
    }
    io::stdout().write_all(&bytes).map_err(|_| "write")
}
fn main() {
    if run().is_err() {
        std::process::exit(1);
    }
}
