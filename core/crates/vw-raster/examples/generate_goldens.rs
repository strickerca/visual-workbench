//! Run centrally after review; stdout is a text-only BLAKE3 manifest, not evidence
//! that any second platform matched it. Never auto-update goldens in a test.
#[path = "../tests/support/golden.rs"]
mod golden;
#[path = "../tests/support/mod.rs"]
mod support;
use support::*;
fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("{}", serde_json::to_string_pretty(&golden::hashes()?)?);
    Ok(())
}
