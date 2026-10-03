#![allow(clippy::unwrap_used, clippy::expect_used)]
#[path = "../../tests/support/mod.rs"]
mod support;
use vw_core::*;

#[tokio::main(flavor = "current_thread")]
async fn main() -> std::result::Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let out = std::path::PathBuf::from(args.next().ok_or("output directory required")?);
    let project_path =
        std::path::PathBuf::from(args.next().ok_or("new temporary project path required")?);
    if args.next().is_some() || !out.is_dir() || !project_path.is_absolute() {
        return Err("invalid fixture arguments".into());
    }
    let project = create_image_project(support::create(&project_path), support::cancel()).await?;
    let result = support::draw(&project).await;
    project.close().await?;
    let hex = |bytes: &[u8]| bytes.iter().map(|b| format!("{b:02x}")).collect::<String>();
    let value = serde_json::json!({"schema":1,"source_png_hex":hex(&support::source()),"export_blake3":result.blake3,"state_hash":result.revision.state_hash,"device_id":support::device(),"ids":(1..=6).map(support::id).collect::<Vec<_>>(),"time_ms":support::TIME});
    std::fs::write(
        out.join("ffi-golden.json"),
        serde_json::to_vec_pretty(&value)?,
    )?;
    let mut properties = format!(
        "schema=1\nsource_png_hex={}\nexport_png_hex={}\nexport_blake3={}\nstate_hash={}\ndevice_id={}\ntime_ms={}\n",
        hex(&support::source()),
        hex(&result.bytes),
        result.blake3,
        result.revision.state_hash,
        support::device(),
        support::TIME
    );
    for n in 1..=6 {
        properties.push_str(&format!("id{n}={}\n", support::id(n)));
    }
    std::fs::write(out.join("ffi-golden.properties"), properties)?;
    println!("FFI fixture written from Rust core; synthetic source only");
    Ok(())
}
